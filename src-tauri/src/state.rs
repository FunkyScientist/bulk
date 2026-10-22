//! Per-drive state: the scanned tree plus the tree-list view built over it.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bulk_core::scan::Progress;
use bulk_core::snapshot::Meta;
use bulk_core::tree::{NodeId, SortKey, Tree, flags};
use bulk_core::usn::{FrnIndex, Journal};
use bulk_core::volume::Volume;
use serde::Serialize;

#[derive(Default)]
pub struct AppState {
    pub drives: Mutex<HashMap<char, Drive>>,
    /// Set once saved snapshots have been loaded at startup.
    pub restored: AtomicBool,
}

pub struct Drive {
    pub vol: Volume,
    pub tree: Option<Tree>,
    pub scanned_at: Option<u64>,
    pub method: Option<&'static str>,
    pub scan: Option<Arc<Progress>>,
    pub view: View,
    /// Journal position the tree is current to.
    pub journal: Option<Journal>,
    /// Built on first use for applying journal changes.
    pub index: Option<FrnIndex>,
    /// Changed since `current.bulk` was last written.
    pub dirty: bool,
    /// Set while a watcher keeps this tree live; store `true` to stop it.
    pub watch: Option<Arc<AtomicBool>>,
}

impl Drive {
    pub fn new(vol: Volume) -> Drive {
        Drive {
            vol,
            tree: None,
            scanned_at: None,
            method: None,
            scan: None,
            view: View::default(),
            journal: None,
            index: None,
            dirty: false,
            watch: None,
        }
    }

    pub fn stop_watch(&mut self) {
        if let Some(stop) = self.watch.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    pub fn meta(&self) -> Option<Meta> {
        let tree = self.tree.as_ref()?;
        let root = tree.get(tree.root);
        Some(Meta {
            letter: self.vol.letter,
            serial: self.vol.serial,
            label: self.vol.label.clone(),
            taken: self.scanned_at.unwrap_or(0),
            method: self.method.unwrap_or("walk").to_string(),
            files: root.files,
            alloc: root.alloc,
            size: root.size,
            journal: self.journal,
        })
    }
}

/// The flattened, sorted list of rows the tree list shows. Rebuilt whenever a
/// folder is expanded or collapsed or the sort changes.
pub struct View {
    pub expanded: HashSet<NodeId>,
    pub sort: SortKey,
    pub rows: Vec<(NodeId, u16)>,
}

impl Default for View {
    fn default() -> Self {
        View {
            expanded: HashSet::new(),
            sort: SortKey::Alloc,
            rows: Vec::new(),
        }
    }
}

impl View {
    pub fn reset(&mut self, tree: &Tree) {
        self.expanded.clear();
        self.expanded.insert(tree.root);
        self.rebuild(tree);
    }

    pub fn rebuild(&mut self, tree: &Tree) {
        self.rows.clear();
        let mut stack: Vec<(NodeId, u16)> = vec![(tree.root, 0)];
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if self.expanded.contains(&id) {
                let kids = tree.sorted_children(id, self.sort);
                for &k in kids.iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
        }
    }

    pub fn index_of(&self, id: NodeId) -> Option<usize> {
        self.rows.iter().position(|&(r, _)| r == id)
    }

    /// Expand every ancestor of `id` so it becomes visible.
    pub fn reveal(&mut self, tree: &Tree, id: NodeId) -> Option<usize> {
        let mut changed = false;
        for a in tree.ancestors(id) {
            changed |= self.expanded.insert(a);
        }
        if changed {
            self.rebuild(tree);
        }
        self.index_of(id)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: NodeId,
    pub depth: u16,
    pub name: String,
    pub dir: bool,
    pub flags: u16,
    pub size: u64,
    pub alloc: u64,
    pub files: u32,
    pub mtime: u32,
    pub has_kids: bool,
    pub open: bool,
    /// Share of the parent's size on disk, 0..1.
    pub share: f32,
}

pub fn row(tree: &Tree, id: NodeId, depth: u16, open: bool) -> Row {
    let n = tree.get(id);
    let parent_alloc = if n.parent == bulk_core::tree::NONE {
        n.alloc
    } else {
        tree.get(n.parent).alloc
    };
    Row {
        id,
        depth,
        name: tree.name(id).to_string(),
        dir: n.is_dir(),
        flags: n.flags & !flags::DELETED,
        size: n.size,
        alloc: n.alloc,
        files: n.files,
        mtime: n.mtime,
        has_kids: tree.has_children(id),
        open,
        share: if parent_alloc == 0 {
            0.0
        } else {
            (n.alloc as f64 / parent_alloc as f64) as f32
        },
    }
}
