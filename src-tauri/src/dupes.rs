//! Duplicate finder commands. Candidates are collected under the drives lock, then
//! hashing runs without holding it.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bulk_core::dupes::{self, DupeProgress, Group};
use bulk_core::tree::NodeId;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;

#[derive(Default)]
pub struct DupesState(Mutex<Option<Arc<DupeProgress>>>);

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DupesEvent {
    #[serde(rename_all = "camelCase")]
    Progress {
        phase: u64,
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        groups: Vec<Group>,
        elapsed_ms: u64,
        candidates: usize,
    },
    Error {
        message: String,
    },
    Cancelled,
}

#[tauri::command]
pub fn dupes_start(
    app: AppHandle,
    state: State<AppState>,
    letter: char,
    root: Option<NodeId>,
    min_size: u64,
    skip_system: bool,
    on_event: Channel<DupesEvent>,
) -> Result<(), String> {
    let cands = {
        let drives = state.drives.lock().unwrap();
        let tree = drives
            .get(&letter)
            .and_then(|d| d.tree.as_ref())
            .ok_or("not scanned")?;
        let root = root
            .filter(|&r| (r as usize) < tree.len())
            .unwrap_or(tree.root);
        dupes::candidates(tree, root, min_size, skip_system)
    };
    let progress = Arc::new(DupeProgress::default());
    if let Some(old) = app
        .state::<DupesState>()
        .0
        .lock()
        .unwrap()
        .replace(progress.clone())
    {
        old.cancel();
    }

    std::thread::spawn(move || {
        let t0 = Instant::now();
        let n = cands.len();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ticker = {
            let (p, done, ch) = (progress.clone(), done.clone(), on_event.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let _ = ch.send(DupesEvent::Progress {
                        phase: p.phase.load(Ordering::Relaxed),
                        files_done: p.files_done.load(Ordering::Relaxed),
                        files_total: p.files_total.load(Ordering::Relaxed),
                        bytes_done: p.bytes_done.load(Ordering::Relaxed),
                        bytes_total: p.bytes_total.load(Ordering::Relaxed),
                    });
                    std::thread::sleep(Duration::from_millis(120));
                }
            })
        };
        let mut groups = dupes::find(cands, &progress);
        done.store(true, Ordering::Relaxed);
        let _ = ticker.join();
        if progress.cancelled() {
            let _ = on_event.send(DupesEvent::Cancelled);
            return;
        }
        groups.truncate(1000);
        let _ = on_event.send(DupesEvent::Done {
            groups,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            candidates: n,
        });
    });
    Ok(())
}

#[tauri::command]
pub fn dupes_cancel(state: State<DupesState>) {
    if let Some(p) = state.0.lock().unwrap().take() {
        p.cancel();
    }
}
