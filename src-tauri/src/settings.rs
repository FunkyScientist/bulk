//! Settings commands. The settings themselves live in `bulk_core::settings` so the
//! helper's scheduled snapshots read the same file.

use std::sync::Mutex;

pub use bulk_core::settings::Settings;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

pub struct SettingsState(pub Mutex<Settings>);

pub fn get(app: &AppHandle) -> Settings {
    app.state::<SettingsState>().0.lock().unwrap().clone()
}

pub fn history_keep(app: &AppHandle) -> usize {
    get(app).history_keep.clamp(1, 365)
}

#[tauri::command]
pub fn settings_get(state: State<SettingsState>) -> Settings {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
pub fn settings_set(state: State<SettingsState>, settings: Settings) -> Result<Settings, String> {
    let mut s = settings;
    s.history_keep = s.history_keep.clamp(1, 365);
    bulk_core::settings::save(&s).map_err(|e| e.to_string())?;
    crate::shell::set_explorer_menu(s.explorer_menu).map_err(|e| e.to_string())?;
    *state.0.lock().unwrap() = s.clone();
    Ok(s)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    path: String,
    bytes: u64,
    files: usize,
}

/// How much space saved snapshots take.
#[tauri::command]
pub async fn snapshot_storage() -> StorageInfo {
    let root = bulk_core::snapshot::root();
    let (mut bytes, mut files) = (0, 0);
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => {
                    bytes += m.len();
                    files += 1;
                }
                Err(_) => {}
            }
        }
    }
    StorageInfo {
        path: root.to_string_lossy().into_owned(),
        bytes,
        files,
    }
}
