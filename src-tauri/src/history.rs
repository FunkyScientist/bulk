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
pub(crate) fn summarize(
    taken: u64,
    old: &bulk_core::Tree,
    new: &bulk_core::Tree,
    folder: Option<NodeId>,
    f: diff::Filters,
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
    let (new_alloc, new_files) = diff::totals(new, f.new, new_root);
    let (old_alloc, old_files) = old_root.map_or((0, 0), |o| diff::totals(old, f.old, o));
    DiffSummary {
        taken,
        old_alloc,
        new_alloc,
        old_files,
        new_files,
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
    new_tree(d)?;
    if meta.serial != d.vol.serial {
        return Err("snapshot is from a different drive".into());
    }
    d.diff = Some((meta, old));
    d.diff_filter = None;
    d.sync_diff_filter();
    let (meta, old) = d.diff.as_ref().unwrap();
    Ok(summarize(
        meta.taken,
        old,
        new_tree(d)?,
        folder,
        d.diff_filters(),
    ))
}

/// Re-scope the loaded comparison to `folder` (or the whole drive).
#[tauri::command]
pub fn diff_scope(
    state: State<AppState>,
    letter: char,
    folder: Option<NodeId>,
) -> Res<DiffSummary> {
    let mut drives = state.drives.lock().unwrap();
    let d = drives.get_mut(&letter).ok_or("unknown drive")?;
    d.sync_diff_filter();
    let new = d.tree.as_ref().ok_or("not scanned")?;
    let (meta, old) = d.diff.as_ref().ok_or("no snapshot loaded")?;
    Ok(summarize(meta.taken, old, new, folder, d.diff_filters()))
}

/// Write the loaded comparison, scoped to `folder`, to `path` (chosen by the user).
#[tauri::command]
pub async fn export_changes(
    state: State<'_, AppState>,
    letter: char,
    folder: Option<NodeId>,
    format: crate::commands::ExportFormat,
    path: PathBuf,
    theme: Option<bulk_core::export::Theme>,
) -> Res<()> {
    let theme = theme.unwrap_or_default();
    use crate::commands::ExportFormat;
    use bulk_core::export::{self, ChangeSummary};
    // Collect under the lock (it's quick); write the file without it.
    let (summary, rows, only) = {
        let mut drives = state.drives.lock().unwrap();
        let d = drives.get_mut(&letter).ok_or("unknown drive")?;
        d.sync_diff_filter();
        let new = d.tree.as_ref().ok_or("not scanned")?;
        let (meta, old) = d.diff.as_ref().ok_or("no snapshot loaded")?;
        let f = d.diff_filters();
        let s = summarize(meta.taken, old, new, folder, f);
        let rows = export::collect_changes(old, new, s.old_root, s.new_root, 500_000, f);
        let only = d.view.filter.as_ref().map(|f| f.query.clone());
        (s, rows, only)
    };
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let file = std::fs::File::create(&path)?;
        let scope = match &only {
            Some(q) => format!("{} · only {q}", summary.scope),
            None => summary.scope.clone(),
        };
        let facts = ChangeSummary {
            scope: &scope,
            since: summary.taken as u32,
            generated: crate::scan::now() as u32,
            old_alloc: summary.old_alloc,
            new_alloc: summary.new_alloc,
            old_files: summary.old_files,
            new_files: summary.new_files,
        };
        match format {
            ExportFormat::Csv => export::write_changes_csv(&rows, file)?,
            ExportFormat::Json => export::write_changes_json(&facts, &rows, file)?,
            ExportFormat::Html => export::write_changes_html(&facts, &rows, &theme, file)?,
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// Write duplicate sets found by the duplicate finder to `path`.
#[tauri::command]
pub async fn export_dupes(
    scope: String,
    sets: Vec<bulk_core::export::DupeSet>,
    format: crate::commands::ExportFormat,
    path: PathBuf,
    theme: Option<bulk_core::export::Theme>,
) -> Res<()> {
    let theme = theme.unwrap_or_default();
    use crate::commands::ExportFormat;
    use bulk_core::export;
    let sets: Vec<_> = sets.into_iter().filter(|s| s.files.len() > 1).collect();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let file = std::fs::File::create(&path)?;
        match format {
            ExportFormat::Csv => export::write_dupes_csv(&sets, file)?,
            ExportFormat::Json => export::write_dupes_json(&scope, &sets, file)?,
            ExportFormat::Html => export::write_dupes_html(&scope, &sets, &theme, file)?,
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn diff_children(
    state: State<AppState>,
    letter: char,
    old_id: Option<NodeId>,
    new_id: Option<NodeId>,
) -> Res<Vec<Item>> {
    let mut drives = state.drives.lock().unwrap();
    let d = drives.get_mut(&letter).ok_or("unknown drive")?;
    d.sync_diff_filter();
    let new = d.tree.as_ref().ok_or("not scanned")?;
    let (_, old) = d.diff.as_ref().ok_or("no snapshot loaded")?;
    let old_id = old_id.filter(|&i| (i as usize) < old.len());
    let new_id = new_id.filter(|&i| (i as usize) < new.len());
    Ok(diff::children(
        old,
        new,
        old_id,
        new_id,
        400,
        d.diff_filters(),
    ))
}

#[tauri::command]
pub fn diff_close(state: State<AppState>, letter: char) {
    if let Some(d) = state.drives.lock().unwrap().get_mut(&letter) {
        d.diff = None;
        d.diff_filter = None;
    }
}

fn new_tree(d: &crate::state::Drive) -> Res<&bulk_core::Tree> {
    d.tree.as_ref().ok_or_else(|| "not scanned".into())
}
