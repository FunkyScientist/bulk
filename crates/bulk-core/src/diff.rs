//! Compare two trees of the same drive, one folder at a time. Children are matched
//! by name (case-insensitively), so the comparison works for any filesystem.

use std::collections::HashMap;

use serde::Serialize;

use crate::tree::{NodeId, Tree};

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
) -> Vec<Item> {
    let mut by_name: HashMap<Vec<u8>, NodeId> = HashMap::new();
    if let Some(o) = old_id {
        for c in old.children(o) {
            by_name.insert(old.name_lower(c).to_vec(), c);
        }
    }

    let mut out = Vec::new();
    if let Some(n) = new_id {
        for c in new.children(n) {
            let nn = new.get(c);
            let matched = by_name.remove(new.name_lower(c));
            let (oa, of) = matched.map_or((0, 0), |o| (old.get(o).alloc, old.get(o).files));
            let delta = nn.alloc as i64 - oa as i64;
            let files_changed = matched.is_some_and(|o| old.get(o).files != nn.files);
            if matched.is_some() && delta == 0 && !files_changed {
                continue;
            }
            out.push(Item {
                name: new.name(c).to_string(),
                old_id: matched,
                new_id: Some(c),
                dir: nn.is_dir(),
                old_alloc: oa,
                new_alloc: nn.alloc,
                delta,
                old_files: of,
                new_files: nn.files,
                status: if matched.is_some() {
                    Status::Changed
                } else {
                    Status::Added
                },
                has_kids: new.has_children(c) || matched.is_some_and(|o| old.has_children(o)),
            });
        }
    }
    for (_, o) in by_name {
        let on = old.get(o);
        out.push(Item {
            name: old.name(o).to_string(),
            old_id: Some(o),
            new_id: None,
            dir: on.is_dir(),
            old_alloc: on.alloc,
            new_alloc: 0,
            delta: -(on.alloc as i64),
            old_files: on.files,
            new_files: 0,
            status: Status::Removed,
            has_kids: old.has_children(o),
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

        let items = children(&old, &new, Some(old.root), Some(new.root), 100);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name, "new.iso");
        assert_eq!(items[0].status, Status::Added);
        assert_eq!(items[1].status, Status::Removed);
        assert_eq!(items[1].delta, -50);
        assert_eq!(items[2].name, "docs");
        assert_eq!(items[2].delta, 20);

        let inner = children(&old, &new, items[2].old_id, items[2].new_id, 100);
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].delta, 20);
    }
}
