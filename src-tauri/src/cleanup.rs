//! The Cleanup tab: space that can usually be got back (see `bulk_core::junk`),
//! the suggestions the user hid, and the actions for findings that aren't deleted
//! file by file.

use bulk_core::junk::{self, Finding, Safety};
use bulk_core::tree::NodeId;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::AppState;

type Res<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    /// Full path of the folder the findings cover.
    scope: String,
    /// What's marked safe to delete, bytes.
    safe: u64,
    /// What's worth a look first, bytes.
    review: u64,
    findings: Vec<Finding>,
    /// Kinds and paths the user asked not to see.
    hidden: Vec<String>,
}

/// Findings under `folder` (or the whole drive), without the hidden ones.
#[tauri::command]
pub async fn cleanup(
    app: AppHandle,
    state: State<'_, AppState>,
    letter: char,
    folder: Option<NodeId>,
) -> Res<CleanupResult> {
    let hidden = crate::settings::get(&app).cleanup_ignore;
    let drives = state.drives.lock().unwrap();
    let tree = drives
        .get(&letter)
        .and_then(|d| d.tree.as_ref())
        .ok_or("not scanned")?;
    let root = folder
        .filter(|&f| (f as usize) < tree.len() && tree.get(f).is_dir())
        .unwrap_or(tree.root);
    let findings = junk::without(junk::scan(tree, root, crate::scan::now()), &hidden);
    let sum = |s: Safety| {
        findings
            .iter()
            .filter(|f| f.safety == s)
            .map(|f| f.alloc)
            .sum()
    };
    Ok(CleanupResult {
        scope: tree.path(root),
        safe: sum(Safety::Safe),
        review: sum(Safety::Review),
        findings,
        hidden,
    })
}

/// Stop (or start again) suggesting a kind of finding or one path.
#[tauri::command]
pub fn cleanup_ignore(app: AppHandle, entry: String, ignore: bool) -> Vec<String> {
    crate::settings::update(&app, |s| {
        s.cleanup_ignore.retain(|e| *e != entry);
        if ignore {
            s.cleanup_ignore.push(entry);
        }
    });
    crate::settings::get(&app).cleanup_ignore
}

/// Empty every Recycle Bin. Windows asks first and shows its own progress.
#[tauri::command]
pub async fn empty_recycle_bin() -> Res<()> {
    tauri::async_runtime::spawn_blocking(|| {
        use windows_sys::Win32::UI::Shell::SHEmptyRecycleBinW;
        let r = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), 0) };
        // Cancelling the confirmation isn't an error worth showing.
        const CANCELLED: i32 = 0x800704C7u32 as i32;
        if r != 0 && r != CANCELLED {
            return Err(format!("couldn't empty the Recycle Bin (error {r:#x})"));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Windows' Disk Cleanup, for what Windows has to remove itself (update files).
#[tauri::command]
pub fn disk_cleanup() -> Res<()> {
    std::process::Command::new("cleanmgr.exe")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
