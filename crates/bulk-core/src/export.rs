//! Reports: a self-contained HTML page, a CSV list of everything, and a JSON tree
//! of folders.

use std::fmt::Write as _;
use std::io::{BufWriter, Write};

use serde::{Deserialize, Serialize};

use crate::diff;
use crate::layout::{self, TreemapOptions, category, tile_flags};
use crate::tree::{NodeId, SortKey, Tree, flags};

const CATEGORY_NAMES: [&str; 11] = [
    "Folders",
    "Other",
    "Video",
    "Images",
    "Audio",
    "Archives",
    "Code",
    "Documents",
    "Programs",
    "Disk images",
    "Data & logs",
];
/// The chart categories (see `layout::category`) as file kinds in a colour template.
const CATEGORY_KINDS: [&str; 11] = [
    "folder",
    "other",
    "video",
    "image",
    "audio",
    "archive",
    "code",
    "document",
    "program",
    "diskimage",
    "database",
];

/// Category colour as CSS, set per theme in the page's style sheet.
fn cat_color(c: usize) -> String {
    format!("var(--c{c})")
}

/// One light/dark half of the app's colour template: `ui` colours by name (as the
/// app's CSS variables) and file-kind colours.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ThemeMode {
    pub ui: std::collections::BTreeMap<String, String>,
    pub files: std::collections::BTreeMap<String, String>,
}

/// The colour template in use, so reports look like the app.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub light: ThemeMode,
    pub dark: ThemeMode,
}

/// Values go into a style sheet; anything that could end a rule or the `<style>`
/// element is dropped.
fn safe(v: &str) -> Option<&str> {
    let v = v.trim();
    (!v.is_empty() && !v.contains(['<', '>', '{', '}', ';', '\\'])).then_some(v)
}

fn safe_key(k: &str) -> bool {
    !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl ThemeMode {
    /// Declarations for one mode: the template's own variables (so values like
    /// `var(--danger)` resolve), then the report's names mapped onto them.
    fn vars(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.ui {
            if let (true, Some(v)) = (safe_key(k), safe(v)) {
                let _ = write!(s, "--{k}:{v};");
            }
        }
        let has = |k: &str| self.ui.get(k).and_then(|v| safe(v)).is_some();
        for (report, app) in [
            ("bg", "app-bg"),
            ("card", "popover"),
            ("muted", "fg-muted"),
            ("subtle", "fg-subtle"),
            ("track", "panel-2"),
            ("folder", "chart-folder"),
            ("collapsed", "accent-soft"),
            ("up", "grew"),
            ("down", "shrank"),
        ] {
            if has(app) {
                let _ = write!(s, "--{report}:var(--{app});");
            }
        }
        for (i, kind) in CATEGORY_KINDS.iter().enumerate() {
            if let Some(v) = self.files.get(*kind).and_then(|v| safe(v)) {
                let _ = write!(s, "--c{i}:{v};");
            }
        }
        s
    }
}

impl Theme {
    fn css(&self) -> String {
        format!(
            ":root{{{}}}@media (prefers-color-scheme:dark){{:root{{{}}}}}",
            self.light.vars(),
            self.dark.vars()
        )
    }
}

/// `2024-03-09T14:05:00Z` from unix seconds; empty for 0.
pub fn iso(unix: u32) -> String {
    if unix == 0 {
        return String::new();
    }
    // Civil from days (Howard Hinnant).
    let days = unix as i64 / 86_400;
    let secs = unix as i64 % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Every file and folder under `root`, one row each.
pub fn write_csv(tree: &Tree, root: NodeId, out: impl Write) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, out);
    // A BOM so Excel reads non-ASCII names correctly.
    w.write_all("\u{feff}path,type,size,size_on_disk,files,modified\r\n".as_bytes())?;
    let mut stack = vec![(root, tree.path(root))];
    while let Some((id, path)) = stack.pop() {
        let n = tree.get(id);
        writeln!(
            w,
            "{},{},{},{},{},{}\r",
            csv_field(&path),
            if n.is_dir() { "folder" } else { "file" },
            n.size,
            n.alloc,
            n.files,
            iso(n.mtime)
        )?;
        for c in tree.children(id) {
            let base = path.trim_end_matches('\\');
            stack.push((c, format!("{base}\\{}", tree.name(c))));
        }
    }
    w.flush()
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Folders under `root` as nested JSON (files are summarised in each folder's
/// totals rather than listed, which keeps whole-drive exports manageable).
pub fn write_json(tree: &Tree, root: NodeId, out: impl Write) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, out);
    fn folder(tree: &Tree, id: NodeId, w: &mut impl Write) -> std::io::Result<()> {
        let n = tree.get(id);
        write!(
            w,
            "{{\"name\":{},\"size\":{},\"sizeOnDisk\":{},\"files\":{},\"modified\":\"{}\",\"children\":[",
            json_str(tree.name(id)),
            n.size,
            n.alloc,
            n.files,
            iso(n.mtime)
        )?;
        let mut first = true;
        for c in tree.sorted_children(id, SortKey::Alloc) {
            if !tree.get(c).is_dir() {
                continue;
            }
            if !first {
                w.write_all(b",")?;
            }
            first = false;
            folder(tree, c, w)?;
        }
        w.write_all(b"]}")
    }
    write!(w, "{{\"path\":{},\"root\":", json_str(&tree.path(root)))?;
    folder(tree, root, &mut w)?;
    w.write_all(b"}")?;
    w.flush()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn human(b: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

const REPORT_CSS: &str = r#"
:root{--bg:#f7f7fa;--card:#fff;--fg:#16161d;--muted:#5f5f70;--subtle:#8c8c9c;--line:rgba(16,16,40,.08);--track:rgba(16,16,40,.06);--folder:rgba(20,20,60,.045);--collapsed:rgba(106,88,245,.16);--accent:#6a58f5;--shadow:0 1px 2px rgba(16,16,40,.04),0 8px 24px -12px rgba(16,16,40,.12);--up:#e5405e;--down:#17a67b;--logo-1:var(--accent);--c0:#6a58f5;--c1:#9aa1b2;--c2:#f0506f;--c3:#e0a020;--c4:#a865f5;--c5:#f27c35;--c6:#16b2aa;--c7:#3b88f0;--c8:#7a6cf2;--c9:#e055a8;--c10:#6f9a8d}
@media (prefers-color-scheme:dark){:root{--bg:#0f0f14;--card:#17171e;--fg:#ececf2;--muted:#a3a3b5;--subtle:#6e6e80;--line:rgba(255,255,255,.07);--track:rgba(255,255,255,.06);--folder:rgba(255,255,255,.03);--collapsed:rgba(139,124,255,.2);--accent:#8b7cff;--shadow:0 1px 2px rgba(0,0,0,.3),0 12px 32px -16px rgba(0,0,0,.6);--up:#ff6b81;--down:#3fd0a4;--c0:#8b7cff;--c1:#7d8496;--c2:#ff6b8b;--c3:#f5b544;--c4:#c38bff;--c5:#ff9a5a;--c6:#3fd0c9;--c7:#5fa8ff;--c8:#9d92ff;--c9:#f37bc4;--c10:#8fb3a8}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.55 "Geist",ui-sans-serif,system-ui,"Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}
.mono{font-family:"Geist Mono",ui-monospace,Consolas,monospace}
.num{font-variant-numeric:tabular-nums}
main{max-width:1200px;margin:0 auto;padding:40px 24px 72px}
.brand{display:flex;align-items:center;gap:8px;color:var(--muted);font-size:13px;font-weight:500}
.brand svg{width:18px;height:18px}
header{display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:space-between;gap:12px 24px;margin-top:28px}
h1{font-size:34px;line-height:1.1;margin:0;font-weight:600;letter-spacing:-.03em}
.path{margin-top:8px;color:var(--muted);font-size:12.5px;word-break:break-all}
.date{color:var(--subtle);font-size:12.5px}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px;margin-top:28px}
.stat{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:16px 18px;box-shadow:var(--shadow)}
.stat span{display:block;color:var(--muted);font-size:12.5px}
.stat b{display:block;margin-top:4px;font-size:26px;font-weight:600;letter-spacing:-.02em}
.card{background:var(--card);border:1px solid var(--line);border-radius:16px;padding:20px 22px;margin-top:16px;box-shadow:var(--shadow)}
h2{display:flex;align-items:baseline;justify-content:space-between;gap:12px;font-size:15px;font-weight:600;letter-spacing:-.01em;margin:0 0 14px}
h2 small{font-size:12px;font-weight:400;color:var(--subtle)}
.map svg{width:100%;height:auto;display:block;border-radius:10px}
.map text{font-size:11px;font-family:"Geist",system-ui,sans-serif;pointer-events:none}
.map .h{fill:var(--muted);font-weight:500}.map .f{fill:rgba(12,12,20,.85);font-weight:500}
.map rect:hover{stroke:var(--fg);stroke-width:1.5}
#tip{position:fixed;z-index:9;pointer-events:none;max-width:420px;padding:8px 11px;border-radius:10px;background:var(--card);border:1px solid var(--line);box-shadow:var(--shadow);font-size:12.5px;opacity:0;transition:opacity .1s}
#tip b{display:block;font-weight:500;word-break:break-all}#tip span{color:var(--muted)}
.types{display:flex;gap:2px;height:10px;border-radius:6px;overflow:hidden}.types span{display:block;height:100%}
.legend{list-style:none;padding:0;margin:18px 0 0;display:grid;grid-template-columns:repeat(auto-fill,minmax(230px,1fr));gap:8px 28px}
.legend li{display:flex;align-items:center;gap:10px;font-size:13px}
.legend i{width:10px;height:10px;border-radius:3px;flex:none}
.legend b{margin-left:auto;font-weight:500}.legend em{width:52px;text-align:right;color:var(--subtle);font-style:normal}
.grid{display:grid;grid-template-columns:1fr 1fr;gap:16px}
@media (max-width:920px){.grid{grid-template-columns:1fr}}
ol{list-style:none;margin:0;padding:0}
ol li{display:grid;grid-template-columns:22px minmax(0,1fr) auto;gap:3px 10px;align-items:center;padding:10px 0;border-top:1px solid var(--line)}
ol li:first-child{border-top:0;padding-top:0}
.rank{color:var(--subtle);font-size:12px}
.name{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:500}
.name i{display:inline-block;width:8px;height:8px;border-radius:2px;margin-right:7px;vertical-align:1px}
.size{text-align:right;font-weight:500}
.where{grid-column:2;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--subtle);font-size:11.5px}
.pct{grid-column:3;text-align:right;color:var(--subtle);font-size:11.5px}
.bar{grid-column:2/4;height:4px;margin-top:3px;border-radius:2px;background:var(--track);overflow:hidden}
.bar span{display:block;height:100%;border-radius:2px;background:var(--accent)}
footer{margin-top:36px;display:flex;justify-content:space-between;color:var(--subtle);font-size:12px}
footer a{color:var(--muted);text-decoration:none;font-weight:500}footer a:hover{color:var(--accent);text-decoration:underline}
@media print{body{background:#fff}.card,.stat{box-shadow:none;break-inside:avoid}#tip{display:none}}
"#;

const REPORT_JS: &str = r#"
const tip=document.getElementById('tip'),map=document.querySelector('.map svg');
map.addEventListener('mousemove',e=>{
  const r=e.target.closest('rect');
  if(!r){tip.style.opacity=0;return}
  const b=document.createElement('b'),s=document.createElement('span');
  b.textContent=r.dataset.p;s.textContent=r.dataset.s;
  tip.replaceChildren(b,s);
  const x=Math.min(e.clientX+14,innerWidth-tip.offsetWidth-8);
  const y=e.clientY+18+tip.offsetHeight>innerHeight?e.clientY-tip.offsetHeight-12:e.clientY+18;
  tip.style.left=x+'px';tip.style.top=y+'px';tip.style.opacity=1;
});
map.addEventListener('mouseleave',()=>tip.style.opacity=0);
"#;

const LOGO: &str = r##"<svg viewBox="0 0 1024 1024" aria-hidden="true"><rect x="64" y="64" width="896" height="896" rx="224" style="fill:var(--fg)"/><rect x="232" y="232" width="288" height="560" rx="40" style="fill:var(--bg)"/><rect x="232" y="504" width="560" height="288" rx="40" style="fill:var(--bg)"/><rect x="560" y="232" width="232" height="232" rx="40" style="fill:var(--logo-1)"/></svg>"##;

/// A single-file HTML report: treemap, largest folders and files, and file types.
pub fn write_html(
    tree: &Tree,
    root: NodeId,
    title: &str,
    generated: u32,
    theme: &Theme,
    out: impl Write,
) -> std::io::Result<()> {
    let r = tree.get(root);
    let total = r.alloc.max(1);

    // Largest folders and files, and space per file type, in one pass.
    let mut folders: Vec<NodeId> = Vec::new();
    let mut files: Vec<NodeId> = Vec::new();
    let mut by_cat = [0u64; 11];
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        for c in tree.children(id) {
            let n = tree.get(c);
            if n.has(flags::EXTRA_LINK) {
                continue;
            }
            if n.is_dir() {
                folders.push(c);
                stack.push(c);
            } else {
                files.push(c);
                by_cat[category(tree.name(c)) as usize] += n.alloc;
            }
        }
    }
    let folder_count = folders.len() as u64;
    let top = |v: &mut Vec<NodeId>| {
        let k = v.len().min(25);
        if k > 0 {
            v.select_nth_unstable_by(k - 1, |&a, &b| tree.get(b).alloc.cmp(&tree.get(a).alloc));
            v.truncate(k);
            v.sort_by(|&a, &b| tree.get(b).alloc.cmp(&tree.get(a).alloc));
        }
    };
    top(&mut folders);
    top(&mut files);

    // Treemap as SVG; the page script shows each tile's path and size on hover.
    let (tw, th) = (1200.0f32, 620.0f32);
    let tiles = layout::treemap(
        tree,
        root,
        &TreemapOptions {
            width: tw,
            height: th,
            max_tiles: 3500,
            min_folder_area: 400.0,
            ..Default::default()
        },
        None,
    );
    let mut svg = String::new();
    for t in &tiles {
        let n = tree.get(t.id);
        let dir_open = t.flags & tile_flags::DIR != 0 && t.flags & tile_flags::COLLAPSED == 0;
        let fill = if dir_open {
            "var(--folder)"
        } else if t.flags & tile_flags::DIR != 0 {
            "var(--collapsed)"
        } else {
            &cat_color(t.category as usize)
        };
        let _ = write!(
            svg,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="3" fill="{fill}" data-p="{}" data-s="{} · {:.1}%"/>"#,
            t.rect.x + 0.5,
            t.rect.y + 0.5,
            (t.rect.w - 1.0).max(0.5),
            (t.rect.h - 1.0).max(0.5),
            esc(&tree.path(t.id)),
            human(n.alloc),
            n.alloc as f64 / total as f64 * 100.0
        );
        let class = if t.flags & tile_flags::DIR != 0 {
            "h"
        } else {
            "f"
        };
        let (show, dy) = if t.flags & tile_flags::HEADER != 0 {
            (true, 12.0)
        } else {
            (t.rect.w > 60.0 && t.rect.h > 22.0 && !dir_open, 16.0)
        };
        if show {
            let max = ((t.rect.w - 12.0) / 6.5).max(0.0) as usize;
            let name: String = tree.name(t.id).chars().take(max).collect();
            let _ = write!(
                svg,
                r#"<text x="{:.1}" y="{:.1}" class="{class}">{}</text>"#,
                t.rect.x + 6.0,
                t.rect.y + dy,
                esc(&name)
            );
        }
    }

    let list = |ids: &[NodeId]| {
        let max = ids.first().map_or(1, |&i| tree.get(i).alloc.max(1));
        let mut s = String::new();
        for (rank, &id) in ids.iter().enumerate() {
            let n = tree.get(id);
            let swatch = if n.is_dir() {
                String::new()
            } else {
                format!(
                    r#"<i style="background:{}"></i>"#,
                    cat_color(category(tree.name(id)) as usize)
                )
            };
            let _ = write!(
                s,
                r#"<li><span class="rank num">{rank}</span><span class="name" title="{path}">{swatch}{name}</span><span class="size num">{size}</span><span class="where mono" title="{path}">{dir}</span><span class="pct num">{pct:.1}%</span><span class="bar"><span style="width:{w:.1}%"></span></span></li>"#,
                rank = rank + 1,
                path = esc(&tree.path(id)),
                name = esc(tree.name(id)),
                dir = esc(&tree.path(n.parent)),
                w = n.alloc as f64 / max as f64 * 100.0,
                size = human(n.alloc),
                pct = n.alloc as f64 / total as f64 * 100.0
            );
        }
        s
    };

    let mut cats: Vec<(usize, u64)> = by_cat
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, v)| v > 0)
        .collect();
    cats.sort_by(|a, b| b.1.cmp(&a.1));
    let cat_total: u64 = cats.iter().map(|c| c.1).sum::<u64>().max(1);
    let mut bar = String::new();
    let mut legend = String::new();
    for (c, v) in &cats {
        let pct = *v as f64 / cat_total as f64 * 100.0;
        let _ = write!(
            bar,
            r#"<span style="flex:{pct:.3};background:{}" title="{} · {}"></span>"#,
            cat_color(*c),
            CATEGORY_NAMES[*c],
            human(*v)
        );
        let _ = write!(
            legend,
            r#"<li><i style="background:{}"></i>{}<b class="num">{}</b><em class="num">{pct:.1}%</em></li>"#,
            cat_color(*c),
            CATEGORY_NAMES[*c],
            human(*v)
        );
    }

    let mut w = BufWriter::new(out);
    write!(
        w,
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · Bulk report</title>
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">
<style>{css}{theme}</style></head><body><main>
<div class="brand">{logo}Bulk report</div>
<header><div><h1>{title}</h1><div class="path mono">{path}</div></div><div class="date num">{date}</div></header>
<section class="stats">
<div class="stat"><span>On disk</span><b class="num">{size}</b></div>
<div class="stat"><span>Total size</span><b class="num">{logical}</b></div>
<div class="stat"><span>Files</span><b class="num">{count}</b></div>
<div class="stat"><span>Folders</span><b class="num">{folder_count}</b></div>
</section>
<section class="card"><h2>Map <small>Hover a block for its path and size</small></h2><div class="map"><svg viewBox="0 0 {tw} {th}" role="img" aria-label="Treemap">{svg}</svg></div></section>
<section class="card"><h2>File types</h2><div class="types">{bar}</div><ul class="legend">{legend}</ul></section>
<div class="grid"><section class="card"><h2>Largest folders</h2><ol>{folders}</ol></section><section class="card"><h2>Largest files</h2><ol>{file_rows}</ol></section></div>
<footer><span>Made with <a href="https://github.com/FunkyScientist/bulk" target="_blank" rel="noopener">Bulk</a></span><span class="num">{date}</span></footer></main>
<div id="tip"></div><script>{js}</script></body></html>"#,
        css = REPORT_CSS,
        theme = theme.css(),
        js = REPORT_JS,
        logo = LOGO,
        title = esc(title),
        path = esc(&tree.path(root)),
        date = iso(generated).replace('T', " ").replace('Z', " UTC"),
        size = human(r.alloc),
        count = thousands(r.files as u64),
        folder_count = thousands(folder_count),
        logical = human(r.size),
        folders = list(&folders),
        file_rows = list(&files),
    )?;
    w.flush()
}

/// One changed file or folder in a "what changed" export.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRow {
    pub path: String,
    pub status: diff::Status,
    pub dir: bool,
    pub old_alloc: u64,
    pub new_alloc: u64,
    pub delta: i64,
    pub old_files: u32,
    pub new_files: u32,
}

/// Everything that changed under a folder, depth first, biggest change first within
/// each folder. Stops after `limit` rows.
pub fn collect_changes(
    old: &Tree,
    new: &Tree,
    old_root: Option<NodeId>,
    new_root: NodeId,
    limit: usize,
    filters: diff::Filters,
) -> Vec<ChangeRow> {
    let mut out = Vec::new();
    let base = new.path(new_root);
    let mut stack = vec![(old_root, Some(new_root), base)];
    while let Some((o, n, path)) = stack.pop() {
        let items = diff::children(old, new, o, n, usize::MAX, filters);
        // Reverse so the biggest change is visited (and written) first.
        for it in items.into_iter().rev() {
            let child = format!("{}\\{}", path.trim_end_matches('\\'), it.name);
            if it.dir && it.has_kids {
                stack.push((it.old_id, it.new_id, child.clone()));
            }
            out.push(ChangeRow {
                path: child,
                status: it.status,
                dir: it.dir,
                old_alloc: it.old_alloc,
                new_alloc: it.new_alloc,
                delta: it.delta,
                old_files: it.old_files,
                new_files: it.new_files,
            });
        }
        if out.len() >= limit {
            out.truncate(limit);
            break;
        }
    }
    // Depth-first order with each folder's own row ahead of its contents.
    out.sort_by(|a, b| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
    });
    out
}

fn status_word(s: diff::Status) -> &'static str {
    match s {
        diff::Status::Added => "added",
        diff::Status::Removed => "removed",
        diff::Status::Changed => "changed",
    }
}

pub fn write_changes_csv(rows: &[ChangeRow], out: impl Write) -> std::io::Result<()> {
    let mut w = BufWriter::new(out);
    w.write_all(
        "\u{feff}path,type,status,size_on_disk_before,size_on_disk_after,change,files_before,files_after\r\n"
            .as_bytes(),
    )?;
    for r in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{}\r",
            csv_field(&r.path),
            if r.dir { "folder" } else { "file" },
            status_word(r.status),
            r.old_alloc,
            r.new_alloc,
            r.delta,
            r.old_files,
            r.new_files
        )?;
    }
    w.flush()
}

/// Header facts shared by the change exports.
pub struct ChangeSummary<'a> {
    pub scope: &'a str,
    pub since: u32,
    pub generated: u32,
    pub old_alloc: u64,
    pub new_alloc: u64,
    pub old_files: u32,
    pub new_files: u32,
}

pub fn write_changes_json(
    s: &ChangeSummary,
    rows: &[ChangeRow],
    out: impl Write,
) -> std::io::Result<()> {
    let mut w = BufWriter::new(out);
    let v = serde_json::json!({
        "scope": s.scope,
        "since": iso(s.since),
        "generated": iso(s.generated),
        "sizeOnDiskBefore": s.old_alloc,
        "sizeOnDiskAfter": s.new_alloc,
        "filesBefore": s.old_files,
        "filesAfter": s.new_files,
        "changes": rows,
    });
    serde_json::to_writer_pretty(&mut w, &v)?;
    w.flush()
}

fn signed(n: i64) -> String {
    match n.cmp(&0) {
        std::cmp::Ordering::Equal => "±0 B".into(),
        std::cmp::Ordering::Greater => format!("+{}", human(n as u64)),
        std::cmp::Ordering::Less => format!("−{}", human(n.unsigned_abs())),
    }
}

const LIST_CSS: &str = r#"
.up{color:var(--up)}.down{color:var(--down)}
.tag{display:inline-block;margin-left:8px;padding:0 6px;border-radius:4px;font-size:11px;font-weight:500;vertical-align:1px}
.tag.added{background:color-mix(in oklab,var(--up) 12%,transparent);color:var(--up)}.tag.removed{background:color-mix(in oklab,var(--down) 12%,transparent);color:var(--down)}
.set{padding:12px 0;border-top:1px solid var(--line)}.set:first-child{border-top:0;padding-top:0}
.set h3{display:flex;gap:10px;align-items:baseline;margin:0 0 6px;font-size:14px;font-weight:600}
.set h3 small{color:var(--subtle);font-weight:400;font-size:12px}.set h3 em{margin-left:auto;font-style:normal;font-size:12.5px}
.set div{display:flex;gap:12px;padding:3px 0;font-size:12.5px;color:var(--muted)}.set div span:first-child{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.set div span:last-child{margin-left:auto;flex:none;color:var(--subtle)}
"#;

fn page(title: &str, subtitle: &str, stats: &str, body: &str, theme: &Theme) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{t} · Bulk report</title>
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">
<style>{css}{list}{theme}</style></head><body><main>
<div class="brand">{logo}Bulk report</div>
<header><div><h1>{t}</h1><div class="path mono">{sub}</div></div></header>
<section class="stats">{stats}</section>
{body}
<footer><span>Made with <a href="https://github.com/FunkyScientist/bulk" target="_blank" rel="noopener">Bulk</a></span></footer></main></body></html>"#,
        t = esc(title),
        sub = esc(subtitle),
        css = REPORT_CSS,
        list = LIST_CSS,
        theme = theme.css(),
        logo = LOGO,
    )
}

fn stat(label: &str, value: &str, class: &str) -> String {
    format!(r#"<div class="stat"><span>{label}</span><b class="num {class}">{value}</b></div>"#)
}

/// A shareable page of what changed: totals, then the biggest changes. Folders that
/// merely contain changes are left out of the list, since their files are in it.
pub fn write_changes_html(
    s: &ChangeSummary,
    rows: &[ChangeRow],
    theme: &Theme,
    out: impl Write,
) -> std::io::Result<()> {
    let delta = s.new_alloc as i64 - s.old_alloc as i64;
    let files = s.new_files as i64 - s.old_files as i64;
    let mut stats = String::new();
    stats += &stat(
        "Change",
        &signed(delta),
        if delta > 0 { "up" } else { "down" },
    );
    stats += &stat("Now", &human(s.new_alloc), "");
    stats += &stat("Then", &human(s.old_alloc), "");
    stats += &stat(
        "Files",
        &format!(
            "{}{}",
            if files >= 0 { "+" } else { "−" },
            thousands(files.unsigned_abs())
        ),
        "",
    );

    let mut list: Vec<&ChangeRow> = rows
        .iter()
        .filter(|r| !r.dir || r.status != diff::Status::Changed)
        .collect();
    list.sort_by(|a, b| b.delta.unsigned_abs().cmp(&a.delta.unsigned_abs()));
    list.truncate(300);
    let max = list.first().map_or(1, |r| r.delta.unsigned_abs().max(1));
    let mut items = String::new();
    for (i, r) in list.iter().enumerate() {
        let (dir, name) = r.path.rsplit_once('\\').unwrap_or(("", &r.path));
        let tag = match r.status {
            diff::Status::Added => r#"<span class="tag added">New</span>"#,
            diff::Status::Removed => r#"<span class="tag removed">Deleted</span>"#,
            diff::Status::Changed => "",
        };
        let _ = write!(
            items,
            r#"<li><span class="rank num">{rank}</span><span class="name" title="{path}">{name}{tag}</span><span class="size num {cls}">{d}</span><span class="where mono" title="{path}">{dir}</span><span class="pct num">{a} → {b}</span><span class="bar"><span style="width:{w:.1}%;background:{color}"></span></span></li>"#,
            rank = i + 1,
            path = esc(&r.path),
            name = esc(name),
            dir = esc(dir),
            cls = if r.delta > 0 { "up" } else { "down" },
            d = signed(r.delta),
            a = human(r.old_alloc),
            b = human(r.new_alloc),
            w = r.delta.unsigned_abs() as f64 / max as f64 * 100.0,
            color = if r.delta > 0 {
                "var(--up)"
            } else {
                "var(--down)"
            },
        );
    }
    let body = format!(
        r#"<section class="card"><h2>Biggest changes <small>{n} items</small></h2><ol>{items}</ol></section>"#,
        n = thousands(list.len() as u64),
    );
    let since = iso(s.since).replace('T', " ");
    let subtitle = format!("{} · since {}", s.scope, since.replace('Z', " UTC"));
    let mut w = BufWriter::new(out);
    w.write_all(page("What changed", &subtitle, &stats, &body, theme).as_bytes())?;
    w.flush()
}

/// A set of identical files for the duplicates export.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeSet {
    pub size: u64,
    pub files: Vec<DupeFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeFile {
    pub path: String,
    pub mtime: u32,
}

pub fn write_dupes_csv(sets: &[DupeSet], out: impl Write) -> std::io::Result<()> {
    let mut w = BufWriter::new(out);
    w.write_all("\u{feff}set,path,size,modified\r\n".as_bytes())?;
    for (i, s) in sets.iter().enumerate() {
        for f in &s.files {
            writeln!(
                w,
                "{},{},{},{}\r",
                i + 1,
                csv_field(&f.path),
                s.size,
                iso(f.mtime)
            )?;
        }
    }
    w.flush()
}

pub fn write_dupes_json(scope: &str, sets: &[DupeSet], out: impl Write) -> std::io::Result<()> {
    let mut w = BufWriter::new(out);
    let wasted: u64 = sets
        .iter()
        .map(|s| s.size * (s.files.len() as u64 - 1))
        .sum();
    let v = serde_json::json!({ "scope": scope, "extraBytes": wasted, "sets": sets });
    serde_json::to_writer_pretty(&mut w, &v)?;
    w.flush()
}

pub fn write_dupes_html(
    scope: &str,
    sets: &[DupeSet],
    theme: &Theme,
    out: impl Write,
) -> std::io::Result<()> {
    let wasted: u64 = sets
        .iter()
        .map(|s| s.size * (s.files.len() as u64 - 1))
        .sum();
    let copies: usize = sets.iter().map(|s| s.files.len()).sum();
    let mut stats = String::new();
    stats += &stat("In extra copies", &human(wasted), "up");
    stats += &stat("Sets", &thousands(sets.len() as u64), "");
    stats += &stat("Files", &thousands(copies as u64), "");
    let mut body = String::new();
    for s in sets {
        let first = &s.files[0].path;
        let name = first.rsplit_once('\\').map_or(first.as_str(), |(_, n)| n);
        let _ = write!(
            body,
            r#"<div class="set"><h3>{name}<small>{n} copies of {size}</small><em class="up num">{extra} extra</em></h3>"#,
            name = esc(name),
            n = s.files.len(),
            size = human(s.size),
            extra = human(s.size * (s.files.len() as u64 - 1)),
        );
        for f in &s.files {
            let date = iso(f.mtime).replace('T', " ");
            let _ = write!(
                body,
                r#"<div><span class="mono" title="{p}">{p}</span><span class="num">{d}</span></div>"#,
                p = esc(&f.path),
                d = date.replace('Z', " UTC"),
            );
        }
        body.push_str("</div>");
    }
    let body = format!(r#"<section class="card"><h2>Duplicate sets</h2>{body}</section>"#);
    let mut w = BufWriter::new(out);
    w.write_all(page("Duplicate files", scope, &stats, &body, theme).as_bytes())?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut t = Tree::new("C:");
        let d = t.add(t.root, "My, \"Docs\"", flags::DIR, 0, 0, 0, 0);
        t.add(d, "a.mp4", 0, 100, 4096, 1_700_000_000, 0);
        t.add(t.root, "b.zip", 0, 50, 4096, 0, 0);
        t.aggregate();
        t
    }

    #[test]
    fn csv_quotes_and_lists_everything() {
        let t = sample();
        let mut out = Vec::new();
        write_csv(&t, t.root, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.lines().count(), 5);
        assert!(s.contains("\"C:\\My, \"\"Docs\"\"\\a.mp4\",file,100,4096,1,2023-11-14T22:13:20Z"));
    }

    #[test]
    fn changes_cover_nested_files() {
        let old = sample();
        let mut new = sample();
        let docs = new.find_path("C:\\My, \"Docs\"").unwrap();
        new.add(docs, "new.pdf", 0, 10, 4096, 0, 0);
        new.aggregate();
        let rows = collect_changes(
            &old,
            &new,
            Some(old.root),
            new.root,
            100,
            diff::Filters::default(),
        );
        let paths: Vec<_> = rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["C:\\My, \"Docs\"", "C:\\My, \"Docs\"\\new.pdf"]);
        assert_eq!(rows[1].status, diff::Status::Added);
        let mut csv = Vec::new();
        write_changes_csv(&rows, &mut csv).unwrap();
        assert!(
            String::from_utf8(csv)
                .unwrap()
                .contains("new.pdf\",file,added,0,4096,4096")
        );
    }

    #[test]
    fn json_and_html_render() {
        let t = sample();
        let mut j = Vec::new();
        write_json(&t, t.root, &mut j).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&j).unwrap();
        assert_eq!(v["root"]["children"][0]["name"], "My, \"Docs\"");
        let mut h = Vec::new();
        write_html(&t, t.root, "C: Local disk", 0, &Theme::default(), &mut h).unwrap();
        let h = String::from_utf8(h).unwrap();
        assert!(h.contains("<svg") && h.contains("Largest files") && h.contains("a.mp4"));
    }

    #[test]
    fn html_uses_the_colour_template() {
        let t = sample();
        let mut theme = Theme::default();
        theme.dark.ui.insert("accent".into(), "#3ecf8e".into());
        theme.dark.ui.insert("app-bg".into(), "#0d1310".into());
        theme.dark.files.insert("video".into(), "#ff7a85".into());
        // Anything that could break out of the style sheet is dropped.
        theme
            .dark
            .ui
            .insert("fg".into(), "red}</style><script>".into());
        let mut h = Vec::new();
        write_html(&t, t.root, "C:", 0, &theme, &mut h).unwrap();
        let h = String::from_utf8(h).unwrap();
        assert!(h.contains("--accent:#3ecf8e;"));
        assert!(h.contains("--bg:var(--app-bg);"));
        assert!(h.contains("--c2:#ff7a85;"));
        assert!(!h.contains("</style><script>"));
    }
}
