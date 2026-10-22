use std::os::windows::process::CommandExt;

use bulk_core::helper;
use bulk_core::tree::{NodeId, SortKey};
use bulk_core::volume::{DriveKind, Volume};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::scan::{self, HelperState, HelperStatus, ScanEvent};
use crate::state::{AppState, Row, row};

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveSummary {
    #[serde(flatten)]
    vol: Volume,
    auto_scan: bool,
    scanning: bool,
    scanned: bool,
    scanned_at: Option<u64>,
    method: Option<&'static str>,
    files: u32,
    alloc: u64,
}

#[tauri::command]
pub fn drives(app: AppHandle, state: State<AppState>) -> Vec<DriveSummary> {
    scan::refresh_drives(&app);
    let drives = state.drives.lock().unwrap();
    let mut out: Vec<DriveSummary> = drives
        .values()
        .map(|d| {
            let root = d.tree.as_ref().map(|t| *t.get(t.root));
            DriveSummary {
                vol: d.vol.clone(),
                auto_scan: d.vol.auto_scan(),
                scanning: d.scan.is_some(),
                scanned: d.tree.is_some(),
                scanned_at: d.scanned_at,
                method: d.method,
                files: root.map_or(0, |r| r.files),
                alloc: root.map_or(0, |r| r.alloc),
            }
        })
        .collect();
    out.sort_by_key(|d| (d.vol.kind != DriveKind::Fixed, d.vol.letter));
    out
}

#[tauri::command]
pub fn helper_status(state: State<HelperState>) -> HelperStatus {
    *state.status.lock().unwrap()
}

/// Register the helper task (one UAC prompt). Resolves once the helper answers.
#[tauri::command]
pub async fn enable_fast_scan(app: AppHandle) -> Res<HelperStatus> {
    let ok = tauri::async_runtime::spawn_blocking(|| helper::install_elevated(&helper::helper_exe()))
        .await
        .map_err(err)?
        .map_err(err)?;
    if !ok {
        return Err("cancelled".into());
    }
    let status = tauri::async_runtime::spawn_blocking(|| {
        let c = helper::Client::new().ok()?;
        c.ensure_running(std::time::Duration::from_secs(6)).then_some(())
    })
    .await
    .map_err(err)?
    .map_or(HelperStatus::Outdated, |_| HelperStatus::Ready);
    *app.state::<HelperState>().status.lock().unwrap() = status;
    let _ = app.emit("helper-status", status);
    Ok(status)
}

#[tauri::command]
pub fn scan(app: AppHandle, letter: char, on_event: Channel<ScanEvent>) -> Res<()> {
    scan::start(app, letter, on_event)
}

#[tauri::command]
pub fn cancel_scan(state: State<AppState>, letter: char) {
    if let Some(p) = state.drives.lock().unwrap().get(&letter).and_then(|d| d.scan.clone()) {
        p.cancel();
    }
}

#[derive(Serialize)]
pub struct RowsPage {
    total: usize,
    rows: Vec<Row>,
}

#[tauri::command]
pub fn rows(state: State<AppState>, letter: char, start: usize, count: usize) -> Res<RowsPage> {
    let drives = state.drives.lock().unwrap();
    let d = drives.get(&letter).ok_or("unknown drive")?;
    let Some(tree) = &d.tree else { return Ok(RowsPage { total: 0, rows: vec![] }) };
    let v = &d.view;
    let end = (start + count.min(500)).min(v.rows.len());
    let rows = v.rows[start.min(end)..end]
        .iter()
        .map(|&(id, depth)| row(tree, id, depth, v.expanded.contains(&id)))
        .collect();
    Ok(RowsPage { total: v.rows.len(), rows })
}

/// Expand or collapse a folder. Returns the new row count.
#[tauri::command]
pub fn toggle(state: State<AppState>, letter: char, id: NodeId, open: Option<bool>) -> Res<usize> {
    let mut drives = state.drives.lock().unwrap();
    let d = drives.get_mut(&letter).ok_or("unknown drive")?;
    let tree = d.tree.as_ref().ok_or("not scanned")?;
    let open = open.unwrap_or(!d.view.expanded.contains(&id));
    if open {
        d.view.expanded.insert(id);
    } else if id != tree.root {
        d.view.expanded.remove(&id);
    }
    d.view.rebuild(tree);
    Ok(d.view.rows.len())
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    Size,
    Alloc,
    Files,
    Name,
    Modified,
}

#[tauri::command]
pub fn set_sort(state: State<AppState>, letter: char, key: Sort) -> Res<usize> {
    let mut drives = state.drives.lock().unwrap();
    let d = drives.get_mut(&letter).ok_or("unknown drive")?;
    d.view.sort = match key {
        Sort::Size => SortKey::Size,
        Sort::Alloc => SortKey::Alloc,
        Sort::Files => SortKey::Files,
        Sort::Name => SortKey::Name,
        Sort::Modified => SortKey::Modified,
    };
    if let Some(tree) = &d.tree {
        d.view.rebuild(tree);
    }
    Ok(d.view.rows.len())
}

/// Expand ancestors so `id` is visible; returns its row index.
#[tauri::command]
pub fn reveal(state: State<AppState>, letter: char, id: NodeId) -> Res<Option<usize>> {
    let mut drives = state.drives.lock().unwrap();
    let d = drives.get_mut(&letter).ok_or("unknown drive")?;
    let tree = d.tree.as_ref().ok_or("not scanned")?;
    Ok(d.view.reveal(tree, id))
}

#[tauri::command]
pub fn node_path(state: State<AppState>, letter: char, id: NodeId) -> Res<String> {
    let drives = state.drives.lock().unwrap();
    let tree = drives.get(&letter).and_then(|d| d.tree.as_ref()).ok_or("not scanned")?;
    if id as usize >= tree.len() {
        return Err("bad node".into());
    }
    Ok(tree.path(id))
}

#[tauri::command]
pub fn show_in_explorer(path: String) -> Res<()> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.replace('"', "")))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn backdrop() -> &'static str {
    if crate::win::has_mica() { "mica" } else { "solid" }
}
