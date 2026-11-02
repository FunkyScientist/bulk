//! Colours for `bulk find`, from the colour template picked in the app, so the
//! terminal matches it. Uses the template's dark half.

use std::collections::HashMap;

use ratatui::style::Color;
use serde_json::Value;

const DEFAULT: &str = include_str!("../../../src-tauri/templates/default.json");

pub struct Theme {
    pub accent: Color,
    pub danger: Color,
    pub ok: Color,
    pub warn: Color,
    pub muted: Color,
    pub subtle: Color,
    files: HashMap<String, Color>,
}

/// One mode's `ui` or `files` colours from the template picked in the app, over
/// the default template's, as the app does.
fn section(mode: &str, key: &str) -> HashMap<String, String> {
    let id = bulk_core::settings::load().template;
    let dir = bulk_core::snapshot::root().with_file_name("templates");
    let chosen: Option<Value> = std::fs::read_to_string(dir.join(format!("{id}.json")))
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok());
    let base: Value = serde_json::from_str(DEFAULT).unwrap_or(Value::Null);
    let mut m = HashMap::new();
    for t in [Some(&base), chosen.as_ref()].into_iter().flatten() {
        if let Some(o) = t[mode][key].as_object() {
            for (k, v) in o {
                if let Some(s) = v.as_str() {
                    m.insert(k.clone(), s.to_string());
                }
            }
        }
    }
    m
}

/// The template for HTML reports, so they look like the app.
pub fn report() -> bulk_core::export::Theme {
    let mode = |m: &str| bulk_core::export::ThemeMode {
        ui: section(m, "ui").into_iter().collect(),
        files: section(m, "files").into_iter().collect(),
    };
    bulk_core::export::Theme {
        light: mode("light"),
        dark: mode("dark"),
    }
}

impl Theme {
    pub fn load() -> Theme {
        let ui = section("dark", "ui");
        let files = section("dark", "files");
        let colour = |k: &str, fallback: Color| {
            resolve(&ui, ui.get(k).map(String::as_str).unwrap_or(""), 0).unwrap_or(fallback)
        };
        Theme {
            accent: colour("accent", Color::Magenta),
            danger: colour("danger", Color::Red),
            ok: colour("ok", Color::Green),
            warn: colour("warn", Color::Yellow),
            muted: colour("fg-muted", Color::Gray),
            subtle: colour("fg-subtle", Color::DarkGray),
            files: files
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), resolve(&ui, v, 0)?)))
                .collect(),
        }
    }

    /// Colour for a kind of file (`video`, `folder`…).
    pub fn file(&self, key: &str) -> Color {
        self.files
            .get(key)
            .or_else(|| self.files.get("other"))
            .copied()
            .unwrap_or(Color::Gray)
    }
}

/// `#rgb`, `#rrggbb`, `rgb()`/`rgba()`, or `var(--x)` naming another ui colour.
fn resolve(ui: &HashMap<String, String>, v: &str, depth: u8) -> Option<Color> {
    let v = v.trim();
    if let Some(name) = v.strip_prefix("var(--").and_then(|s| s.strip_suffix(')')) {
        return (depth < 4)
            .then(|| resolve(ui, ui.get(name.trim())?, depth + 1))
            .flatten();
    }
    if let Some(hex) = v.strip_prefix('#') {
        let n = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            6 | 8 => Some(Color::Rgb(n(&hex[0..2])?, n(&hex[2..4])?, n(&hex[4..6])?)),
            3 => {
                let d = |i: usize| n(&hex[i..i + 1]).map(|x| x * 17);
                Some(Color::Rgb(d(0)?, d(1)?, d(2)?))
            }
            _ => None,
        };
    }
    let inner = v
        .strip_prefix("rgba(")
        .or_else(|| v.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let mut parts = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.trim()
                .parse::<f32>()
                .ok()
                .map(|x| x.clamp(0.0, 255.0) as u8)
        });
    Some(Color::Rgb(parts.next()??, parts.next()??, parts.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        let ui: HashMap<String, String> = [
            ("ok".to_string(), "#3fd0a4".to_string()),
            ("added".to_string(), "var(--ok)".to_string()),
        ]
        .into();
        assert_eq!(
            resolve(&ui, "var(--added)", 0),
            Some(Color::Rgb(0x3f, 0xd0, 0xa4))
        );
        assert_eq!(resolve(&ui, "#fff", 0), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(resolve(&ui, "rgb(1, 2, 3)", 0), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(resolve(&ui, "color-mix(in oklab, red, blue)", 0), None);
        assert!(matches!(Theme::load().file("video"), Color::Rgb(..)));
    }
}
