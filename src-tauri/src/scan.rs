//! Scan orchestration: picks the helper (fast MFT path) or the directory walker,
//! streams progress to the UI and stores the finished tree.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bulk_core::helper::{self, Client, Event, ProgressInfo};
use bulk_core::scan::{Progress, walk};
use bulk_core::snapshot;
use bulk_core::tree::NodeId;
use bulk_core::usn::Journal;
use bulk_core::volume;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, Drive};

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum HelperStatus {
    Checking,
    Ready,
    /// The task isn't registered (the user declined the prompt); allowing it
    /// needs one UAC prompt.
    NotInstalled,
    /// The task points at a different copy of the helper (e.g. after an update).
    Outdated,
    /// Fast mode is off in settings: scans walk folders, the helper isn't used.
    Off,
}

pub struct HelperState {
    pub status: Mutex<HelperStatus>,
}

impl Default for HelperState {
    fn default() -> Self {
        HelperState {
            status: Mutex::new(HelperStatus::Checking),
        }
    }
}

pub fn init_drives(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut drives = state.drives.lock().unwrap();
    for v in volume::list() {
        drives.insert(v.letter, Drive::new(v));
    }
}

/// Refresh volume info (free space etc.) and pick up newly attached drives.
pub fn refresh_drives(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut drives = state.drives.lock().unwrap();
    let vols = volume::list();
    drives
        .retain(|l, d| d.tree.is_some() || d.scan.is_some() || vols.iter().any(|v| v.letter == *l));
    for v in vols {
        match drives.get_mut(&v.letter) {
            Some(d) => d.vol = v,
            None => {
                drives.insert(v.letter, Drive::new(v));
            }
        }
    }
}

/// Connect to the helper when fast mode is on. If its task isn't set up yet (first
/// run, or after an update), ask for elevation right away, unless the user has
/// already said no.
pub fn check_helper(app: AppHandle) {
    std::thread::spawn(move || {
        let settings = crate::settings::get(&app);
        if !settings.fast_mode {
            set_helper_status(&app, HelperStatus::Off);
            return;
        }
        set_helper_status(&app, HelperStatus::Checking);
        let mut status = probe_helper();
        // A helper that's busy finishing old connections (e.g. right after the app
        // restarted) can miss the first check; give it one more chance.
        if status != HelperStatus::Ready && helper::task_installed() {
            std::thread::sleep(Duration::from_secs(3));
            status = probe_helper();
        }
        if status != HelperStatus::Ready && !settings.helper_declined {
            status = install_helper(&app);
        }
        eprintln!("helper status: {status:?}");
        set_helper_status(&app, status);
    });
}

/// Register the helper's task (one UAC prompt) and start it. A declined prompt
/// is remembered so startup doesn't ask every time.
pub fn install_helper(app: &AppHandle) -> HelperStatus {
    match helper::install_elevated(&helper::helper_exe()) {
        Ok(true) => {
            crate::settings::update(app, |s| s.helper_declined = false);
            let ok = Client::new().is_ok_and(|c| c.ensure_running(Duration::from_secs(6)));
            if ok {
                HelperStatus::Ready
            } else {
                HelperStatus::Outdated
            }
        }
        _ => {
            crate::settings::update(app, |s| s.helper_declined = true);
            HelperStatus::NotInstalled
        }
    }
}

/// Stop using the helper: live updates end and scans walk folders. The task stays
/// installed so turning fast mode back on needs no prompt.
pub fn helper_off(app: &AppHandle) {
    set_helper_status(app, HelperStatus::Off);
    let state = app.state::<AppState>();
    for d in state.drives.lock().unwrap().values_mut() {
        d.stop_watch();
    }
}

/// Record the helper status; once it's ready, restored drives can go live.
pub fn set_helper_status(app: &AppHandle, status: HelperStatus) {
    *app.state::<HelperState>().status.lock().unwrap() = status;
    let _ = app.emit("helper-status", status);
    if status == HelperStatus::Ready {
        let app = app.clone();
        std::thread::spawn(move || {
            // Restoring may still be in flight; wait for it so its journals are known.
            let state = app.state::<AppState>();
            while !state.restored.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(30));
            }
            let letters: Vec<char> = state.drives.lock().unwrap().keys().copied().collect();
            for l in letters {
                crate::live::start(&app, l);
            }
        });
    }
}

fn probe_helper() -> HelperStatus {
    let Ok(client) = Client::new() else {
        return HelperStatus::NotInstalled;
    };
    if !client.ensure_running(Duration::from_secs(4)) {
        return if helper::task_installed() {
            HelperStatus::Outdated
        } else {
            HelperStatus::NotInstalled
        };
    }
    match client.hello() {
        Ok(Event::Hello { protocol, exe, .. })
            if protocol == helper::PROTOCOL
                && std::path::Path::new(&exe) == helper::helper_exe().as_path() =>
        {
            HelperStatus::Ready
        }
        other => {
            eprintln!(
                "helper mismatch: {other:?} (expected {:?})",
                helper::helper_exe()
            );
            HelperStatus::Outdated
        }
    }
}

pub fn helper_ready(app: &AppHandle) -> bool {
    *app.state::<HelperState>().status.lock().unwrap() == HelperStatus::Ready
}

#[derive(Serialize, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ScanEvent {
    Started {
        method: &'static str,
    },
    Progress(ProgressInfo),
    Done {
        elapsed_ms: u64,
        files: u32,
        alloc: u64,
    },
    Error {
        message: String,
    },
    Cancelled,
}

pub fn start(app: AppHandle, letter: char, channel: Channel<ScanEvent>) -> Result<(), String> {
    let (vol, progress) = {
        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let d = drives.get_mut(&letter).ok_or("unknown drive")?;
        if d.scan.is_some() {
            return Err("already scanning".into());
        }
        d.stop_watch();
        d.needs_rescan = false;
        let p = Arc::new(Progress::default());
        d.scan = Some(p.clone());
        (d.vol.clone(), p)
    };

    std::thread::spawn(move || {
        let t0 = Instant::now();
        let use_helper = vol.is_ntfs() && helper_ready(&app);
        let method = if use_helper { "mft" } else { "walk" };
        let _ = channel.send(ScanEvent::Started { method });

        let result = if use_helper {
            scan_via_helper(letter, &progress, &channel)
        } else {
            scan_via_walk(&vol.root, vol.cluster as u64, &progress, &channel)
        };

        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let Some(d) = drives.get_mut(&letter) else {
            return;
        };
        d.scan = None;
        match result {
            Ok((tree, journal)) => {
                let root = *tree.get(tree.root);
                d.view.reset(&tree);
                d.tree = Some(tree);
                d.method = Some(method);
                d.scanned_at = Some(now());
                d.journal = journal;
                d.index = None;
                d.dirty = false;
                let saved = d.meta().zip(d.tree.as_ref().map(|t| t.compact()));
                drop(drives);
                let _ = channel.send(ScanEvent::Done {
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    files: root.files,
                    alloc: root.alloc,
                });
                // Keep the result for instant reopening and for "what changed".
                if let Some((meta, tree)) = saved {
                    let _ = snapshot::save_current(&meta, &tree);
                    let _ =
                        snapshot::save_history(&meta, &tree, crate::settings::history_keep(&app));
                }
                crate::live::start(&app, letter);
            }
            Err(_) if progress.cancelled() => {
                let _ = channel.send(ScanEvent::Cancelled);
            }
            Err(e) => {
                let _ = channel.send(ScanEvent::Error {
                    message: format!("{e:#}"),
                });
            }
        }
    });
    Ok(())
}

/// Folders with more files than this are rescanned from the file table when the
/// helper is available.
const HELPER_FOLDER_FILES: u32 = 50_000;

/// Rescan one folder with the folder walker (or the helper, for huge folders) and merge it into the drive's tree.
/// Live updates keep running; the drive is saved again on exit.
pub fn start_folder(
    app: AppHandle,
    letter: char,
    folder: NodeId,
    channel: Channel<ScanEvent>,
) -> Result<(), String> {
    let (path, cluster, progress, via_helper) = {
        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let d = drives.get_mut(&letter).ok_or("unknown drive")?;
        if d.scan.is_some() {
            return Err("already scanning".into());
        }
        let tree = d.tree.as_ref().ok_or("not scanned")?;
        if folder as usize >= tree.len() || !tree.get(folder).is_dir() {
            return Err("not a folder".into());
        }
        // Walking a huge folder takes longer than reading the whole file table.
        let via_helper =
            d.vol.is_ntfs() && tree.get(folder).files > HELPER_FOLDER_FILES && helper_ready(&app);
        let p = Arc::new(Progress::default());
        d.scan = Some(p.clone());
        (tree.path(folder), d.vol.cluster as u64, p, via_helper)
    };

    std::thread::spawn(move || {
        let t0 = Instant::now();
        let method = if via_helper { "mft" } else { "walk" };
        let _ = channel.send(ScanEvent::Started { method });
        // The fresh tree, and the folder within it to merge from.
        let result = if via_helper {
            scan_via_helper(letter, &progress, &channel).and_then(|(t, _)| {
                let from = t
                    .find_path(&path)
                    .ok_or_else(|| anyhow::anyhow!("{path} no longer exists"))?;
                Ok((t, from))
            })
        } else {
            scan_via_walk(&path, cluster, &progress, &channel).map(|(t, _)| {
                let root = t.root;
                (t, root)
            })
        };

        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let Some(d) = drives.get_mut(&letter) else {
            return;
        };
        d.scan = None;
        // Find the folder again by path: it may have moved while we were walking.
        let target = d.tree.as_ref().and_then(|t| t.find_path(&path));
        match (result, target) {
            (Ok((fresh, from)), Some(target)) => {
                let tree = d.tree.as_mut().unwrap();
                tree.merge_folder(target, &fresh, from);
                let n = *tree.get(target);
                d.view.rebuild(d.tree.as_ref().unwrap());
                d.dirty = true;
                drop(drives);
                let _ = channel.send(ScanEvent::Done {
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    files: n.files,
                    alloc: n.alloc,
                });
                let _ = app.emit("drive-updated", letter);
            }
            (Err(_), _) if progress.cancelled() => {
                let _ = channel.send(ScanEvent::Cancelled);
            }
            (Err(e), _) => {
                let _ = channel.send(ScanEvent::Error {
                    message: format!("{e:#}"),
                });
            }
            (Ok(_), None) => {
                let _ = channel.send(ScanEvent::Error {
                    message: format!("{path} no longer exists"),
                });
            }
        }
    });
    Ok(())
}

fn scan_via_helper(
    letter: char,
    progress: &Progress,
    channel: &Channel<ScanEvent>,
) -> anyhow::Result<(bulk_core::Tree, Option<Journal>)> {
    Client::new()?.scan(letter, |p| {
        let _ = channel.send(ScanEvent::Progress(p));
        !progress.cancelled()
    })
}

fn scan_via_walk(
    root: &str,
    cluster: u64,
    progress: &Arc<Progress>,
    channel: &Channel<ScanEvent>,
) -> anyhow::Result<(bulk_core::Tree, Option<Journal>)> {
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ticker = {
        let (p, done, ch) = (progress.clone(), done.clone(), channel.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let _ = ch.send(ScanEvent::Progress(ProgressInfo::of(&p)));
                std::thread::sleep(Duration::from_millis(80));
            }
        })
    };
    let r = walk::walk(root, cluster, progress);
    done.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    Ok((r?, None))
}

pub use bulk_core::os::now;
