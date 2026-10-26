//! Per-drive state: the scanned tree plus the tree-list view built over it.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bulk_core::filter::Filter;
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
    /// Older snapshot loaded for the "what changed" view.
    pub diff: Option<(Meta, Tree)>,
    /// The Browse filter applied to `diff`'s snapshot.
    pub diff_filter: Option<Filter>,
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
            diff: None,
            diff_filter: None,
        }
    }

    /// Bring `diff_filter` in line with the Browse filter.
    pub fn sync_diff_filter(&mut self) {
        let want = self.view.filter.as_ref().map(|f| f.query.clone());
        let have = self.diff_filter.as_ref().map(|f| f.query.clone());
        if want == have {
            return;
        }
        self.diff_filter = match (want, &self.diff) {
            (Some(q), Some((_, old))) => Some(Filter::new(old, &q, crate::scan::now())),
            _ => None,
        };
    }

    /// Filters for comparing against the loaded snapshot.
    pub fn diff_filters(&self) -> bulk_core::diff::Filters<'_> {
        bulk_core::diff::Filters {
            old: self.diff_filter.as_ref(),
            new: self.view.filter.as_ref(),
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
    /// Only show what matches this search, with folder sizes to match.
    pub filter: Option<Filter>,
}

impl Default for View {
    fn default() -> Self {
        View {
            expanded: HashSet::new(),
            sort: SortKey::Alloc,
            rows: Vec::new(),
            filter: None,
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
        if let Some(f) = &mut self.filter {
            f.refresh(tree, crate::scan::now());
        }
        self.rows.clear();
        let mut stack: Vec<(NodeId, u16)> = vec![(tree.root, 0)];
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if self.expanded.contains(&id) {
                let mut kids = tree.sorted_children_all(id, self.sort);
                if let Some(f) = &self.filter {
                    kids.retain(|&k| f.shows(k));
                    let by = |k: NodeId| {
                        let v = f.get(k);
                        match self.sort {
                            SortKey::Size => v.size,
                            SortKey::Alloc => v.alloc,
                            SortKey::Files => v.files as u64,
                            _ => 0,
                        }
                    };
                    if matches!(self.sort, SortKey::Size | SortKey::Alloc | SortKey::Files) {
                        kids.sort_by(|&a, &b| by(b).cmp(&by(a)).then(a.cmp(&b)));
                    }
                }
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
        // A stale id from another drive, or from before a rescan.
        if id as usize >= tree.len() {
            return None;
        }
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

pub fn row(tree: &Tree, id: NodeId, depth: u16, open: bool, filter: Option<&Filter>) -> Row {
    let n = tree.get(id);
    let (size, alloc, files) = match filter {
        Some(f) => {
            let v = f.get(id);
            (v.size, v.alloc, v.files)
        }
        None => (n.size, n.alloc, n.files),
    };
    let parent_alloc = if n.parent == bulk_core::tree::NONE {
        alloc
    } else {
        match filter {
            Some(f) => f.get(n.parent).alloc,
            None => tree.get(n.parent).alloc,
        }
    };
    Row {
        id,
        depth,
        name: tree.name(id).to_string(),
        dir: n.is_dir(),
        flags: n.flags & !flags::DELETED,
        size,
        alloc,
        files,
        mtime: n.mtime,
        has_kids: match filter {
            Some(f) => tree.children(id).any(|c| f.shows(c)),
            None => tree.has_children(id),
        },
        open,
        share: if parent_alloc == 0 {
            0.0
        } else {
            (alloc as f64 / parent_alloc as f64) as f32
        },
    }
}
