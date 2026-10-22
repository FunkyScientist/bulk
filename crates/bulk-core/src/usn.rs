//! NTFS change journal: find out which files changed since a USN, read their current
//! state from the MFT, and fold that into an existing tree.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::scan::mft::{FileState, Volume};
use crate::tree::{NodeId, Tree, flags};

const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00F4;
const FSCTL_READ_USN_JOURNAL: u32 = 0x0009_00BB;
const ERROR_JOURNAL_ENTRY_DELETED: i32 = 1181;
const ERROR_JOURNAL_DELETE_IN_PROGRESS: i32 = 1178;
const ERROR_JOURNAL_NOT_ACTIVE: i32 = 1179;

const RECORD_MASK: u64 = 0xFFFF_FFFF_FFFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub id: u64,
    /// Oldest USN still in the journal.
    pub first: i64,
    /// USN the next change will get.
    pub next: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// The journal was reset or has wrapped past `from`; a full rescan is needed.
    #[error("change journal no longer covers the snapshot")]
    Gap,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn query(vol: &Volume) -> Result<Journal, JournalError> {
    let mut out = [0u8; 80];
    vol.ioctl(FSCTL_QUERY_USN_JOURNAL, &[], &mut out)
        .map_err(map_err)?;
    Ok(Journal {
        id: u64::from_le_bytes(out[0..8].try_into().unwrap()),
        first: i64::from_le_bytes(out[8..16].try_into().unwrap()),
        next: i64::from_le_bytes(out[16..24].try_into().unwrap()),
    })
}

fn map_err(e: std::io::Error) -> JournalError {
    match e.raw_os_error() {
        Some(
            ERROR_JOURNAL_ENTRY_DELETED
            | ERROR_JOURNAL_DELETE_IN_PROGRESS
            | ERROR_JOURNAL_NOT_ACTIVE,
        ) => JournalError::Gap,
        _ => JournalError::Io(e),
    }
}

/// File references that changed from `from` onwards, and the USN to continue from.
/// With `wait`, blocks up to that long for the first new entry.
pub fn changed_since(
    vol: &Volume,
    journal: u64,
    from: i64,
    wait: Option<Duration>,
) -> Result<(Vec<u64>, i64), JournalError> {
    let mut frns = HashSet::new();
    let mut next = from;
    let mut buf = vec![0u8; 256 * 1024];
    let mut first = true;
    loop {
        // READ_USN_JOURNAL_DATA_V0
        let mut input = [0u8; 40];
        input[0..8].copy_from_slice(&next.to_le_bytes());
        input[8..12].copy_from_slice(&u32::MAX.to_le_bytes()); // every reason
        let (timeout, bytes) = match wait {
            Some(w) if first => (w.as_secs().max(1), 1u64),
            _ => (0, 0),
        };
        input[16..24].copy_from_slice(&timeout.to_le_bytes());
        input[24..32].copy_from_slice(&bytes.to_le_bytes());
        input[32..40].copy_from_slice(&journal.to_le_bytes());
        first = false;

        let got = vol
            .ioctl(FSCTL_READ_USN_JOURNAL, &input, &mut buf)
            .map_err(map_err)? as usize;
        if got < 8 {
            break;
        }
        let new_next = i64::from_le_bytes(buf[0..8].try_into().unwrap());
        let mut off = 8;
        while off + 60 <= got {
            let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
            if len < 60 || off + len > got {
                break;
            }
            let major = u16::from_le_bytes(buf[off + 4..off + 6].try_into().unwrap());
            if major == 2 {
                frns.insert(u64::from_le_bytes(
                    buf[off + 8..off + 16].try_into().unwrap(),
                ));
            }
            off += len;
        }
        let caught_up = got == 8 || new_next == next;
        next = new_next;
        if caught_up {
            break;
        }
    }
    Ok((frns.into_iter().collect(), next))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Removed(u64),
    Present(FileStateWire),
}

/// `FileState` in a serialisable form for the helper pipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileStateWire {
    pub frn: u64,
    pub flags: u16,
    pub size: u64,
    pub alloc: u64,
    pub mtime: u32,
    pub names: Vec<(u64, String)>,
}

impl From<FileState> for FileStateWire {
    fn from(s: FileState) -> Self {
        FileStateWire {
            frn: s.frn,
            flags: s.flags,
            size: s.size,
            alloc: s.alloc,
            mtime: s.mtime,
            names: s.names,
        }
    }
}

/// Look up the current state of each changed file.
pub fn states(vol: &Volume, frns: &[u64]) -> Vec<Change> {
    frns.iter()
        .map(|&frn| match vol.file_state(frn) {
            Ok(Some(s)) => Change::Present(s.into()),
            _ => Change::Removed(frn),
        })
        .collect()
}

/// Maps MFT record numbers to the primary node for that file.
pub struct FrnIndex(HashMap<u64, NodeId>);

impl FrnIndex {
    pub fn build(tree: &Tree) -> FrnIndex {
        let mut m = HashMap::with_capacity(tree.len());
        for (i, n) in tree.nodes.iter().enumerate() {
            if n.frn != 0 && !n.has(flags::EXTRA_LINK) && !n.has(flags::DELETED | flags::GONE) {
                m.insert(n.frn & RECORD_MASK, i as NodeId);
            }
        }
        FrnIndex(m)
    }

    fn live(&self, tree: &Tree, frn: u64) -> Option<NodeId> {
        let id = *self.0.get(&(frn & RECORD_MASK))?;
        let n = tree.get(id);
        (!n.has(flags::DELETED | flags::GONE)).then_some(id)
    }
}

/// Fold journal changes into `tree`. Returns how many nodes changed.
pub fn apply(tree: &mut Tree, index: &mut FrnIndex, changes: &[Change]) -> usize {
    let mut touched = 0;
    let mut pending: Vec<&FileStateWire> = Vec::new();

    for c in changes {
        match c {
            Change::Removed(frn) => {
                if let Some(id) = index.live(tree, *frn)
                    && tree.get(id).frn == *frn
                    && id != tree.root
                {
                    // Keep it on screen, greyed out, until the next scan.
                    tree.mark_gone(id);
                    index.0.remove(&(frn & RECORD_MASK));
                    touched += 1;
                }
            }
            Change::Present(s) => pending.push(s),
        }
    }

    // New folders can arrive in the same batch as their contents, so keep going
    // while each pass places something.
    loop {
        let before = pending.len();
        pending.retain(|s| !apply_one(tree, index, s, &mut touched));
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }
    touched
}

/// Returns true once the change has been placed (or can be ignored).
fn apply_one(
    tree: &mut Tree,
    index: &mut FrnIndex,
    s: &FileStateWire,
    touched: &mut usize,
) -> bool {
    let Some((parent_ref, name)) = s.names.first() else {
        return true;
    };
    let rec = s.frn & RECORD_MASK;
    if index.live(tree, s.frn) == Some(tree.root) {
        return true;
    }
    let Some(parent) = index
        .live(tree, *parent_ref)
        .filter(|&p| tree.get(p).is_dir())
    else {
        return false;
    };

    let is_dir = s.flags & flags::DIR != 0;
    match index.live(tree, s.frn) {
        // Same record number but a different file: the old one is gone.
        Some(old) if tree.get(old).frn != s.frn => {
            tree.remove(old);
            index.0.remove(&rec);
            let f = s.flags | flags::NEW;
            let id = tree.add_live(parent, name, f, s.size, s.alloc, s.mtime, s.frn);
            index.0.insert(rec, id);
        }
        Some(id) => {
            let n = *tree.get(id);
            if n.parent != parent && !tree.is_ancestor(id, parent) {
                // Deleting in Explorer is a move into the Recycle Bin; show it as
                // deleted where it was.
                if in_recycle_bin(tree, parent) && !in_recycle_bin(tree, id) {
                    tree.leave_ghost(id);
                }
                tree.move_to(id, parent);
                tree.drop_ghosts_named(parent, name);
            }
            if tree.name(id) != name {
                tree.rename(id, name);
            }
            if is_dir {
                let keep = n.flags & (flags::DIR | flags::EXTRA_LINK | flags::DELETED | flags::NEW);
                tree.get_mut(id).flags = keep | (s.flags & !flags::DIR);
            } else {
                let keep = n.flags & (flags::EXTRA_LINK | flags::DELETED | flags::NEW);
                tree.get_mut(id).flags = keep | s.flags;
                tree.set_size(id, s.size, s.alloc, s.mtime);
            }
        }
        None => {
            tree.drop_ghosts_named(parent, name);
            let f = s.flags | flags::NEW;
            let id = tree.add_live(parent, name, f, s.size, s.alloc, s.mtime, s.frn);
            index.0.insert(rec, id);
        }
    }
    *touched += 1;
    true
}

/// Whether `id` is inside the drive's `$Recycle.Bin`.
fn in_recycle_bin(tree: &Tree, id: NodeId) -> bool {
    let top = std::iter::once(id)
        .chain(tree.ancestors(id))
        .take_while(|&a| a != tree.root)
        .last();
    top.is_some_and(|t| tree.name_lower(t) == b"$recycle.bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(frn: u64, parent: u64, name: &str, size: u64, dir: bool) -> Change {
        Change::Present(FileStateWire {
            frn,
            flags: if dir { flags::DIR } else { 0 },
            size,
            alloc: size,
            mtime: 1,
            names: vec![(parent, name.into())],
        })
    }

    #[test]
    fn applies_adds_moves_resizes_and_deletes() {
        let mut t = Tree::new("C:");
        t.get_mut(t.root).frn = 5;
        let docs = t.add(t.root, "Docs", flags::DIR, 0, 0, 0, 100);
        let a = t.add(docs, "a.txt", 0, 10, 10, 0, 101);
        t.add(t.root, "b.txt", 0, 20, 20, 0, 102);
        t.aggregate();
        let mut idx = FrnIndex::build(&t);

        let n = apply(
            &mut t,
            &mut idx,
            &[
                st(101, 5, "a2.txt", 50, false),   // moved to root, renamed, grown
                st(201, 200, "new.bin", 7, false), // child of a folder in this batch
                st(200, 100, "New", 0, true),      // new folder
                Change::Removed(102),              // deleted
            ],
        );
        assert_eq!(n, 4);
        assert_eq!(t.name(a), "a2.txt");
        assert_eq!(t.get(a).parent, t.root);
        assert_eq!(t.get(docs).size, 7);
        assert_eq!(t.get(t.root).size, 57);
        let new = t.find_path("C:\\Docs\\New\\new.bin").unwrap();
        assert!(t.get(new).has(flags::NEW));
        assert!(!t.get(a).has(flags::NEW));
        assert!(t.find_path("C:\\b.txt").is_none());
        // Saved snapshots start clean.
        let c = t.compact();
        let saved = c.find_path("C:\\Docs\\New\\new.bin").unwrap();
        assert!(!c.get(saved).has(flags::NEW));
    }

    #[test]
    fn deleted_items_stay_greyed_out_until_restored() {
        let mut t = Tree::new("C:");
        t.get_mut(t.root).frn = 5;
        let bin = t.add(t.root, "$Recycle.Bin", flags::DIR, 0, 0, 0, 300);
        let docs = t.add(t.root, "Docs", flags::DIR, 0, 0, 0, 100);
        let a = t.add(docs, "a.txt", 0, 10, 10, 0, 101);
        t.add(docs, "b.txt", 0, 20, 20, 0, 102);
        t.aggregate();
        let mut idx = FrnIndex::build(&t);

        // a.txt goes to the Recycle Bin, b.txt is deleted outright.
        apply(
            &mut t,
            &mut idx,
            &[st(101, 300, "$R1.txt", 10, false), Change::Removed(102)],
        );
        assert_eq!(t.get(docs).size, 0);
        assert_eq!(t.children(docs).count(), 0);
        let ghosts: Vec<_> = t.children_all(docs).map(|g| t.name(g).to_string()).collect();
        assert_eq!(ghosts.len(), 2);
        assert!(ghosts.iter().all(|n| n == "a.txt" || n == "b.txt"));
        assert_eq!(t.get(a).parent, bin);

        // Restoring a.txt puts it back and clears its ghost.
        apply(&mut t, &mut idx, &[st(101, 100, "a.txt", 10, false)]);
        assert_eq!(t.get(docs).size, 10);
        assert_eq!(t.children_all(docs).count(), 2);
        assert!(t.compact().find_path("C:\\Docs\\b.txt").is_none());
    }
}
