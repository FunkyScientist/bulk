//! Filesystem scanners. `walk` works anywhere without privileges; `mft` reads the
//! NTFS master file table directly and needs admin rights.

pub mod walk;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::tree::flags;

/// Shared counters a UI can poll while a scan runs. Also carries cancellation.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    /// For MFT scans: records processed and total records, for a progress bar.
    pub done: AtomicU64,
    pub total: AtomicU64,
    cancel: AtomicBool,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    #[inline]
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn add(&self, files: u64, dirs: u64, bytes: u64) {
        self.files.fetch_add(files, Ordering::Relaxed);
        self.dirs.fetch_add(dirs, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

pub(crate) const ATTR_HIDDEN: u32 = 0x2;
pub(crate) const ATTR_SYSTEM: u32 = 0x4;
pub(crate) const ATTR_DIRECTORY: u32 = 0x10;
pub(crate) const ATTR_SPARSE: u32 = 0x200;
pub(crate) const ATTR_REPARSE: u32 = 0x400;
pub(crate) const ATTR_COMPRESSED: u32 = 0x800;
pub(crate) const ATTR_OFFLINE: u32 = 0x1000;
pub(crate) const ATTR_RECALL_ON_OPEN: u32 = 0x40000;
pub(crate) const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x400000;

/// Map Win32 file attributes to node flags.
pub(crate) fn attr_flags(attrs: u32) -> u16 {
    let mut f = 0;
    if attrs & ATTR_DIRECTORY != 0 {
        f |= flags::DIR;
    }
    if attrs & ATTR_HIDDEN != 0 {
        f |= flags::HIDDEN;
    }
    if attrs & ATTR_SYSTEM != 0 {
        f |= flags::SYSTEM;
    }
    if attrs & ATTR_REPARSE != 0 {
        f |= flags::REPARSE;
    }
    if attrs & ATTR_COMPRESSED != 0 {
        f |= flags::COMPRESSED;
    }
    if attrs & ATTR_SPARSE != 0 {
        f |= flags::SPARSE;
    }
    if attrs & (ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS) != 0 {
        f |= flags::OFFLINE;
    }
    f
}

/// FILETIME (100ns ticks since 1601) to unix seconds, clamped to `u32`.
#[inline]
pub(crate) fn filetime_to_unix(ft: u64) -> u32 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    (ft / 10_000_000).saturating_sub(EPOCH_DIFF).min(u32::MAX as u64) as u32
}
