//! Duplicate files. Narrowed down in stages so only real candidates are read in
//! full: same size, then same first and last 64 KB, then same BLAKE3 hash.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rayon::prelude::*;
use serde::Serialize;

use crate::tree::{NodeId, Tree, flags};

const EDGE: u64 = 64 * 1024;
const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;

/// Folders skipped when `skip_system` is on: duplicates there are normal and not
/// safe to remove.
const SYSTEM_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "$windows.~bt",
    "$windows.~ws",
    "recovery",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: NodeId,
    pub path: String,
    pub size: u64,
    pub mtime: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub size: u64,
    pub files: Vec<Candidate>,
}

impl Group {
    /// Space that removing all but one copy would free.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

#[derive(Default)]
pub struct DupeProgress {
    /// 0 = listing, 1 = quick check, 2 = full compare.
    pub phase: AtomicU64,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    cancel: AtomicBool,
}

impl DupeProgress {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Files under `root` worth comparing: at least `min_size`, stored locally, and
/// not a second hard link to something already listed.
/// With `filter`, only files matching it are considered.
pub fn candidates(
    tree: &Tree,
    root: NodeId,
    min_size: u64,
    skip_system: bool,
    filter: Option<&crate::filter::Filter>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        for c in tree.children(id) {
            let n = tree.get(c);
            if n.has(flags::EXTRA_LINK)
                || n.has(flags::REPARSE)
                || n.has(flags::OFFLINE)
                || n.has(flags::META)
                || filter.is_some_and(|f| !f.shows(c))
            {
                continue;
            }
            if n.is_dir() {
                let name = std::str::from_utf8(tree.name_lower(c)).unwrap_or("");
                if !(skip_system && SYSTEM_DIRS.contains(&name)) {
                    stack.push(c);
                }
            } else if n.size >= min_size.max(1) {
                out.push(Candidate {
                    id: c,
                    path: String::new(),
                    size: n.size,
                    mtime: n.mtime,
                });
            }
        }
    }
    // Only files that share a size with another file can be duplicates; resolve
    // paths just for those.
    let mut by_size: HashMap<u64, usize> = HashMap::new();
    for c in &out {
        *by_size.entry(c.size).or_default() += 1;
    }
    out.retain(|c| by_size[&c.size] > 1);
    for c in &mut out {
        c.path = tree.path(c.id);
    }
    out
}

pub fn find(cands: Vec<Candidate>, p: &DupeProgress) -> Vec<Group> {
    // Stage 1: same size.
    let groups = regroup(cands.into_iter().map(|c| ((c.size, 0u128), c)));

    // Stage 2: same head and tail.
    p.phase.store(1, Ordering::Relaxed);
    let stage2: Vec<Candidate> = groups.into_iter().flatten().collect();
    p.files_total.store(stage2.len() as u64, Ordering::Relaxed);
    p.files_done.store(0, Ordering::Relaxed);
    // Opening files is slow mostly because of waiting (antivirus, cold metadata),
    // not CPU, so this stage gets a wider pool than the core count.
    let wide = rayon::ThreadPoolBuilder::new().num_threads(32).build();
    let quick = || -> Vec<((u64, u128), Candidate)> {
        stage2
            .into_par_iter()
            .filter_map(|c| {
                if p.cancelled() {
                    return None;
                }
                let h = sample_hash(&c.path, c.size).ok();
                p.files_done.fetch_add(1, Ordering::Relaxed);
                h.map(|h| ((c.size, h as u128), c))
            })
            .collect()
    };
    let keyed = match wide {
        Ok(pool) => pool.install(quick),
        Err(_) => quick(),
    };
    let groups = regroup(keyed);
    if p.cancelled() {
        return Vec::new();
    }

    // Stage 3: full hash. Files small enough to be covered by the edge hash are
    // already fully compared.
    p.phase.store(2, Ordering::Relaxed);
    let (small, big): (Vec<Vec<Candidate>>, Vec<Vec<Candidate>>) =
        groups.into_iter().partition(|g| g[0].size <= EDGE * 2);
    let stage3: Vec<Candidate> = big.into_iter().flatten().collect();
    p.files_total.store(stage3.len() as u64, Ordering::Relaxed);
    p.files_done.store(0, Ordering::Relaxed);
    p.bytes_total
        .store(stage3.iter().map(|c| c.size).sum(), Ordering::Relaxed);
    p.bytes_done.store(0, Ordering::Relaxed);
    let keyed: Vec<((u64, u128), Candidate)> = stage3
        .into_par_iter()
        .with_max_len(1)
        .filter_map(|c| {
            if p.cancelled() {
                return None;
            }
            let h = full_hash(&c.path, p).ok();
            p.files_done.fetch_add(1, Ordering::Relaxed);
            h.map(|h| ((c.size, h), c))
        })
        .collect();
    if p.cancelled() {
        return Vec::new();
    }

    let mut out: Vec<Group> = regroup(keyed)
        .into_iter()
        .chain(small)
        .map(|files| Group {
            size: files[0].size,
            files,
        })
        .collect();
    out.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    out
}

/// Bucket by key, keeping only buckets with two or more entries.
fn regroup(items: impl IntoIterator<Item = ((u64, u128), Candidate)>) -> Vec<Vec<Candidate>> {
    let mut m: HashMap<(u64, u128), Vec<Candidate>> = HashMap::new();
    for (k, c) in items {
        m.entry(k).or_default().push(c);
    }
    m.into_values().filter(|g| g.len() > 1).collect()
}

fn open(path: &str) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
        .open(long(path))
}

/// `\\?\` prefix so paths over 260 characters still open.
fn long(path: &str) -> String {
    if path.starts_with(r"\\") {
        path.to_string()
    } else {
        format!(r"\\?\{path}")
    }
}

/// Hash 64 KB at the start, the end and three points in between. Files of 128 KB
/// or less are read whole, so for them this is a full comparison.
fn sample_hash(path: &str, size: u64) -> std::io::Result<u64> {
    let mut f = open(path)?;
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = vec![0u8; EDGE as usize];
    if size <= EDGE * 2 {
        let mut all = Vec::with_capacity(size as usize);
        f.read_to_end(&mut all)?;
        h.update(&all);
        return Ok(h.digest());
    }
    let last = size - EDGE;
    for at in [0, size / 4, size / 2, size / 4 * 3, last] {
        f.seek(SeekFrom::Start(at.min(last)))?;
        let n = read_full(&mut f, &mut buf)?;
        h.update(&buf[..n]);
    }
    Ok(h.digest())
}

fn full_hash(path: &str, p: &DupeProgress) -> std::io::Result<u128> {
    let mut f = open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if p.cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        p.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
    }
    let out = h.finalize();
    Ok(u128::from_le_bytes(
        out.as_bytes()[..16].try_into().unwrap(),
    ))
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        let n = f.read(&mut buf[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_true_duplicates_only() {
        let dir = std::env::temp_dir().join(format!("bulk-dupes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut other = big.clone();
        other[150_000] ^= 1; // same size, same edges, different middle
        std::fs::write(dir.join("a.bin"), &big).unwrap();
        std::fs::write(dir.join("b.bin"), &big).unwrap();
        std::fs::write(dir.join("c.bin"), &other).unwrap();
        std::fs::write(dir.join("d.txt"), b"tiny but same").unwrap();
        std::fs::write(dir.join("e.txt"), b"tiny but same").unwrap();

        let cand = |name: &str, size: u64| Candidate {
            id: 0,
            path: dir.join(name).to_string_lossy().into_owned(),
            size,
            mtime: 0,
        };
        let groups = find(
            vec![
                cand("a.bin", 300_000),
                cand("b.bin", 300_000),
                cand("c.bin", 300_000),
                cand("d.txt", 13),
                cand("e.txt", 13),
            ],
            &DupeProgress::default(),
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].size, 300_000);
        assert_eq!(groups[0].files.len(), 2);
        assert!(groups[0].files.iter().all(|f| !f.path.ends_with("c.bin")));
    }
}
