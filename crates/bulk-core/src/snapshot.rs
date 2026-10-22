//! Saved scans. Each drive (keyed by volume serial, so letters can change) has a
//! `current.bulk` that is overwritten as the tree changes, used to reopen
//! instantly, and a `history/` of dated snapshots used for "what changed".

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tree::Tree;
use crate::usn::Journal;
use crate::wire;

const MAGIC: &[u8; 4] = b"BLKS";
const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub letter: char,
    pub serial: u32,
    pub label: String,
    /// Unix seconds when the tree was last brought up to date.
    pub taken: u64,
    /// How the data was gathered: `mft` or `walk`.
    pub method: String,
    pub files: u32,
    pub alloc: u64,
    pub size: u64,
    /// Change journal position the tree is current to (NTFS + helper only).
    pub journal: Option<Journal>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: PathBuf,
    #[serde(flatten)]
    pub meta: Meta,
}

/// `%LOCALAPPDATA%\Bulk\snapshots`
pub fn root() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Bulk").join("snapshots")
}

pub fn drive_dir(serial: u32) -> PathBuf {
    root().join(format!("{serial:08X}"))
}

pub fn current_path(serial: u32) -> PathBuf {
    drive_dir(serial).join("current.bulk")
}

fn history_dir(serial: u32) -> PathBuf {
    drive_dir(serial).join("history")
}

pub fn save(path: &Path, meta: &Meta, tree: &Tree) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write beside the target and rename, so a crash never leaves half a file.
    let tmp = path.with_extension("tmp");
    {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        let meta_json = serde_json::to_vec(meta)?;
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&(meta_json.len() as u32).to_le_bytes())?;
        w.write_all(&meta_json)?;
        let mut z = zstd::Encoder::new(w, 1)?;
        z.multithread(
            std::thread::available_parallelism()
                .map_or(1, |n| n.get() as u32 / 2)
                .max(1),
        )?;
        wire::write_tree(&mut z, tree)?;
        z.finish()?.flush()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn save_current(meta: &Meta, tree: &Tree) -> anyhow::Result<()> {
    save(&current_path(meta.serial), meta, tree)
}

/// Add a dated snapshot and keep only the newest `keep`.
pub fn save_history(meta: &Meta, tree: &Tree, keep: usize) -> anyhow::Result<PathBuf> {
    let path = history_dir(meta.serial).join(format!("{}.bulk", meta.taken));
    save(&path, meta, tree)?;
    prune(meta.serial, keep);
    Ok(path)
}

fn read_meta(r: &mut impl Read) -> anyhow::Result<Meta> {
    let mut head = [0u8; 12];
    r.read_exact(&mut head)?;
    anyhow::ensure!(&head[0..4] == MAGIC, "not a Bulk snapshot");
    anyhow::ensure!(
        u32::from_le_bytes(head[4..8].try_into()?) == VERSION,
        "unsupported snapshot version"
    );
    let len = u32::from_le_bytes(head[8..12].try_into()?) as usize;
    anyhow::ensure!(len < 1 << 20, "corrupt snapshot");
    let mut json = vec![0u8; len];
    r.read_exact(&mut json)?;
    Ok(serde_json::from_slice(&json)?)
}

pub fn load_meta(path: &Path) -> anyhow::Result<Meta> {
    read_meta(&mut BufReader::new(File::open(path)?))
}

pub fn load(path: &Path) -> anyhow::Result<(Meta, Tree)> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(path)?);
    let meta = read_meta(&mut r)?;
    let mut z = zstd::Decoder::with_buffer(r)?;
    let tree = wire::read_tree(&mut z)?;
    Ok((meta, tree))
}

/// History snapshots for a drive, newest first.
pub fn history(serial: u32) -> Vec<Entry> {
    let Ok(dir) = std::fs::read_dir(history_dir(serial)) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "bulk"))
        .filter_map(|path| load_meta(&path).ok().map(|meta| Entry { path, meta }))
        .collect();
    out.sort_by(|a, b| b.meta.taken.cmp(&a.meta.taken));
    out
}

pub fn prune(serial: u32, keep: usize) {
    for e in history(serial).into_iter().skip(keep) {
        let _ = std::fs::remove_file(e.path);
    }
}

/// Whether `path` is a snapshot file Bulk wrote (guards commands that take a path).
///
/// Checked lexically rather than with `canonicalize`, which follows Windows' AppData
/// redirection for packaged parent apps and then no longer matches `root()`.
pub fn is_snapshot_path(path: &Path) -> bool {
    use std::path::Component;
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    let p = path.to_string_lossy().to_ascii_lowercase();
    let r = root().to_string_lossy().to_ascii_lowercase();
    p.starts_with(&format!("{}\\", r.trim_end_matches('\\')))
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bulk"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::flags;

    #[test]
    fn save_and_load_roundtrip() {
        let mut t = Tree::new("Q:");
        let d = t.add(t.root, "dir", flags::DIR, 0, 0, 0, 0);
        t.add(d, "f", 0, 42, 4096, 3, 77);
        t.aggregate();
        let meta = Meta {
            letter: 'Q',
            serial: 0xDEAD,
            label: "test".into(),
            taken: 123,
            method: "walk".into(),
            files: 1,
            alloc: 4096,
            size: 42,
            journal: Some(Journal {
                id: 1,
                first: 0,
                next: 99,
            }),
        };
        let path = std::env::temp_dir().join(format!("bulk-test-{}.bulk", std::process::id()));
        save(&path, &meta, &t).unwrap();
        let (m, back) = load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(m.journal.unwrap().next, 99);
        assert_eq!(back.get(back.root).size, 42);
        assert!(back.find_path("Q:\\dir\\f").is_some());
    }

    #[test]
    fn snapshot_path_check() {
        let ok = drive_dir(1).join("history").join("5.bulk");
        assert!(is_snapshot_path(&ok));
        assert!(!is_snapshot_path(
            &drive_dir(1).join("..").join("..").join("x.bulk")
        ));
        assert!(!is_snapshot_path(Path::new(r"C:\Windows\x.bulk")));
        assert!(!is_snapshot_path(&drive_dir(1).join("current.txt")));
    }
}
