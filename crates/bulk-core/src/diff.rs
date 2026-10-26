//! Compare two trees of the same drive, one folder at a time. Children are matched
//! by name (case-insensitively), so the comparison works for any filesystem.

use std::collections::HashMap;

use serde::Serialize;

use crate::filter::Filter;
use crate::tree::{NodeId, Tree};

/// Filters for the old and new tree: only matching files are compared, and folder
/// sizes count only those.
#[derive(Clone, Copy, Default)]
pub struct Filters<'a> {
    pub old: Option<&'a Filter>,
    pub new: Option<&'a Filter>,
}

/// Size on disk and file count of `id`, as the filter sees it.
pub fn totals(tree: &Tree, f: Option<&Filter>, id: NodeId) -> (u64, u32) {
    match f {
        Some(f) => {
            let v = f.get(id);
            (v.alloc, v.files)
        }
        None => (tree.get(id).alloc, tree.get(id).files),
    }
}

fn shown(f: Option<&Filter>, id: NodeId) -> bool {
    f.is_none_or(|f| f.shows(id))
}

fn has_kids(tree: &Tree, f: Option<&Filter>, id: NodeId) -> bool {
    match f {
        Some(f) => tree.children(id).any(|c| f.shows(c)),
        None => tree.has_children(id),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    pub old_id: Option<NodeId>,
    pub new_id: Option<NodeId>,
    pub dir: bool,
    pub old_alloc: u64,
    pub new_alloc: u64,
    pub delta: i64,
    pub old_files: u32,
    pub new_files: u32,
    pub status: Status,
    /// Whether expanding this item can show anything.
    pub has_kids: bool,
}

/// Changed children of a folder present in either tree, biggest change first.
/// Unchanged items are left out.
pub fn children(
    old: &Tree,
    new: &Tree,
    old_id: Option<NodeId>,
    new_id: Option<NodeId>,
    limit: usize,
    f: Filters,
) -> Vec<Item> {
    let mut by_name: HashMap<Vec<u8>, NodeId> = HashMap::new();
    if let Some(o) = old_id {
        for c in old.children(o).filter(|&c| shown(f.old, c)) {
            by_name.insert(old.name_lower(c).to_vec(), c);
        }
    }

    let mut out = Vec::new();
    if let Some(n) = new_id {
        for c in new.children(n) {
            let matched = by_name.remove(new.name_lower(c));
            if !shown(f.new, c) && matched.is_none() {
                continue;
            }
            let (na, nf) = if shown(f.new, c) {
                totals(new, f.new, c)
            } else {
                (0, 0)
            };
            let (oa, of) = matched.map_or((0, 0), |o| totals(old, f.old, o));
            let delta = na as i64 - oa as i64;
            if matched.is_some() && delta == 0 && of == nf {
                continue;
            }
            out.push(Item {
                name: new.name(c).to_string(),
                old_id: matched,
                new_id: Some(c),
                dir: new.get(c).is_dir(),
                old_alloc: oa,
                new_alloc: na,
                delta,
                old_files: of,
                new_files: nf,
                status: if matched.is_none() {
                    Status::Added
                } else if na == 0 && nf == 0 {
                    Status::Removed
                } else {
                    Status::Changed
                },
                has_kids: has_kids(new, f.new, c)
                    || matched.is_some_and(|o| has_kids(old, f.old, o)),
            });
        }
    }
    for (_, o) in by_name {
        let (oa, of) = totals(old, f.old, o);
        out.push(Item {
            name: old.name(o).to_string(),
            old_id: Some(o),
            new_id: None,
            dir: old.get(o).is_dir(),
            old_alloc: oa,
            new_alloc: 0,
            delta: -(oa as i64),
            old_files: of,
            new_files: 0,
            status: Status::Removed,
            has_kids: has_kids(old, f.old, o),
        });
    }
    out.sort_by(|a, b| {
        b.delta
            .unsigned_abs()
            .cmp(&a.delta.unsigned_abs())
            .then_with(|| a.name.cmp(&b.name))
    });
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::flags;

    #[test]
    fn finds_added_removed_and_changed() {
        let mut old = Tree::new("C:");
        let d = old.add(old.root, "Docs", flags::DIR, 0, 0, 0, 0);
        old.add(d, "keep.txt", 0, 10, 10, 0, 0);
        old.add(old.root, "gone.bin", 0, 50, 50, 0, 0);
        old.add(old.root, "same.bin", 0, 5, 5, 0, 0);
        old.aggregate();

        let mut new = Tree::new("C:");
        let d2 = new.add(new.root, "docs", flags::DIR, 0, 0, 0, 0);
        new.add(d2, "keep.txt", 0, 30, 30, 0, 0);
        new.add(new.root, "new.iso", 0, 70, 70, 0, 0);
        new.add(new.root, "same.bin", 0, 5, 5, 0, 0);
        new.aggregate();

        let items = children(
            &old,
            &new,
            Some(old.root),
            Some(new.root),
            100,
            Filters::default(),
        );
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name, "new.iso");
        assert_eq!(items[0].status, Status::Added);
        assert_eq!(items[1].status, Status::Removed);
        assert_eq!(items[1].delta, -50);
        assert_eq!(items[2].name, "docs");
        assert_eq!(items[2].delta, 20);

        let inner = children(
            &old,
            &new,
            items[2].old_id,
            items[2].new_id,
            100,
            Filters::default(),
        );
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].delta, 20);

        // Only .iso files: just the new one shows up.
        let (fo, fnew) = (
            Filter::new(&old, "ext:iso", 0),
            Filter::new(&new, "ext:iso", 0),
        );
        let f = Filters {
            old: Some(&fo),
            new: Some(&fnew),
        };
        let items = children(&old, &new, Some(old.root), Some(new.root), 100, f);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "new.iso");
    }
}
