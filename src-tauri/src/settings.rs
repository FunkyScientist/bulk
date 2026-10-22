//! User settings, stored as JSON next to the snapshots.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Dated snapshots kept per drive.
    pub history_keep: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { history_keep: 30 }
    }
}

pub struct SettingsState(pub Mutex<Settings>);

fn path() -> PathBuf {
    bulk_core::snapshot::root().with_file_name("settings.json")
}

pub fn load() -> Settings {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(s)?)
}

pub fn get(app: &AppHandle) -> Settings {
    app.state::<SettingsState>().0.lock().unwrap().clone()
}

pub fn history_keep(app: &AppHandle) -> usize {
    get(app).history_keep.clamp(1, 365)
}
