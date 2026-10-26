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

ui         app-bg, app-tint, sidebar-tint, panel, panel-2, popover, hover, line,
           line-strong, fg, fg-muted, fg-subtle, switch-off, accent, accent-fg,
           accent-soft, danger, ok, warn, shadow (a CSS box-shadow)
files      colours in the treemap, sunburst and Types tab: folder, other, none,
           video, image, audio, document, spreadsheet, presentation, archive,
           program, system, code, diskimage, database, design, font, email, log
branches   sunburst colours for each top-level folder, in order (any number)

Bulk writes its own templates (default, fern, tide, ember, graphite) only when
they're missing, so edits to them are kept. Delete one to get the original back.
";

pub fn dir() -> PathBuf {
    bulk_core::snapshot::root().with_file_name("templates")
}

/// Write the shipped templates and the readme if they aren't there yet.
fn seed(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    for (id, json) in SHIPPED {
        let p = dir.join(format!("{id}.json"));
        if !p.exists() {
            let _ = std::fs::write(p, json);
        }
    }
    let readme = dir.join("README.txt");
    if !readme.exists() {
        let _ = std::fs::write(readme, README);
    }
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
