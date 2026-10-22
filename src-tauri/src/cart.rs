//! Cleanup cart: collect items across drives, review them, then recycle or delete
//! them in one go, with undo for recycled items.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use bulk_core::trash;
use bulk_core::tree::NodeId;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

type Res<T> = Result<T, String>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CartItem {
    letter: char,
    id: NodeId,
    path: String,
    name: String,
    dir: bool,
    alloc: u64,
    files: u32,
}

#[derive(Default)]
pub struct CartState {
    items: Mutex<Vec<CartItem>>,
    /// Recycled batches that can still be undone, by id.
    undo: Mutex<HashMap<u32, Vec<(char, PathBuf, SystemTime)>>>,
    next_undo: AtomicU32,
}

fn lower(p: &str) -> String {
    p.to_ascii_lowercase()
}

/// Whether `child` is inside `parent` (or is it).
fn within(parent: &str, child: &str) -> bool {
    let (p, c) = (lower(parent), lower(child));
    c == p || c.starts_with(&format!("{}\\", p.trim_end_matches('\\')))
}

impl CartState {
    /// Nodes of `tree` (drive `letter`) that are in the cart, found by path so they
    /// survive a rescan.
    pub fn nodes(&self, letter: char, tree: &bulk_core::tree::Tree) -> HashSet<NodeId> {
        let items = self.items.lock().unwrap();
        items
            .iter()
            .filter(|c| c.letter == letter)
            .filter_map(|c| tree.find_path(&c.path))
            .collect()
    }
}

#[tauri::command]
pub fn cart_get(cart: State<CartState>) -> Vec<CartItem> {
    cart.items.lock().unwrap().clone()
}

/// Add nodes to the cart. Items already covered by a folder in the cart are
/// skipped, and adding a folder drops anything inside it.
#[tauri::command]
pub fn cart_add(
    app: AppHandle,
    state: State<AppState>,
    cart: State<CartState>,
    letter: char,
    ids: Vec<NodeId>,
) -> Res<Vec<CartItem>> {
    let new = items_for(&state, letter, ids)?;
    let mut items = cart.items.lock().unwrap();
    for it in new {
        if items.iter().any(|x| within(&x.path, &it.path)) {
            continue;
        }
        items.retain(|x| !within(&it.path, &x.path));
        items.push(it);
    }
    let out = items.clone();
    drop(items);
    let _ = app.emit("cart-changed", out.len());
    Ok(out)
}

/// Cart entries for nodes of drive `letter` (the root and gone items are skipped).
fn items_for(state: &AppState, letter: char, ids: Vec<NodeId>) -> Res<Vec<CartItem>> {
    let drives = state.drives.lock().unwrap();
    let tree = drives
        .get(&letter)
        .and_then(|d| d.tree.as_ref())
        .ok_or("not scanned")?;
    Ok(ids
        .into_iter()
        .filter(|&id| (id as usize) < tree.len() && id != tree.root)
        .filter(|&id| !tree.get(id).has(bulk_core::tree::flags::GONE))
        .map(|id| {
            let n = tree.get(id);
            CartItem {
                letter,
                id,
                path: tree.path(id),
                name: tree.name(id).to_string(),
                dir: n.is_dir(),
                alloc: n.alloc,
                files: n.files,
            }
        })
        .collect())
}

/// Delete nodes right away (Shift+Del or the right-click menu), without going
/// through the cart. Same result and undo as emptying the cart.
#[tauri::command]
pub async fn delete_nodes(
    app: AppHandle,
    letter: char,
    ids: Vec<NodeId>,
    permanent: bool,
) -> Res<DeleteResult> {
    let items = items_for(&app.state::<AppState>(), letter, ids)?;
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || delete_items(&a, items, permanent))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cart_remove(app: AppHandle, cart: State<CartState>, paths: Vec<String>) -> Vec<CartItem> {
    let mut items = cart.items.lock().unwrap();
    items.retain(|x| !paths.iter().any(|p| lower(p) == lower(&x.path)));
    let out = items.clone();
    let _ = app.emit("cart-changed", out.len());
    out
}

#[tauri::command]
pub fn cart_clear(app: AppHandle, cart: State<CartState>) {
    cart.items.lock().unwrap().clear();
    let _ = app.emit("cart-changed", 0);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    path: String,
    error: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    freed: u64,
    deleted: usize,
    failed: Vec<Failure>,
    /// Set when the items went to the Recycle Bin and can be put back.
    undo: Option<u32>,
}

/// Empty the cart into the Recycle Bin, or permanently when `permanent`.
#[tauri::command]
pub async fn cart_delete(app: AppHandle, permanent: bool) -> Res<DeleteResult> {
    let items = app.state::<CartState>().items.lock().unwrap().clone();
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || delete_items(&a, items, permanent))
        .await
        .map_err(|e| e.to_string())
}

fn delete_items(app: &AppHandle, items: Vec<CartItem>, permanent: bool) -> DeleteResult {
    let cart = app.state::<CartState>();
    let mut res = DeleteResult {
        freed: 0,
        deleted: 0,
        failed: Vec::new(),
        undo: None,
    };
    let mut undo = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let _ = app.emit("cart-progress", (i, items.len()));
        // Allow for clock skew between us and the Recycle Bin's timestamp.
        let started = SystemTime::now() - Duration::from_secs(2);
        let path = PathBuf::from(&it.path);
        let r = if permanent {
            trash::delete_permanently(&path)
        } else {
            trash::recycle(&path)
        };
        match r {
            Ok(()) => {
                res.freed += it.alloc;
                res.deleted += 1;
                if !permanent {
                    undo.push((it.letter, path, started));
                }
                remove_from_tree(app, it);
                cart.items
                    .lock()
                    .unwrap()
                    .retain(|x| lower(&x.path) != lower(&it.path));
            }
            Err(e) => res.failed.push(Failure {
                path: it.path.clone(),
                error: format!("{e:#}"),
            }),
        }
    }
    if !undo.is_empty() {
        let id = cart.next_undo.fetch_add(1, Ordering::Relaxed);
        cart.undo.lock().unwrap().insert(id, undo);
        res.undo = Some(id);
    }
    let _ = app.emit("cart-changed", cart.items.lock().unwrap().len());
    res
}

/// Mark a deleted item gone in the in-memory tree right away (greyed out until the
/// next scan), so the UI updates even on drives that aren't kept live.
fn remove_from_tree(app: &AppHandle, it: &CartItem) {
    let state = app.state::<AppState>();
    let mut drives = state.drives.lock().unwrap();
    let Some(d) = drives.get_mut(&it.letter) else {
        return;
    };
    let Some(tree) = d.tree.as_mut() else { return };
    // By path: ids from when it was added don't survive a rescan.
    if let Some(id) = tree.find_path(&it.path) {
        tree.mark_gone(id);
        d.view.rebuild(tree);
        d.dirty = true;
    }
    drop(drives);
    let _ = app.emit("drive-updated", it.letter);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    restored: usize,
    failed: Vec<Failure>,
    /// Drives whose view won't pick the restored items up by itself.
    needs_rescan: Vec<char>,
}

#[tauri::command]
pub async fn cart_undo(app: AppHandle, id: u32) -> Res<UndoResult> {
    let batch = app
        .state::<CartState>()
        .undo
        .lock()
        .unwrap()
        .remove(&id)
        .ok_or("nothing to undo")?;
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut res = UndoResult {
            restored: 0,
            failed: Vec::new(),
            needs_rescan: Vec::new(),
        };
        for (letter, path, since) in batch {
            let found = trash::find_recycled(&path, since);
            let outcome = match found.iter().max_by_key(|r| r.deleted) {
                Some(item) => trash::restore(item),
                None => Err(anyhow::anyhow!("not found in the Recycle Bin")),
            };
            match outcome {
                Ok(()) => {
                    res.restored += 1;
                    let state = a.state::<AppState>();
                    let live = state
                        .drives
                        .lock()
                        .unwrap()
                        .get(&letter)
                        .is_some_and(|d| d.watch.is_some());
                    if !live && !res.needs_rescan.contains(&letter) {
                        res.needs_rescan.push(letter);
                    }
                }
                Err(e) => res.failed.push(Failure {
                    path: path.display().to_string(),
                    error: format!("{e:#}"),
                }),
            }
        }
        res
    })
    .await
    .map_err(|e| e.to_string())
}
