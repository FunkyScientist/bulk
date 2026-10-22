//! Arena-backed file tree. Every file and folder is a fixed-size `Node`; names live in
//! one shared byte buffer. Children form an intrusive singly linked list so nodes can be
//! added and removed cheaply when the change journal reports updates.

use std::collections::HashMap;

pub type NodeId = u32;
pub const NONE: NodeId = u32::MAX;

pub mod flags {
    pub const DIR: u16 = 1 << 0;
    pub const HIDDEN: u16 = 1 << 1;
    pub const SYSTEM: u16 = 1 << 2;
    pub const REPARSE: u16 = 1 << 3;
    pub const COMPRESSED: u16 = 1 << 4;
    pub const SPARSE: u16 = 1 << 5;
    /// Additional hard link to a file already counted elsewhere. Shown, but not
    /// added to its parent's totals.
    pub const EXTRA_LINK: u16 = 1 << 6;
    pub const DELETED: u16 = 1 << 7;
    /// Cloud placeholder / offline file whose data isn't stored locally.
    pub const OFFLINE: u16 = 1 << 8;
    /// NTFS metadata file such as `$MFT`.
    pub const META: u16 = 1 << 9;
    /// Deleted while the app watched. Kept in place (greyed out in the UI) until the
    /// next scan, but no longer counted in any totals and skipped by `children`.
    pub const GONE: u16 = 1 << 10;
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Node {
    pub parent: NodeId,
    pub first_child: NodeId,
    pub next_sibling: NodeId,
    pub name_off: u32,
    pub name_len: u16,
    pub flags: u16,
    /// Modification time, unix seconds. For folders, the newest time below.
    pub mtime: u32,
    /// Logical size in bytes. For folders, the sum of everything below.
    pub size: u64,
    /// Size on disk in bytes.
    pub alloc: u64,
    /// Number of files at or below this node.
    pub files: u32,
    pub _pad: u32,
    /// NTFS file reference number (record index + sequence), or 0 if unknown.
    pub frn: u64,
}

impl Node {
    #[inline]
    pub fn is_dir(&self) -> bool {
        self.flags & flags::DIR != 0
    }
    #[inline]
    pub fn has(&self, f: u16) -> bool {
        self.flags & f != 0
    }
}

impl Default for Node {
    fn default() -> Self {
        Node {
            parent: NONE,
            first_child: NONE,
            next_sibling: NONE,
            name_off: 0,
            name_len: 0,
            flags: 0,
            mtime: 0,
            size: 0,
            alloc: 0,
            files: 0,
            _pad: 0,
            frn: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Size,
    Alloc,
    Files,
    Name,
    Modified,
}

#[derive(Clone, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    names: Vec<u8>,
    /// ASCII-lowercased copy of `names`, same offsets. Used by search.
    lower: Vec<u8>,
    pub root: NodeId,
}

impl Tree {
    /// New tree whose root is named `root_path`, e.g. `C:` or `D:\Projects`.
    pub fn new(root_path: &str) -> Tree {
        Tree::with_capacity(root_path, 0, 0)
    }

    pub fn with_capacity(root_path: &str, nodes: usize, name_bytes: usize) -> Tree {
        let mut t = Tree {
            nodes: Vec::with_capacity(nodes),
            names: Vec::with_capacity(name_bytes),
            lower: Vec::with_capacity(name_bytes),
            root: 0,
        };
        let root_path = root_path.trim_end_matches('\\');
        t.root = t.push_node(root_path, flags::DIR, 0, 0, 0, 0);
        t
    }

    /// Rebuild a tree from raw parts (used by snapshot loading).
    pub fn from_parts(nodes: Vec<Node>, names: Vec<u8>, root: NodeId) -> Tree {
        let lower = names.to_ascii_lowercase();
        Tree {
            nodes,
            names,
            lower,
            root,
        }
    }

    pub fn parts(&self) -> (&[Node], &[u8]) {
        (&self.nodes, &self.names)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    #[inline]
    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    #[inline]
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    #[inline]
    pub fn name(&self, id: NodeId) -> &str {
        let n = &self.nodes[id as usize];
        let bytes = &self.names[n.name_off as usize..n.name_off as usize + n.name_len as usize];
        // Names are only ever written from `&str`.
        unsafe { std::str::from_utf8_unchecked(bytes) }
    }

    #[inline]
    pub fn name_lower(&self, id: NodeId) -> &[u8] {
        let n = &self.nodes[id as usize];
        &self.lower[n.name_off as usize..n.name_off as usize + n.name_len as usize]
    }

    /// Create a node that isn't attached to any parent yet.
    pub fn push_node(
        &mut self,
        name: &str,
        flags: u16,
        size: u64,
        alloc: u64,
        mtime: u32,
        frn: u64,
    ) -> NodeId {
        let name = truncate_utf8(name, u16::MAX as usize);
        let off = self.names.len() as u32;
        self.names.extend_from_slice(name.as_bytes());
        self.lower
            .extend(name.bytes().map(|b| b.to_ascii_lowercase()));
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            name_off: off,
            name_len: name.len() as u16,
            flags,
            size,
            alloc,
            mtime,
            files: u32::from(flags & flags::DIR == 0),
            frn,
            ..Node::default()
        });
        id
    }

    /// Add a child under `parent`. Does not update the parent's totals; call
    /// `aggregate` once after bulk inserts, or `add_live` for single updates.
    pub fn add(
        &mut self,
        parent: NodeId,
        name: &str,
        flags: u16,
        size: u64,
        alloc: u64,
        mtime: u32,
        frn: u64,
    ) -> NodeId {
        let id = self.push_node(name, flags, size, alloc, mtime, frn);
        self.link(id, parent);
        id
    }

    /// Attach a detached node as the first child of `parent`.
    pub fn link(&mut self, child: NodeId, parent: NodeId) {
        let first = self.nodes[parent as usize].first_child;
        let c = &mut self.nodes[child as usize];
        c.parent = parent;
        c.next_sibling = first;
        self.nodes[parent as usize].first_child = child;
    }

    /// Detach `child` from its parent's child list. Totals are not touched.
    pub fn unlink(&mut self, child: NodeId) {
        let parent = self.nodes[child as usize].parent;
        if parent == NONE {
            return;
        }
        let next = self.nodes[child as usize].next_sibling;
        let p = &mut self.nodes[parent as usize];
        if p.first_child == child {
            p.first_child = next;
        } else {
            let mut cur = p.first_child;
            while cur != NONE {
                let n = self.nodes[cur as usize].next_sibling;
                if n == child {
                    self.nodes[cur as usize].next_sibling = next;
                    break;
                }
                cur = n;
            }
        }
        let c = &mut self.nodes[child as usize];
        c.parent = NONE;
        c.next_sibling = NONE;
    }

    /// Live children of `id` (skips items that are `GONE`).
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            cur: self.nodes[id as usize].first_child,
            all: false,
        }
    }

    /// Children of `id` including deleted ones still shown greyed out.
    pub fn children_all(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            cur: self.nodes[id as usize].first_child,
            all: true,
        }
    }

    pub fn child_count(&self, id: NodeId) -> usize {
        self.children(id).count()
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        self.nodes[id as usize].first_child != NONE
    }

    /// Children of `id` ordered by `key` (sizes descending, names ascending).
    pub fn sorted_children(&self, id: NodeId, key: SortKey) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self.children(id).collect();
        self.sort_ids(&mut v, key);
        v
    }

    /// Like `sorted_children`, but including deleted items still shown.
    pub fn sorted_children_all(&self, id: NodeId, key: SortKey) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self.children_all(id).collect();
        self.sort_ids(&mut v, key);
        v
    }

    pub fn sort_ids(&self, v: &mut [NodeId], key: SortKey) {
        let n = &self.nodes;
        match key {
            SortKey::Size => v.sort_unstable_by(|&a, &b| {
                n[b as usize].size.cmp(&n[a as usize].size).then(a.cmp(&b))
            }),
            SortKey::Alloc => v.sort_unstable_by(|&a, &b| {
                n[b as usize]
                    .alloc
                    .cmp(&n[a as usize].alloc)
                    .then(a.cmp(&b))
            }),
            SortKey::Files => v.sort_unstable_by(|&a, &b| {
                n[b as usize]
                    .files
                    .cmp(&n[a as usize].files)
                    .then(a.cmp(&b))
            }),
            SortKey::Modified => v.sort_unstable_by(|&a, &b| {
                n[b as usize]
                    .mtime
                    .cmp(&n[a as usize].mtime)
                    .then(a.cmp(&b))
            }),
            SortKey::Name => v.sort_by(|&a, &b| {
                let (na, nb) = (&self.nodes[a as usize], &self.nodes[b as usize]);
                nb.is_dir()
                    .cmp(&na.is_dir())
                    .then_with(|| self.name_lower(a).cmp(self.name_lower(b)))
            }),
        }
    }

    /// Ancestors from `id`'s parent up to the root.
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            tree: self,
            cur: self.nodes[id as usize].parent,
        }
    }

    pub fn depth(&self, id: NodeId) -> usize {
        self.ancestors(id).count()
    }

    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> bool {
        id == ancestor || self.ancestors(id).any(|a| a == ancestor)
    }

    /// Full Windows path of `id`.
    pub fn path(&self, id: NodeId) -> String {
        let mut chain: Vec<NodeId> = vec![id];
        chain.extend(self.ancestors(id));
        let mut s = String::with_capacity(chain.len() * 16);
        for (i, &n) in chain.iter().rev().enumerate() {
            if i > 0 {
                s.push('\\');
            }
            s.push_str(self.name(n));
        }
        // A bare drive root needs its trailing slash to be a valid path.
        if chain.len() == 1 && s.ends_with(':') {
            s.push('\\');
        }
        s
    }

    /// Find a node by full path (case-insensitive). Returns `None` if the path is
    /// outside this tree or doesn't exist.
    pub fn find_path(&self, path: &str) -> Option<NodeId> {
        let root = self.name(self.root).to_ascii_lowercase();
        let lower = path.trim_end_matches('\\').to_ascii_lowercase();
        let rest = lower.strip_prefix(&root)?;
        if rest.is_empty() {
            return Some(self.root);
        }
        let rest = rest.strip_prefix('\\')?;
        let mut cur = self.root;
        for part in rest.split('\\').filter(|p| !p.is_empty()) {
            cur = self
                .children(cur)
                .find(|&c| self.name_lower(c) == part.as_bytes())?;
        }
        Some(cur)
    }

    /// Recompute folder totals bottom-up. Call after bulk construction.
    pub fn aggregate(&mut self) {
        self.aggregate_from(self.root);
    }

    /// Recompute folder totals for `top` and everything below it. Ancestors of
    /// `top` are left alone.
    fn aggregate_from(&mut self, top: NodeId) {
        // Iterative post-order: push in pre-order, then fold in reverse.
        let mut order: Vec<NodeId> = Vec::new();
        let mut stack = vec![top];
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(self.children(id));
        }
        for &id in &order {
            let n = &mut self.nodes[id as usize];
            if n.is_dir() {
                n.size = 0;
                n.alloc = 0;
                n.files = 0;
            }
        }
        for &id in order.iter().rev() {
            if id == top {
                continue;
            }
            let n = self.nodes[id as usize];
            if n.parent == NONE || n.has(flags::EXTRA_LINK) {
                continue;
            }
            let p = &mut self.nodes[n.parent as usize];
            p.size += n.size;
            p.alloc += n.alloc;
            p.files += n.files;
            p.mtime = p.mtime.max(n.mtime);
        }
    }

    /// Bring folder `target` in line with the same folder (`from`) in a fresh scan
    /// `sub`. Items are matched by name, so existing ids (and the file ids live
    /// updates rely on) survive; new items are added, missing ones removed, and
    /// every ancestor's totals adjusted.
    pub fn merge_folder(&mut self, target: NodeId, sub: &Tree, from: NodeId) {
        let before = self.nodes[target as usize];
        self.merge_children(target, sub, from);
        self.aggregate_from(target);
        let after = self.nodes[target as usize];
        if !after.has(flags::EXTRA_LINK) {
            self.propagate(
                target,
                after.size as i64 - before.size as i64,
                after.alloc as i64 - before.alloc as i64,
                after.files as i64 - before.files as i64,
                after.mtime,
            );
        }
    }

    fn merge_children(&mut self, target: NodeId, sub: &Tree, from: NodeId) {
        // A fresh scan settles what's really there; deleted leftovers go.
        let ghosts: Vec<NodeId> = self
            .children_all(target)
            .filter(|&c| self.nodes[c as usize].has(flags::GONE))
            .collect();
        for g in ghosts {
            self.drop_subtree(g);
        }
        let mut old: HashMap<Vec<u8>, NodeId> = self
            .children(target)
            .map(|c| (self.name_lower(c).to_vec(), c))
            .collect();
        for sc in sub.children(from) {
            let s = *sub.get(sc);
            match old.remove(sub.name_lower(sc)) {
                Some(oc) if self.nodes[oc as usize].is_dir() == s.is_dir() => {
                    let o = &mut self.nodes[oc as usize];
                    // Keep link and metadata flags only the MFT scan knows about.
                    o.flags = s.flags | (o.flags & (flags::EXTRA_LINK | flags::META));
                    o.mtime = s.mtime;
                    if s.frn != 0 {
                        o.frn = s.frn;
                    }
                    if s.is_dir() {
                        self.merge_children(oc, sub, sc);
                    } else {
                        o.size = s.size;
                        o.alloc = s.alloc;
                    }
                }
                replaced => {
                    if let Some(oc) = replaced {
                        self.drop_subtree(oc);
                    }
                    self.copy_from(target, sub, sc);
                }
            }
        }
        for (_, oc) in old {
            self.drop_subtree(oc);
        }
    }

    /// Copy `from` and everything below it out of `sub` and under `parent`.
    fn copy_from(&mut self, parent: NodeId, sub: &Tree, from: NodeId) {
        let mut stack = vec![(from, parent)];
        while let Some((sid, dest)) = stack.pop() {
            let n = *sub.get(sid);
            let id = self.add(dest, sub.name(sid), n.flags, n.size, n.alloc, n.mtime, n.frn);
            stack.extend(sub.children(sid).map(|c| (c, id)));
        }
    }

    /// Unlink and mark deleted without touching any totals.
    fn drop_subtree(&mut self, id: NodeId) {
        self.unlink(id);
        let mut stack = vec![id];
        while let Some(x) = stack.pop() {
            stack.extend(self.children_all(x));
            self.nodes[x as usize].flags |= flags::DELETED;
        }
    }

    /// Mark `id` (and everything below it) deleted but keep it in place so the UI
    /// can show it greyed out. Ancestors' totals drop right away.
    pub fn mark_gone(&mut self, id: NodeId) {
        let n = self.nodes[id as usize];
        if id == self.root || n.has(flags::DELETED) || n.has(flags::GONE) {
            return;
        }
        if !n.has(flags::EXTRA_LINK) {
            self.propagate(
                id,
                -(n.size as i64),
                -(n.alloc as i64),
                -(n.files as i64),
                0,
            );
        }
        let mut stack = vec![id];
        while let Some(x) = stack.pop() {
            stack.extend(self.children_all(x));
            self.nodes[x as usize].flags |= flags::GONE;
        }
    }

    /// Leave a greyed-out copy of `id` where it is now, for an item that's about to
    /// move somewhere it no longer belongs to this folder (e.g. the Recycle Bin).
    /// The copy doesn't count toward any totals.
    pub fn leave_ghost(&mut self, id: NodeId) {
        let n = self.nodes[id as usize];
        if id == self.root || n.parent == NONE {
            return;
        }
        let name = self.name(id).to_string();
        let g = self.push_node(&name, n.flags | flags::GONE, n.size, n.alloc, n.mtime, 0);
        self.nodes[g as usize].files = n.files;
        self.link(g, n.parent);
    }

    /// Forget greyed-out leftovers called `name` under `parent`, e.g. when a
    /// recycled file is restored to where it was.
    pub fn drop_ghosts_named(&mut self, parent: NodeId, name: &str) {
        let lower = name.to_ascii_lowercase();
        let ghosts: Vec<NodeId> = self
            .children_all(parent)
            .filter(|&c| {
                self.nodes[c as usize].has(flags::GONE) && self.name_lower(c) == lower.as_bytes()
            })
            .collect();
        for g in ghosts {
            self.drop_subtree(g);
        }
    }

    /// Add signed deltas to every ancestor of `id` (not `id` itself).
    pub fn propagate(&mut self, id: NodeId, dsize: i64, dalloc: i64, dfiles: i64, mtime: u32) {
        let mut cur = self.nodes[id as usize].parent;
        while cur != NONE {
            let p = &mut self.nodes[cur as usize];
            p.size = p.size.saturating_add_signed(dsize);
            p.alloc = p.alloc.saturating_add_signed(dalloc);
            p.files = (p.files as i64 + dfiles).max(0) as u32;
            p.mtime = p.mtime.max(mtime);
            cur = p.parent;
        }
    }

    /// Insert a node and update ancestor totals.
    pub fn add_live(
        &mut self,
        parent: NodeId,
        name: &str,
        flags: u16,
        size: u64,
        alloc: u64,
        mtime: u32,
        frn: u64,
    ) -> NodeId {
        let id = self.add(parent, name, flags, size, alloc, mtime, frn);
        let n = self.nodes[id as usize];
        if !n.has(flags::EXTRA_LINK) {
            self.propagate(id, size as i64, alloc as i64, n.files as i64, mtime);
        }
        id
    }

    /// Change a file's size and update ancestor totals.
    pub fn set_size(&mut self, id: NodeId, size: u64, alloc: u64, mtime: u32) {
        let n = &mut self.nodes[id as usize];
        let (ds, da) = (size as i64 - n.size as i64, alloc as i64 - n.alloc as i64);
        n.size = size;
        n.alloc = alloc;
        n.mtime = mtime;
        if !n.has(flags::EXTRA_LINK) {
            self.propagate(id, ds, da, 0, mtime);
        }
    }

    /// Remove `id` and everything below it, updating ancestor totals. Node slots are
    /// marked deleted rather than reused, so ids held elsewhere stay valid.
    pub fn remove(&mut self, id: NodeId) {
        if id == self.root {
            return;
        }
        let n = self.nodes[id as usize];
        if n.has(flags::DELETED) {
            return;
        }
        if !n.has(flags::EXTRA_LINK) && !n.has(flags::GONE) {
            self.propagate(
                id,
                -(n.size as i64),
                -(n.alloc as i64),
                -(n.files as i64),
                0,
            );
        }
        self.unlink(id);
        let mut stack = vec![id];
        while let Some(x) = stack.pop() {
            stack.extend(self.children(x));
            self.nodes[x as usize].flags |= flags::DELETED;
        }
    }

    /// Move `id` under `new_parent`, carrying its totals along.
    pub fn move_to(&mut self, id: NodeId, new_parent: NodeId) {
        let n = self.nodes[id as usize];
        let counted = !n.has(flags::EXTRA_LINK);
        if counted {
            self.propagate(
                id,
                -(n.size as i64),
                -(n.alloc as i64),
                -(n.files as i64),
                0,
            );
        }
        self.unlink(id);
        self.link(id, new_parent);
        if counted {
            self.propagate(id, n.size as i64, n.alloc as i64, n.files as i64, n.mtime);
        }
    }

    /// Point `id` at a new name. The old bytes stay in the buffer.
    pub fn rename(&mut self, id: NodeId, name: &str) {
        let name = truncate_utf8(name, u16::MAX as usize);
        let off = self.names.len() as u32;
        self.names.extend_from_slice(name.as_bytes());
        self.lower
            .extend(name.bytes().map(|b| b.to_ascii_lowercase()));
        let n = &mut self.nodes[id as usize];
        n.name_off = off;
        n.name_len = name.len() as u16;
    }

    /// Number of live (non-deleted) folders, excluding the root.
    pub fn dir_count(&self) -> usize {
        self.nodes
            .iter()
            .skip(1)
            .filter(|n| n.is_dir() && !n.has(flags::DELETED) && !n.has(flags::GONE))
            .count()
    }

    /// Copy only the live nodes into a fresh tree with compact ids. Children keep
    /// their order. Useful before saving a snapshot after many live updates.
    pub fn compact(&self) -> Tree {
        let live = self
            .nodes
            .iter()
            .filter(|n| !n.has(flags::DELETED) && !n.has(flags::GONE))
            .count();
        let mut out = Tree::with_capacity(self.name(self.root), live, self.names.len());
        let r = self.nodes[self.root as usize];
        {
            let o = &mut out.nodes[0];
            (o.size, o.alloc, o.files, o.mtime, o.frn) = (r.size, r.alloc, r.files, r.mtime, r.frn);
        }
        let mut stack = vec![(self.root, out.root)];
        while let Some((src, dst)) = stack.pop() {
            // Collect then insert in reverse so `link` (which prepends) keeps the order.
            let kids: Vec<NodeId> = self.children(src).collect();
            for &c in kids.iter().rev() {
                let n = self.nodes[c as usize];
                let id = out.add(dst, self.name(c), n.flags, n.size, n.alloc, n.mtime, n.frn);
                out.nodes[id as usize].files = n.files;
                stack.push((c, id));
            }
        }
        out
    }
}

pub struct Children<'a> {
    tree: &'a Tree,
    cur: NodeId,
    all: bool,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    #[inline]
    fn next(&mut self) -> Option<NodeId> {
        while self.cur != NONE {
            let id = self.cur;
            let n = &self.tree.nodes[id as usize];
            self.cur = n.next_sibling;
            if self.all || n.flags & flags::GONE == 0 {
                return Some(id);
            }
        }
        None
    }
}

pub struct Ancestors<'a> {
    tree: &'a Tree,
    cur: NodeId,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;
    #[inline]
    fn next(&mut self) -> Option<NodeId> {
        if self.cur == NONE {
            return None;
        }
        let id = self.cur;
        self.cur = self.tree.nodes[id as usize].parent;
        Some(id)
    }
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Tree, NodeId, NodeId, NodeId) {
        let mut t = Tree::new("C:");
        let users = t.add(t.root, "Users", flags::DIR, 0, 0, 0, 0);
        let a = t.add(users, "a.bin", 0, 100, 4096, 10, 0);
        let b = t.add(users, "B.txt", 0, 50, 4096, 20, 0);
        t.add(t.root, "pagefile.sys", 0, 1000, 1000, 5, 0);
        t.aggregate();
        (t, users, a, b)
    }

    #[test]
    fn merge_folder_keeps_ids_and_fixes_totals() {
        let (mut t, users, a, b) = sample();
        let mut fresh = Tree::new(r"C:\Users");
        fresh.add(fresh.root, "A.BIN", 0, 300, 8192, 30, 0);
        let docs = fresh.add(fresh.root, "Docs", flags::DIR, 0, 0, 0, 0);
        fresh.add(docs, "c.pdf", 0, 10, 4096, 40, 0);
        fresh.aggregate();

        t.merge_folder(users, &fresh, fresh.root);

        // a.bin matched (case-insensitively) and kept its id; B.txt is gone.
        assert_eq!(t.get(a).size, 300);
        assert!(t.get(b).has(flags::DELETED));
        assert_eq!(t.child_count(users), 2);
        let u = t.get(users);
        assert_eq!((u.size, u.alloc, u.files), (310, 12288, 2));
        let r = t.get(t.root);
        assert_eq!((r.size, r.alloc, r.files), (1310, 13288, 3));
        let pdf = t.find_path(r"C:\Users\Docs\c.pdf").unwrap();
        assert_eq!(t.get(pdf).size, 10);
    }

    #[test]
    fn aggregates_and_paths() {
        let (t, users, a, _) = sample();
        assert_eq!(t.get(t.root).size, 1150);
        assert_eq!(t.get(t.root).files, 3);
        assert_eq!(t.get(users).size, 150);
        assert_eq!(t.get(users).mtime, 20);
        assert_eq!(t.path(a), "C:\\Users\\a.bin");
        assert_eq!(t.path(t.root), "C:\\");
        assert_eq!(
            t.find_path("c:\\users\\b.TXT"),
            t.children(users).find(|&c| t.name(c) == "B.txt")
        );
        assert_eq!(t.find_path("C:\\"), Some(t.root));
        assert_eq!(t.find_path("D:\\x"), None);
    }

    #[test]
    fn live_updates_keep_totals() {
        let (mut t, users, a, b) = sample();
        t.set_size(a, 300, 4096, 30);
        assert_eq!(t.get(users).size, 350);
        assert_eq!(t.get(t.root).size, 1350);
        t.remove(b);
        assert_eq!(t.get(users).size, 300);
        assert_eq!(t.get(t.root).files, 2);
        assert_eq!(t.child_count(users), 1);
        let docs = t.add_live(t.root, "Docs", flags::DIR, 0, 0, 0, 0);
        t.move_to(a, docs);
        assert_eq!(t.get(users).size, 0);
        assert_eq!(t.get(docs).size, 300);
        assert_eq!(t.get(t.root).size, 1300);
        let c = t.compact();
        assert_eq!(c.len(), 5);
        assert_eq!(c.get(c.root).size, 1300);
    }

    #[test]
    fn sorting() {
        let (t, users, a, b) = sample();
        assert_eq!(t.sorted_children(users, SortKey::Size), vec![a, b]);
        assert_eq!(t.sorted_children(users, SortKey::Name), vec![a, b]);
        let top = t.sorted_children(t.root, SortKey::Name);
        assert_eq!(t.name(top[0]), "Users");
    }

    #[test]
    fn extra_links_not_double_counted() {
        let mut t = Tree::new("C:");
        let d = t.add(t.root, "d", flags::DIR, 0, 0, 0, 0);
        t.add(d, "f", 0, 10, 10, 0, 0);
        t.add(t.root, "f-link", flags::EXTRA_LINK, 10, 10, 0, 0);
        t.aggregate();
        assert_eq!(t.get(t.root).size, 10);
    }
}
