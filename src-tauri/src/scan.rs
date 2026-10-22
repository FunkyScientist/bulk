//! Scan orchestration: picks the helper (fast MFT path) or the directory walker,
//! streams progress to the UI and stores the finished tree.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bulk_core::helper::{self, Client, Event, ProgressInfo};
use bulk_core::scan::{Progress, walk};
use bulk_core::snapshot;
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
    /// The task isn't registered yet; enabling needs one UAC prompt.
    NotInstalled,
    /// The task points at a different copy of the helper (e.g. after an update).
    Outdated,
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

pub fn check_helper(app: AppHandle) {
    std::thread::spawn(move || {
        let mut status = probe_helper();
        // A helper that's busy finishing old connections (e.g. right after the app
        // restarted) can miss the first check; give it one more chance.
        if status != HelperStatus::Ready && helper::task_installed() {
            std::thread::sleep(Duration::from_secs(3));
            status = probe_helper();
        }
        eprintln!("helper status: {status:?}");
        set_helper_status(&app, status);
    });
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
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
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

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
