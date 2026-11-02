//! Keeping trees current without rescanning: restore the last snapshot at startup,
//! catch up from the change journal, then watch it while the app runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bulk_core::helper::{CatchUp, Client};
use bulk_core::snapshot;
use bulk_core::usn::{self, Change, FrnIndex};
use tauri::{AppHandle, Emitter, Manager};

use crate::scan::{helper_ready, now};
use crate::state::AppState;

/// How often a live tree is written back to `current.bulk`.
const SAVE_EVERY: Duration = Duration::from_secs(10 * 60);

/// Load each drive's `current.bulk` in parallel. Runs at startup.
pub fn restore(app: AppHandle) {
    std::thread::spawn(move || {
        let targets: Vec<(char, u32)> = {
            let state = app.state::<AppState>();
            let drives = state.drives.lock().unwrap();
            drives
                .values()
                .map(|d| (d.vol.letter, d.vol.serial))
                .collect()
        };
        std::thread::scope(|s| {
            for (letter, serial) in targets {
                let app = &app;
                s.spawn(move || {
                    let path = snapshot::current_path(serial);
                    if !path.exists() {
                        return;
                    }
                    let Ok((meta, tree)) = snapshot::load(&path) else {
                        return;
                    };
                    let state = app.state::<AppState>();
                    let mut drives = state.drives.lock().unwrap();
                    let Some(d) = drives.get_mut(&letter) else {
                        return;
                    };
                    if d.tree.is_some() || d.scan.is_some() {
                        return; // a fresh scan won the race
                    }
                    d.view.reset(&tree);
                    d.tree = Some(tree);
                    d.scanned_at = Some(meta.taken);
                    d.method = Some(if meta.method == "mft" { "mft" } else { "walk" });
                    d.journal = meta.journal;
                    d.index = None;
                    drop(drives);
                    let _ = app.emit("drive-updated", letter);
                });
            }
        });
        app.state::<AppState>()
            .restored
            .store(true, Ordering::Relaxed);
        let _ = app.emit("restored", ());
    });
}

/// Bring a restored drive up to date and keep it live. Asks the UI to rescan if the
/// journal can't cover the gap.
pub fn start(app: &AppHandle, letter: char) {
    let (journal, stop) = {
        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let Some(d) = drives.get_mut(&letter) else {
            return;
        };
        if d.tree.is_none() || d.scan.is_some() || d.watch.is_some() || !d.vol.is_ntfs() {
            return;
        }
        let Some(j) = d.journal else { return };
        let stop = Arc::new(AtomicBool::new(false));
        d.watch = Some(stop.clone());
        (j, stop)
    };
    if !helper_ready(app) {
        clear(app, letter, &stop);
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let client = match Client::new() {
            Ok(c) => c,
            Err(_) => return clear(&app, letter, &stop),
        };

        // Catch up in one go first so the UI gets a single big refresh.
        match client.changes(letter, journal.id, journal.next) {
            Ok(CatchUp::Changes(changes, next)) => apply(&app, letter, &changes, next),
            Ok(CatchUp::Gap) => return gap(&app, letter, &stop),
            Err(_) => return clear(&app, letter, &stop),
        }
        let _ = app.emit("drive-live", letter);

        let mut last_save = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            let from = {
                let state = app.state::<AppState>();
                let drives = state.drives.lock().unwrap();
                match drives.get(&letter).and_then(|d| d.journal) {
                    Some(j) => j.next,
                    None => break,
                }
            };
            let r = client.watch(letter, journal.id, from, |changes, next| {
                if !changes.is_empty() {
                    apply(&app, letter, &changes, next);
                } else {
                    set_next(&app, letter, next);
                }
                if last_save.elapsed() > SAVE_EVERY {
                    save_current(&app, letter);
                    last_save = Instant::now();
                }
                !stop.load(Ordering::Relaxed)
            });
            match r {
                Ok(CatchUp::Gap) => return gap(&app, letter, &stop),
                Ok(_) => {}
                // Helper restarted or went idle: give it a moment and reconnect.
                Err(_) => {
                    std::thread::sleep(Duration::from_secs(3));
                    if !client.ensure_running(Duration::from_secs(5)) {
                        break;
                    }
                }
            }
        }
        clear(&app, letter, &stop);
    });
}

fn clear(app: &AppHandle, letter: char, stop: &Arc<AtomicBool>) {
    let state = app.state::<AppState>();
    let mut drives = state.drives.lock().unwrap();
    if let Some(d) = drives.get_mut(&letter)
        && d.watch.as_ref().is_some_and(|w| Arc::ptr_eq(w, stop))
    {
        d.watch = None;
    }
    drop(drives);
    let _ = app.emit("drive-updated", letter);
}

/// The journal can't cover the gap. Flag the drive rather than only sending an event:
/// at startup this happens before the window is listening.
fn gap(app: &AppHandle, letter: char, stop: &Arc<AtomicBool>) {
    if let Some(d) = app
        .state::<AppState>()
        .drives
        .lock()
        .unwrap()
        .get_mut(&letter)
    {
        d.needs_rescan = true;
    }
    clear(app, letter, stop);
}

fn set_next(app: &AppHandle, letter: char, next: i64) {
    let state = app.state::<AppState>();
    let mut drives = state.drives.lock().unwrap();
    if let Some(j) = drives.get_mut(&letter).and_then(|d| d.journal.as_mut()) {
        j.next = next;
    }
}

fn apply(app: &AppHandle, letter: char, changes: &[Change], next: i64) {
    let touched = {
        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let Some(d) = drives.get_mut(&letter) else {
            return;
        };
        let Some(tree) = d.tree.as_mut() else { return };
        let index = d.index.get_or_insert_with(|| FrnIndex::build(tree));
        let n = usn::apply(tree, index, changes);
        if let Some(j) = d.journal.as_mut() {
            j.next = next;
        }
        d.scanned_at = Some(now());
        if n > 0 {
            d.dirty = true;
            d.view.rebuild(tree);
        }
        n
    };
    if touched > 0 {
        let _ = app.emit("drive-updated", letter);
    }
}

/// Write the drive's tree to `current.bulk` if it changed. Compacts a copy first,
/// so removed nodes don't pile up on disk; the live tree keeps its ids.
pub fn save_current(app: &AppHandle, letter: char) {
    let (meta, tree) = {
        let state = app.state::<AppState>();
        let mut drives = state.drives.lock().unwrap();
        let Some(d) = drives.get_mut(&letter) else {
            return;
        };
        if !d.dirty {
            return;
        }
        let (Some(meta), Some(tree)) = (d.meta(), d.tree.as_ref()) else {
            return;
        };
        let copy = tree.compact();
        d.dirty = false;
        (meta, copy)
    };
    if snapshot::save_current(&meta, &tree).is_err() {
        let state = app.state::<AppState>();
        if let Some(d) = state.drives.lock().unwrap().get_mut(&letter) {
            d.dirty = true;
        }
    }
}

/// Save every changed drive. Called when the app exits.
pub fn save_all(app: &AppHandle) {
    let letters: Vec<char> = {
        let state = app.state::<AppState>();
        let drives = state.drives.lock().unwrap();
        drives
            .values()
            .filter(|d| d.dirty)
            .map(|d| d.vol.letter)
            .collect()
    };
    std::thread::scope(|s| {
        for l in letters {
            s.spawn(move || save_current(app, l));
        }
    });
}
