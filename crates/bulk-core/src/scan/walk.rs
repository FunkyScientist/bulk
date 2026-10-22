//! Parallel directory walker built on `FindFirstFileExW`. Used for non-NTFS drives,
//! for single folders, and when the elevated helper isn't available.

use rayon::prelude::*;
use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE, NO_ERROR};
use windows_sys::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, GetCompressedFileSizeW, INVALID_FILE_SIZE, WIN32_FIND_DATAW,
};

use super::*;
use crate::tree::{NodeId, Tree, flags};

struct Entry {
    name: Box<str>,
    flags: u16,
    size: u64,
    alloc: u64,
    mtime: u32,
    children: Vec<Entry>,
}

/// Walk `root` (a drive like `C:\` or any folder) into a tree. `cluster` is the
/// volume's allocation unit, used to estimate size on disk.
pub fn walk(root: &str, cluster: u64, progress: &Progress) -> anyhow::Result<Tree> {
    let root = root.trim_end_matches('\\');
    let prefix: Vec<u16> = long_path(root).encode_utf16().collect();
    let cluster = cluster.max(512);

    let entries = list_dir(&prefix, cluster, progress);
    if progress.cancelled() {
        anyhow::bail!("scan cancelled");
    }

    let count = progress.files.load(Ordering::Relaxed) + progress.dirs.load(Ordering::Relaxed);
    let mut tree = Tree::with_capacity(root, count as usize + 1, count as usize * 16);
    let root_id = tree.root;
    insert(&mut tree, root_id, entries);
    tree.aggregate();
    Ok(tree)
}

fn insert(tree: &mut Tree, parent: NodeId, entries: Vec<Entry>) {
    for e in entries {
        let id = tree.add(parent, &e.name, e.flags, e.size, e.alloc, e.mtime, 0);
        if !e.children.is_empty() {
            insert(tree, id, e.children);
        }
    }
}

fn long_path(p: &str) -> String {
    if p.starts_with(r"\\?\") {
        p.to_string()
    } else if let Some(unc) = p.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else {
        format!(r"\\?\{p}")
    }
}

fn list_dir(dir: &[u16], cluster: u64, progress: &Progress) -> Vec<Entry> {
    if progress.cancelled() {
        return Vec::new();
    }

    let mut pattern = Vec::with_capacity(dir.len() + 3);
    pattern.extend_from_slice(dir);
    pattern.extend_from_slice(&[b'\\' as u16, b'*' as u16, 0]);

    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe {
        FindFirstFileExW(
            pattern.as_ptr(),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            std::ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Vec::new();
    }

    let mut out = Vec::new();
    let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);
    loop {
        let name_len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(260);
        let name16 = &data.cFileName[..name_len];
        let dot = name16 == [b'.' as u16] || name16 == [b'.' as u16, b'.' as u16];
        if !dot {
            let f = attr_flags(data.dwFileAttributes);
            let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
            let mtime = filetime_to_unix(
                ((data.ftLastWriteTime.dwHighDateTime as u64) << 32)
                    | data.ftLastWriteTime.dwLowDateTime as u64,
            );
            let (size, alloc) = if f & flags::DIR != 0 {
                dirs += 1;
                (0, 0)
            } else {
                files += 1;
                bytes += size;
                let alloc = if f & flags::OFFLINE != 0 {
                    0
                } else if f & (flags::COMPRESSED | flags::SPARSE) != 0 {
                    compressed_size(dir, name16).unwrap_or(size)
                } else {
                    size.div_ceil(cluster) * cluster
                };
                (size, alloc)
            };
            out.push(Entry {
                name: String::from_utf16_lossy(name16).into_boxed_str(),
                flags: f,
                size,
                alloc,
                mtime,
                children: Vec::new(),
            });
        }
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    progress.add(files, dirs, bytes);

    // Junctions and symlinks are listed but not followed, so nothing is counted twice.
    out.par_iter_mut()
        .filter(|e| e.flags & flags::DIR != 0 && e.flags & flags::REPARSE == 0)
        .for_each(|e| {
            let mut sub = Vec::with_capacity(dir.len() + 1 + e.name.len());
            sub.extend_from_slice(dir);
            sub.push(b'\\' as u16);
            sub.extend(e.name.encode_utf16());
            e.children = list_dir(&sub, cluster, progress);
        });
    out
}

fn compressed_size(dir: &[u16], name: &[u16]) -> Option<u64> {
    let mut path = Vec::with_capacity(dir.len() + name.len() + 2);
    path.extend_from_slice(dir);
    path.push(b'\\' as u16);
    path.extend_from_slice(name);
    path.push(0);
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(path.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE && unsafe { GetLastError() } != NO_ERROR {
        return None;
    }
    Some(((high as u64) << 32) | low as u64)
}
