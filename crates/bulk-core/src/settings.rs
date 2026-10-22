//! User settings shared by the app and the helper's scheduled snapshots, stored as
//! JSON in `%LOCALAPPDATA%\Bulk\settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Every {
    Daily,
    Weekly,
}

impl Every {
    /// Minimum gap between automatic snapshots. A little under the period, so a
    /// snapshot taken slightly late doesn't push the next one back a whole day.
    pub fn min_gap_secs(self) -> u64 {
        match self {
            Every::Daily => 20 * 3600,
            Every::Weekly => 6 * 86_400 + 12 * 3600,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Take snapshots in the background through the scheduled task.
    pub auto_snapshots: bool,
    pub snapshot_every: Every,
    /// Dated snapshots kept per drive.
    pub history_keep: usize,
    /// "Analyze with Bulk" in Explorer's right-click menu.
    pub explorer_menu: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_snapshots: true,
            snapshot_every: Every::Daily,
            history_keep: 30,
            explorer_menu: true,
        }
    }
}

pub fn path() -> PathBuf {
    crate::snapshot::root().with_file_name("settings.json")
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
