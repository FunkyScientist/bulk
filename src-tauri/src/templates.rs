//! Colour templates: JSON files in `%LOCALAPPDATA%\Bulk\templates`. The ones Bulk
//! ships are written there on first use; anyone can drop in their own.

use std::path::PathBuf;

use serde::Serialize;

/// Templates that ship with Bulk, by file name.
const SHIPPED: &[(&str, &str)] = &[
    ("default", include_str!("../templates/default.json")),
    ("fern", include_str!("../templates/fern.json")),
    ("tide", include_str!("../templates/tide.json")),
    ("ember", include_str!("../templates/ember.json")),
    ("graphite", include_str!("../templates/graphite.json")),
];

const README: &str = "\
Bulk colour templates
=====================

Every .json file in this folder shows up in Bulk under Settings > Appearance >
Colours. Copy one of the files here, rename it, change the colours and pick it in
Settings (press Reload if Settings is already open).

Format
------

{
  \"name\": \"My theme\",
  \"description\": \"One line shown under the name\",
  \"dark\":  { \"ui\": { ... }, \"files\": { ... }, \"branches\": [ ... ] },
  \"light\": { \"ui\": { ... }, \"files\": { ... }, \"branches\": [ ... ] }
}

Anything you leave out comes from the default template, so a template can be as
small as {\"name\": \"Red\", \"dark\": {\"ui\": {\"accent\": \"#ff5555\"}}}.
Colours are any CSS colour: #rrggbb, rgb(), rgba(), hsl()...

ui         Window and text
             app-bg, app-tint, sidebar-tint, panel, panel-2, popover, hover,
             line, line-strong, fg, fg-muted, fg-subtle, shadow (a box-shadow),
             backdrop (behind dialogs)
           Accent and controls
             accent, accent-fg, accent-soft, switch-off, switch-knob, bar-end
             (end of the size bars' gradient)
           Messages
             danger, danger-fg (text on danger buttons), ok, warn
           Drive usage
             usage, usage-low (under 10% free), usage-full (under 5% free)
           Status
             stale (\"Snapshot from ...\"), live (live dot on a drive)
           Live and cart markers
             added (new since the last scan), removed (deleted since then),
             cart (in the cleanup cart)
           Changes tab
             grew, shrank
           Charts
             chart-folder (folder fill), chart-hatch (folders of tiny files),
             chart-hover, chart-hover-line, chart-select (selected outline),
             chart-label (text on file tiles, or \"auto\" to pick dark or light
             per tile), chart-label-folder
           Title bar
             close-hover, close-fg, logo-1, logo-2, logo-3, logo-4

           Values can refer to other colours, like \"var(--danger)\" or
           \"color-mix(in oklab, var(--accent) 70%, white)\". The default template
           lists every key with its value.
files      colours in the treemap, sunburst and Types tab: folder, other, none,
           video, image, audio, document, spreadsheet, presentation, archive,
           program, system, code, diskimage, database, design, font, email, log
branches   sunburst colours for each top-level folder, in order (any number)

Bulk keeps its own templates (default, fern, tide, ember, graphite) up to date,
but never overwrites one you've edited. Delete one to get the original back. To
make your own, copy a file and give it a new name.
";

pub fn dir() -> PathBuf {
    bulk_core::snapshot::root().with_file_name("templates")
}

/// Write the shipped templates and the readme if they aren't there yet.
/// FNV-1a, to tell whether a shipped template was edited since Bulk wrote it.
fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Write the shipped templates and the readme. A shipped template that's already
/// there is only replaced by a newer version when nobody has edited it since Bulk
/// wrote it; `.shipped.json` remembers what was written.
fn seed(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let ledger_path = dir.join(".shipped.json");
    let mut ledger: std::collections::BTreeMap<String, String> = std::fs::read(&ledger_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let version = |bytes: &[u8]| {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|v| v.get("version").and_then(|v| v.as_u64()))
            .unwrap_or(1)
    };
    for (id, json) in SHIPPED {
        let p = dir.join(format!("{id}.json"));
        let write = match std::fs::read(&p) {
            Err(_) => true,
            Ok(have) => {
                let untouched = match ledger.get(*id) {
                    Some(f) => *f == fingerprint(&have),
                    // Written before the ledger existed: trust that it's ours.
                    None => version(&have) == 1,
                };
                untouched && version(&have) < version(json.as_bytes())
            }
        };
        if write && std::fs::write(&p, json).is_ok() {
            ledger.insert(id.to_string(), fingerprint(json.as_bytes()));
        }
    }
    if let Ok(b) = serde_json::to_vec_pretty(&ledger) {
        let _ = std::fs::write(ledger_path, b);
    }
    // The readme is Bulk's own documentation: always current.
    let _ = std::fs::write(dir.join("README.txt"), README);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    /// File name without `.json`; what settings remember.
    id: String,
    name: String,
    description: String,
    /// The file's contents; `None` when it couldn't be read.
    data: Option<serde_json::Value>,
    /// Why the file can't be used.
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Templates {
    /// Folder to open in Explorer.
    dir: String,
    list: Vec<Template>,
}

fn read(path: &std::path::Path, id: String) -> Template {
    let fail = |id: String, e: String| Template {
        name: id.clone(),
        id,
        description: String::new(),
        data: None,
        error: Some(e),
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return fail(id, e.to_string()),
    };
    // Tolerate a byte-order mark from editors like Notepad.
    let value: serde_json::Value = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(v) => v,
        Err(e) => return fail(id, format!("Not valid JSON: {e}")),
    };
    let Some(obj) = value.as_object() else {
        return fail(id, "Should be a JSON object".into());
    };
    if !obj.contains_key("dark") && !obj.contains_key("light") {
        return fail(id, "Needs a \"dark\" or \"light\" section".into());
    }
    let text = |k: &str| obj.get(k).and_then(|v| v.as_str()).map(str::to_string);
    Template {
        name: text("name").unwrap_or_else(|| id.clone()),
        description: text("description").unwrap_or_default(),
        id,
        data: Some(value),
        error: None,
    }
}

/// Every template in the folder, the default first.
#[tauri::command]
pub async fn templates() -> Templates {
    let dir = dir();
    seed(&dir);
    let mut list: Vec<Template> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
                })
                // Dot-files are Bulk's own bookkeeping (`.shipped.json`).
                .filter(|p| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .filter_map(|p| {
                    let id = p.file_stem()?.to_string_lossy().into_owned();
                    Some(read(&p, id))
                })
                .collect()
        })
        .unwrap_or_default();
    // The default always exists, even if its file was deleted or broken.
    if !list.iter().any(|t| t.id == "default" && t.error.is_none()) {
        list.retain(|t| t.id != "default");
        list.push(Template {
            id: "default".into(),
            name: "Bulk".into(),
            description: String::new(),
            data: serde_json::from_str(SHIPPED[0].1).ok(),
            error: None,
        });
    }
    list.sort_by(|a, b| {
        (b.id == "default")
            .cmp(&(a.id == "default"))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let shown = crate::settings::redirected(&dir).unwrap_or(dir);
    Templates {
        dir: shown.to_string_lossy().into_owned(),
        list,
    }
}
