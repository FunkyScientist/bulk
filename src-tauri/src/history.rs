//! Commands for the "what changed" view: list saved snapshots and compare one
//! against the current tree.

use std::path::PathBuf;

use bulk_core::diff::{self, Item};
use bulk_core::snapshot;
use bulk_core::tree::NodeId;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

type Res<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    path: PathBuf,
    taken: u64,
    alloc: u64,
    files: u32,
    method: String,
}

#[tauri::command]
pub async fn history(state: State<'_, AppState>, letter: char) -> Res<Vec<HistoryEntry>> {
    let serial = {
        let drives = state.drives.lock().unwrap();
        drives.get(&letter).ok_or("unknown drive")?.vol.serial
    };
    let entries = tauri::async_runtime::spawn_blocking(move || snapshot::history(serial))
        .await
        .map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .map(|e| HistoryEntry {
            path: e.path,
            taken: e.meta.taken,
            alloc: e.meta.alloc,
            files: e.meta.files,
            method: e.meta.method,
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    taken: u64,
    old_alloc: u64,
    new_alloc: u64,
    old_files: u32,
    new_files: u32,
    old_root: NodeId,
    new_root: NodeId,
}

/// Load a snapshot to compare against. Only files inside Bulk's snapshot folder
/// are accepted.
#[tauri::command]
pub async fn diff_open(
    state: State<'_, AppState>,
    letter: char,
    path: PathBuf,
) -> Res<DiffSummary> {
    if !snapshot::is_snapshot_path(&path) {
        return Err("not a Bulk snapshot".into());
    }
    let (meta, old) = tauri::async_runtime::spawn_blocking(move || snapshot::load(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut drives = state.drives.lock().unwrap();
    let d = drives.get_mut(&letter).ok_or("unknown drive")?;
    let new = d.tree.as_ref().ok_or("not scanned")?;
    if meta.serial != d.vol.serial {
        return Err("snapshot is from a different drive".into());
    }
    let (o, n) = (*old.get(old.root), *new.get(new.root));
    let summary = DiffSummary {
        taken: meta.taken,
        old_alloc: o.alloc,
        new_alloc: n.alloc,
        old_files: o.files,
        new_files: n.files,
        old_root: old.root,
        new_root: new.root,
    };
    d.diff = Some((meta, old));
    Ok(summary)
}

#[tauri::command]
pub fn diff_children(
    state: State<AppState>,
    letter: char,
    old_id: Option<NodeId>,
    new_id: Option<NodeId>,
) -> Res<Vec<Item>> {
    let drives = state.drives.lock().unwrap();
    let d = drives.get(&letter).ok_or("unknown drive")?;
    let new = d.tree.as_ref().ok_or("not scanned")?;
    let (_, old) = d.diff.as_ref().ok_or("no snapshot loaded")?;
    let old_id = old_id.filter(|&i| (i as usize) < old.len());
    let new_id = new_id.filter(|&i| (i as usize) < new.len());
    Ok(diff::children(old, new, old_id, new_id, 400))
}

#[tauri::command]
pub fn diff_close(state: State<AppState>, letter: char) {
    if let Some(d) = state.drives.lock().unwrap().get_mut(&letter) {
        d.diff = None;
    }
}
