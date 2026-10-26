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

/// Change and save settings from the backend (e.g. remembering a declined prompt).
pub fn update(app: &AppHandle, f: impl FnOnce(&mut Settings)) {
    let state = app.state::<SettingsState>();
    let mut s = state.0.lock().unwrap();
    f(&mut s);
    let _ = bulk_core::settings::save(&s);
}

pub fn history_keep(app: &AppHandle) -> usize {
    get(app).history_keep.clamp(1, 365)
}

#[tauri::command]
pub fn settings_get(state: State<SettingsState>) -> Settings {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
pub async fn settings_set(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let mut s = settings;
    s.history_keep = s.history_keep.clamp(1, 365);
    let old = get(&app);
    // Only the backend tracks this.
    s.helper_declined = old.helper_declined;

    // The snapshot task exists only while automatic snapshots are on. The helper
    // creates it, since it's already elevated.
    if s.auto_snapshots != old.auto_snapshots {
        let on = s.auto_snapshots;
        tauri::async_runtime::spawn_blocking(move || {
            bulk_core::helper::Client::new()?.snapshot_task(on)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "Automatic snapshots need fast mode".to_string())?;
    }

    bulk_core::settings::save(&s).map_err(|e| e.to_string())?;
    crate::shell::set_explorer_menu(s.explorer_menu).map_err(|e| e.to_string())?;
    *app.state::<SettingsState>().0.lock().unwrap() = s.clone();

    if s.fast_mode != old.fast_mode {
        if s.fast_mode {
            // Turning it on by hand is a fresh request: ask again if needed.
            update(&app, |s| s.helper_declined = false);
            crate::scan::check_helper(app.clone());
        } else {
            crate::scan::helper_off(&app);
        }
    }
    Ok(get(&app))
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
    let shown = redirected(&root).unwrap_or(root);
    StorageInfo {
        path: shown.to_string_lossy().into_owned(),
        bytes,
        files,
    }
}

/// When Bulk is started by a packaged (MSIX) app, such as during development,
/// Windows silently redirects its `%LOCALAPPDATA%` writes into that package's
/// folder. Explorer runs outside the package and would open the empty real
/// folder, so find where a freshly written marker actually lands.
pub(crate) fn redirected(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let rest = path.strip_prefix(&local).ok()?;
    let marker = format!(".bulk-where-{}", std::process::id());
    std::fs::write(path.join(&marker), b"").ok()?;
    let found = std::fs::read_dir(local.join("Packages"))
        .ok()?
        .flatten()
        .map(|p| p.path().join(r"LocalCache\Local").join(rest))
        .find(|p| p.join(&marker).exists());
    let _ = std::fs::remove_file(path.join(&marker));
    found
}
