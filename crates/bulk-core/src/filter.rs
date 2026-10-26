//! A search query applied to the whole tree: every folder's size, counting only the
//! files that match. The list and charts show these sizes instead of the real ones.

use rayon::prelude::*;

use crate::search::{self, Query};
use crate::tree::{NodeId, Tree, flags};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Val {
    pub size: u64,
    pub alloc: u64,
    pub files: u32,
}

pub struct Filter {
    pub query: String,
    vals: Vec<Val>,
    /// Matches found (folders that matched count once, however much they hold).
    pub matches: usize,
    /// Cheap fingerprint of the tree the sizes were computed from.
    stamp: (usize, u64, u64, u32),
}

fn stamp(tree: &Tree) -> (usize, u64, u64, u32) {
    let r = tree.get(tree.root);
    (tree.len(), r.alloc, r.size, r.files)
}

impl Filter {
    pub fn new(tree: &Tree, query: &str, now: u64) -> Filter {
        let q = search::parse(query, now);
        let mut f = Filter {
            query: query.to_string(),
            vals: Vec::new(),
            matches: 0,
            stamp: (0, 0, 0, 0),
        };
        f.compute(tree, &q);
        f
    }

    /// Recompute if the tree changed since the last time (live updates, rescans).
    pub fn refresh(&mut self, tree: &Tree, now: u64) {
        if self.stamp != stamp(tree) {
            let q = search::parse(&self.query, now);
            self.compute(tree, &q);
        }
    }

    fn compute(&mut self, tree: &Tree, q: &Query) {
        let n = tree.len();
        let hits = search::matching(tree, q);
        let mut hit = vec![false; n];
        for &id in &hits {
            hit[id as usize] = true;
        }
        let mut vals = vec![Val::default(); n];
        let mut matches = 0;
        // A match inside a matching folder is already counted with the folder.
        let tops: Vec<NodeId> = hits
            .into_par_iter()
            .filter(|&id| !tree.ancestors(id).any(|a| hit[a as usize]))
            .collect();
        for id in tops {
            matches += 1;
            let node = tree.get(id);
            let own = Val {
                size: node.size,
                alloc: node.alloc,
                files: if node.is_dir() { node.files } else { 1 },
            };
            if node.is_dir() {
                // Everything inside a matching folder counts in full.
                let mut stack = vec![id];
                while let Some(d) = stack.pop() {
                    for c in tree.children(d) {
                        let cn = tree.get(c);
                        vals[c as usize] = Val {
                            size: cn.size,
                            alloc: cn.alloc,
                            files: if cn.is_dir() { cn.files } else { 1 },
                        };
                        stack.push(c);
                    }
                }
            }
            vals[id as usize] = own;
            for a in tree.ancestors(id) {
                let v = &mut vals[a as usize];
                v.size += own.size;
                v.alloc += own.alloc;
                v.files += own.files;
            }
        }
        self.vals = vals;
        self.matches = matches;
        self.stamp = stamp(tree);
    }

    pub fn get(&self, id: NodeId) -> Val {
        self.vals.get(id as usize).copied().unwrap_or_default()
    }

    /// Whether `id` has anything that matches (or is a match).
    pub fn shows(&self, id: NodeId) -> bool {
        let v = self.get(id);
        v.files > 0 || v.alloc > 0
    }

    /// A copy of `tree` holding only what matches, for exports.
    pub fn to_tree(&self, tree: &Tree) -> Tree {
        let mut out = Tree::new(tree.name(tree.root));
        let mut stack = vec![(tree.root, out.root)];
        while let Some((from, to)) = stack.pop() {
            for c in tree.children(from) {
                if !self.shows(c) {
                    continue;
                }
                let n = tree.get(c);
                let f = n.flags & !(flags::NEW | flags::GONE);
                if n.is_dir() {
                    let id = out.add(to, tree.name(c), f, 0, 0, n.mtime, n.frn);
                    stack.push((c, id));
                } else {
                    out.add(to, tree.name(c), f, n.size, n.alloc, n.mtime, n.frn);
                }
            }
        }
        out.aggregate();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_count_only_matches() {
        let mut t = Tree::new("C:");
        let v = t.add(t.root, "Videos", flags::DIR, 0, 0, 0, 0);
        let sub = t.add(v, "Trips", flags::DIR, 0, 0, 0, 0);
        t.add(sub, "a.mp4", 0, 100, 100, 0, 0);
        t.add(v, "notes.txt", 0, 10, 10, 0, 0);
        t.add(t.root, "b.mkv", 0, 50, 50, 0, 0);
        t.aggregate();

        let f = Filter::new(&t, "kind:video", 0);
        assert_eq!(f.get(t.root).alloc, 150);
        assert_eq!(f.get(v).alloc, 100);
        assert_eq!(f.get(v).files, 1);
        let notes = t.find_path(r"C:\Videos\notes.txt").unwrap();
        assert!(!f.shows(notes));
        assert_eq!(f.matches, 2);

        // A matching folder counts everything inside it, once.
        let f = Filter::new(&t, "trips OR a.mp4", 0);
        assert_eq!(f.get(t.root).alloc, 100);
        assert_eq!(f.matches, 1);

        let f = Filter::new(&t, "kind:video", 0);
        let copy = f.to_tree(&t);
        assert_eq!(copy.get(copy.root).alloc, 150);
        assert_eq!(copy.get(copy.root).files, 2);
        assert!(copy.find_path(r"C:\Videos\notes.txt").is_none());
        assert!(copy.find_path(r"C:\Videos\Trips\a.mp4").is_some());
    }
}
