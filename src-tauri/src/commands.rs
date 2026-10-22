use std::os::windows::process::CommandExt;

use bulk_core::helper;
use bulk_core::tree::{NodeId, SortKey, Tree};
use bulk_core::volume::{DriveKind, Volume};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

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
    /// Kept current from the change journal.
    live: bool,
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
                live: d.watch.is_some(),
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
    let ok =
        tauri::async_runtime::spawn_blocking(|| helper::install_elevated(&helper::helper_exe()))
            .await
            .map_err(err)?
            .map_err(err)?;
    if !ok {
        return Err("cancelled".into());
    }
    let status = tauri::async_runtime::spawn_blocking(|| {
        let c = helper::Client::new().ok()?;
        c.ensure_running(std::time::Duration::from_secs(6))
            .then_some(())
    })
    .await
    .map_err(err)?
    .map_or(HelperStatus::Outdated, |_| HelperStatus::Ready);
    scan::set_helper_status(&app, status);
    Ok(status)
}

/// Whether startup snapshot loading has finished.
#[tauri::command]
pub fn restored(state: State<AppState>) -> bool {
    state.restored.load(std::sync::atomic::Ordering::Relaxed)
}

#[tauri::command]
pub fn scan(app: AppHandle, letter: char, on_event: Channel<ScanEvent>) -> Res<()> {
    scan::start(app, letter, on_event)
}

#[tauri::command]
pub fn cancel_scan(state: State<AppState>, letter: char) {
    if let Some(p) = state
        .drives
        .lock()
        .unwrap()
        .get(&letter)
        .and_then(|d| d.scan.clone())
    {
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
    let Some(tree) = &d.tree else {
        return Ok(RowsPage {
            total: 0,
            rows: vec![],
        });
    };
    let v = &d.view;
    let end = (start + count.min(500)).min(v.rows.len());
    let rows = v.rows[start.min(end)..end]
        .iter()
        .map(|&(id, depth)| row(tree, id, depth, v.expanded.contains(&id)))
        .collect();
    Ok(RowsPage {
        total: v.rows.len(),
        rows,
    })
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
    let tree = drives
        .get(&letter)
        .and_then(|d| d.tree.as_ref())
        .ok_or("not scanned")?;
    if id as usize >= tree.len() {
        return Err("bad node".into());
    }
    Ok(tree.path(id))
}

#[derive(Serialize)]
pub struct Crumb {
    id: NodeId,
    name: String,
}

/// `id` and its ancestors, root first.
#[tauri::command]
pub fn ancestors(state: State<AppState>, letter: char, id: NodeId) -> Res<Vec<Crumb>> {
    let drives = state.drives.lock().unwrap();
    let tree = drives
        .get(&letter)
        .and_then(|d| d.tree.as_ref())
        .ok_or("not scanned")?;
    if id as usize >= tree.len() {
        return Err("bad node".into());
    }
    let mut chain: Vec<NodeId> = vec![id];
    chain.extend(tree.ancestors(id));
    Ok(chain
        .into_iter()
        .rev()
        .map(|id| Crumb {
            id,
            name: tree.name(id).to_string(),
        })
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    id: NodeId,
    name: String,
    path: String,
    dir: bool,
    flags: u16,
    size: u64,
    alloc: u64,
    files: u32,
    mtime: u32,
    parent: Option<NodeId>,
}

#[tauri::command]
pub fn node_info(state: State<AppState>, letter: char, id: NodeId) -> Res<NodeInfo> {
    let drives = state.drives.lock().unwrap();
    let tree = drives
        .get(&letter)
        .and_then(|d| d.tree.as_ref())
        .ok_or("not scanned")?;
    if id as usize >= tree.len() {
        return Err("bad node".into());
    }
    let n = tree.get(id);
    Ok(NodeInfo {
        id,
        name: tree.name(id).to_string(),
        path: tree.path(id),
        dir: n.is_dir(),
        flags: n.flags,
        size: n.size,
        alloc: n.alloc,
        files: n.files,
        mtime: n.mtime,
        parent: (n.parent != bulk_core::tree::NONE).then_some(n.parent),
    })
}

/// Treemap tiles for `root`, packed as binary (see `encode_shapes`).
#[tauri::command]
pub fn treemap(
    state: State<AppState>,
    letter: char,
    root: Option<NodeId>,
    width: f32,
    height: f32,
) -> Res<tauri::ipc::Response> {
    use bulk_core::layout::{TreemapOptions, tile_flags, treemap};
    let drives = state.drives.lock().unwrap();
    let d = drives.get(&letter).ok_or("unknown drive")?;
    let tree = d.tree.as_ref().ok_or("not scanned")?;
    let root = root
        .filter(|&r| (r as usize) < tree.len())
        .unwrap_or(tree.root);
    let opts = TreemapOptions {
        width: width.clamp(1.0, 16384.0),
        height: height.clamp(1.0, 16384.0),
        use_alloc: d.view.sort != SortKey::Size,
        ..Default::default()
    };
    let tiles = treemap(tree, root, &opts);
    let shapes = tiles.iter().map(|t| Shape {
        geom: [t.rect.x, t.rect.y, t.rect.w, t.rect.h],
        id: t.id,
        depth: t.depth,
        flags: t.flags,
        category: t.category,
        // Names only for tiles big enough to show text.
        label: t.flags & tile_flags::HEADER != 0 || (t.rect.w >= 44.0 && t.rect.h >= 16.0),
    });
    Ok(tauri::ipc::Response::new(encode_shapes(
        tree,
        shapes,
        opts.use_alloc,
    )))
}

/// Sunburst arcs for `root`, in the same binary layout as `treemap` with
/// `x, y` = start and end angle.
#[tauri::command]
pub fn sunburst(
    state: State<AppState>,
    letter: char,
    root: Option<NodeId>,
    depth: u8,
) -> Res<tauri::ipc::Response> {
    let drives = state.drives.lock().unwrap();
    let d = drives.get(&letter).ok_or("unknown drive")?;
    let tree = d.tree.as_ref().ok_or("not scanned")?;
    let root = root
        .filter(|&r| (r as usize) < tree.len())
        .unwrap_or(tree.root);
    let use_alloc = d.view.sort != SortKey::Size;
    let arcs =
        bulk_core::layout::sunburst(tree, root, depth.clamp(1, 12), 0.0025, 20_000, use_alloc);
    let shapes = arcs.iter().map(|a| Shape {
        geom: [a.a0, a.a1, 0.0, 0.0],
        id: a.id,
        depth: a.depth,
        flags: a.flags,
        category: a.category,
        label: a.a1 - a.a0 > 0.05,
    });
    Ok(tauri::ipc::Response::new(encode_shapes(
        tree, shapes, use_alloc,
    )))
}

struct Shape {
    geom: [f32; 4],
    id: NodeId,
    depth: u8,
    flags: u8,
    category: u8,
    label: bool,
}

/// `u32 count`, then per shape `f32 ×4; u32 id; u8 depth, flags, category, 0; u64 value`,
/// then `u32 labels` and per label `u32 id; u16 len; utf-8 bytes`.
fn encode_shapes(tree: &Tree, shapes: impl Iterator<Item = Shape>, use_alloc: bool) -> Vec<u8> {
    let mut body = Vec::new();
    let mut labels = Vec::new();
    let mut count = 0u32;
    let mut label_count = 0u32;
    for s in shapes {
        count += 1;
        for v in s.geom {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&s.id.to_le_bytes());
        body.extend_from_slice(&[s.depth, s.flags, s.category, 0]);
        let n = tree.get(s.id);
        let value = if use_alloc { n.alloc } else { n.size };
        body.extend_from_slice(&value.to_le_bytes());
        if s.label {
            label_count += 1;
            let name = tree.name(s.id);
            let mut end = name.len().min(200);
            while !name.is_char_boundary(end) {
                end -= 1;
            }
            labels.extend_from_slice(&s.id.to_le_bytes());
            labels.extend_from_slice(&(end as u16).to_le_bytes());
            labels.extend_from_slice(&name.as_bytes()[..end]);
        }
    }
    let mut out = Vec::with_capacity(8 + body.len() + labels.len());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(&label_count.to_le_bytes());
    out.extend_from_slice(&labels);
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    letter: char,
    id: NodeId,
    name: String,
    path: String,
    dir: bool,
    alloc: u64,
    mtime: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    total: usize,
    elapsed_ms: f64,
    hits: Vec<SearchHit>,
}

/// Search every scanned drive (or just `letter`). See `bulk_core::search` for the
/// query syntax.
#[tauri::command]
pub async fn search(
    state: State<'_, AppState>,
    query: String,
    letter: Option<char>,
) -> Res<SearchResult> {
    let t0 = std::time::Instant::now();
    let q = bulk_core::search::parse(&query, scan::now());
    let drives = state.drives.lock().unwrap();
    let mut all: Vec<(u8, u64, SearchHit)> = Vec::new();
    let mut total = 0;
    for (l, d) in drives.iter() {
        if letter.is_some_and(|x| x != *l) {
            continue;
        }
        let Some(tree) = &d.tree else { continue };
        let (hits, n) = bulk_core::search::search(tree, &q, 120);
        total += n;
        for h in hits {
            let node = tree.get(h.id);
            all.push((
                h.score,
                node.alloc,
                SearchHit {
                    letter: *l,
                    id: h.id,
                    name: tree.name(h.id).to_string(),
                    path: tree.path(h.id),
                    dir: node.is_dir(),
                    alloc: node.alloc,
                    mtime: node.mtime,
                },
            ));
        }
    }
    all.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    all.truncate(120);
    Ok(SearchResult {
        total,
        elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0,
        hits: all.into_iter().map(|(_, _, h)| h).collect(),
    })
}

/// Open a file with its default app.
#[tauri::command]
pub fn open_path(path: String) -> Res<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let to_w = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (to_w("open"), to_w(&path));
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
    // Values above 32 mean success.
    if r as usize <= 32 {
        return Err(format!("couldn't open {path}"));
    }
    Ok(())
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
    if crate::win::has_mica() {
        "mica"
    } else {
        "solid"
    }
}
