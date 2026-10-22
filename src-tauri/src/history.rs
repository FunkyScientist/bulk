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
    /// The compared folder in the snapshot; `None` if it didn't exist back then.
    old_root: Option<NodeId>,
    new_root: NodeId,
    /// Full path of the compared folder.
    scope: String,
}

/// Totals for `folder` (the whole drive when `None`) now and in the snapshot. The
/// snapshot's ids differ from the live tree's, so the folder is matched by path.
fn summarize(
    taken: u64,
    old: &bulk_core::Tree,
    new: &bulk_core::Tree,
    folder: Option<NodeId>,
) -> DiffSummary {
    let new_root = folder
        .filter(|&f| (f as usize) < new.len() && new.get(f).is_dir())
        .unwrap_or(new.root);
    let scope = new.path(new_root);
    let old_root = if new_root == new.root {
        Some(old.root)
    } else {
        old.find_path(&scope)
    };
    let n = *new.get(new_root);
    let (old_alloc, old_files) = old_root.map_or((0, 0), |o| (old.get(o).alloc, old.get(o).files));
    DiffSummary {
        taken,
        old_alloc,
        new_alloc: n.alloc,
        old_files,
        new_files: n.files,
        old_root,
        new_root,
        scope,
    }
}

/// Load a snapshot to compare against. Only files inside Bulk's snapshot folder
/// are accepted.
#[tauri::command]
pub async fn diff_open(
    state: State<'_, AppState>,
    letter: char,
    path: PathBuf,
    folder: Option<NodeId>,
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
    let summary = summarize(meta.taken, &old, new, folder);
    d.diff = Some((meta, old));
    Ok(summary)
}

/// Re-scope the loaded comparison to `folder` (or the whole drive).
#[tauri::command]
pub fn diff_scope(
    state: State<AppState>,
    letter: char,
    folder: Option<NodeId>,
) -> Res<DiffSummary> {
    let drives = state.drives.lock().unwrap();
    let d = drives.get(&letter).ok_or("unknown drive")?;
    let new = d.tree.as_ref().ok_or("not scanned")?;
    let (meta, old) = d.diff.as_ref().ok_or("no snapshot loaded")?;
    Ok(summarize(meta.taken, old, new, folder))
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
