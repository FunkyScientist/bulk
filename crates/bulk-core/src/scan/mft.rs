//! NTFS master file table reader. Reads `$MFT` straight off the raw volume in large
//! parallel chunks, parses every file record, then links records into a tree by
//! their parent references. Needs admin rights to open `\\.\X:`.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::sync::Mutex;

use rayon::prelude::*;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
};
use windows_sys::Win32::System::IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};
use windows_sys::Win32::System::Threading::CreateEventW;

use super::*;
use crate::tree::{NONE, NodeId, Tree, flags};
use crate::wide::wide;

const ROOT_RECORD: u32 = 5;
/// $BadClus: its $Bad stream is a sparse placeholder as large as the volume.
const BADCLUS_RECORD: u32 = 8;
const FIRST_USER_RECORD: u32 = 16;
const CHUNK: u64 = 4 << 20;

const AT_STANDARD_INFORMATION: u32 = 0x10;
const AT_FILE_NAME: u32 = 0x30;
const AT_DATA: u32 = 0x80;
const AT_REPARSE_POINT: u32 = 0xC0;
const AT_END: u32 = 0xFFFF_FFFF;

const NS_DOS: u8 = 2;

/// Read-only handle to a raw NTFS volume.
pub struct Volume {
    handle: HANDLE,
    pub cluster: u64,
    pub record_size: u64,
    pub sector: u64,
    pub mft_start_lcn: u64,
    pub mft_valid_len: u64,
}

unsafe impl Send for Volume {}
unsafe impl Sync for Volume {}

impl Drop for Volume {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

impl Volume {
    /// Open `\\.\C:` for raw reads. Fails without admin rights.
    pub fn open(letter: char) -> anyhow::Result<Volume> {
        let path = wide(format!("\\\\.\\{}:", letter.to_ascii_uppercase()));
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }

        let mut data: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
        let mut ret = 0u32;
        let ok = unsafe {
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            let r = DeviceIoControl(
                handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                std::ptr::null(),
                0,
                &mut data as *mut _ as *mut _,
                size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                &mut ret,
                &mut ov,
            );
            let ok = r != 0 || GetOverlappedResult(handle, &ov, &mut ret, 1) != 0;
            CloseHandle(ov.hEvent);
            ok
        };
        if !ok {
            let err = std::io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            anyhow::bail!("not an NTFS volume: {err}");
        }

        Ok(Volume {
            handle,
            cluster: data.BytesPerCluster as u64,
            record_size: data.BytesPerFileRecordSegment as u64,
            sector: data.BytesPerSector as u64,
            mft_start_lcn: data.MftStartLcn as u64,
            mft_valid_len: data.MftValidDataLength as u64,
        })
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }

    /// Read `len` bytes at byte `offset`. Both must be sector-aligned.
    fn read(&self, offset: u64, buf: &mut AlignedBuf) -> anyhow::Result<()> {
        let len = buf.len as u32;
        unsafe {
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.Anonymous.Anonymous.Offset = offset as u32;
            ov.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
            ov.hEvent = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            let mut got = 0u32;
            let ok = ReadFile(self.handle, buf.ptr, len, std::ptr::null_mut(), &mut ov) != 0
                || GetOverlappedResult(self.handle, &ov, &mut got, 1) != 0;
            CloseHandle(ov.hEvent);
            if !ok {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(())
    }

    /// Byte ranges of the $MFT data stream on disk, as (disk offset, length, first
    /// record index).
    fn mft_extents(&self) -> anyhow::Result<Vec<(u64, u64, u64)>> {
        let mut buf = AlignedBuf::new(self.record_size.max(self.sector) as usize);
        self.read(self.mft_start_lcn * self.cluster, &mut buf)?;
        let rec = buf.as_mut_slice();
        anyhow::ensure!(apply_fixup(rec, self.sector as usize), "bad $MFT record 0");

        let mut runs = None;
        for a in Attrs::new(rec) {
            if a.kind == AT_DATA && a.nonresident && a.name_len == 0 && a.start_vcn() == 0 {
                runs = Some(decode_runs(a.runlist()));
            }
        }
        let runs = runs.ok_or_else(|| anyhow::anyhow!("$MFT has no data runs"))?;

        let total_bytes = self.mft_valid_len;
        let mut out = Vec::new();
        let mut vcn_bytes = 0u64;
        for (lcn, clusters) in runs {
            let len = clusters * self.cluster;
            if let Some(lcn) = lcn {
                let take = len.min(total_bytes.saturating_sub(vcn_bytes));
                if take > 0 {
                    out.push((lcn * self.cluster, take, vcn_bytes / self.record_size));
                }
            }
            vcn_bytes += len;
            if vcn_bytes >= total_bytes {
                break;
            }
        }
        Ok(out)
    }
}

/// Page-aligned heap buffer, required for unbuffered volume reads.
struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
    layout: Layout,
}

unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    fn new(len: usize) -> AlignedBuf {
        let len = len.div_ceil(4096) * 4096;
        let layout = Layout::from_size_align(len, 4096).unwrap();
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "out of memory");
        AlignedBuf { ptr, len, layout }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

/// One in-use record's contribution. Extension records carry `base` and get folded
/// into their base record.
#[derive(Clone, Copy)]
struct Rec {
    idx: u32,
    base: u32,
    seq: u16,
    flags: u16,
    mtime: u32,
    size: u64,
    alloc: u64,
}

#[derive(Clone, Copy)]
struct Name {
    rec: u32,
    parent: u32,
    parent_seq: u16,
    ns: u8,
    off: u32,
    len: u16,
}

#[derive(Default)]
struct Parsed {
    first_rec: u32,
    recs: Vec<Rec>,
    names: Vec<Name>,
    text: String,
}

#[derive(Clone, Copy, Default)]
struct Merged {
    in_use: bool,
    has_long_name: bool,
    seq: u16,
    flags: u16,
    mtime: u32,
    size: u64,
    alloc: u64,
}

/// Scan an NTFS drive by reading its MFT.
pub fn scan(letter: char, progress: &Progress) -> anyhow::Result<Tree> {
    let vol = Volume::open(letter)?;
    scan_volume(&vol, letter, progress)
}

pub fn scan_volume(vol: &Volume, letter: char, progress: &Progress) -> anyhow::Result<Tree> {
    let rs = vol.record_size;
    let total_records = vol.mft_valid_len / rs;
    progress.total.store(total_records, Ordering::Relaxed);

    // Split every extent into fixed-size chunks so reads and parsing overlap across
    // the thread pool.
    let mut chunks = Vec::new();
    for (disk, len, first_rec) in vol.mft_extents()? {
        let mut off = 0;
        while off < len {
            let n = CHUNK.min(len - off);
            chunks.push((disk + off, n, first_rec + off / rs));
            off += n;
        }
    }

    let results: Mutex<Vec<Parsed>> = Mutex::new(Vec::with_capacity(chunks.len()));
    let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    chunks.par_iter().for_each(|&(disk, len, first_rec)| {
        if progress.cancelled() {
            return;
        }
        let mut buf = AlignedBuf::new(len as usize);
        if let Err(e) = vol.read(disk, &mut buf) {
            *error.lock().unwrap() = Some(e);
            progress.cancel();
            return;
        }
        let p = parse_chunk(
            &mut buf.as_mut_slice()[..len as usize],
            first_rec as u32,
            rs as usize,
            vol.sector as usize,
            progress,
        );
        results.lock().unwrap().push(p);
    });
    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }
    if progress.cancelled() {
        anyhow::bail!("scan cancelled");
    }
    // Chunks finish in any order; sort so hard-link attribution is deterministic.
    let mut results = results.into_inner().unwrap();
    results.sort_unstable_by_key(|p| p.first_rec);

    Ok(build_tree(results, total_records as usize, letter))
}

fn parse_chunk(
    buf: &mut [u8],
    first_rec: u32,
    rs: usize,
    sector: usize,
    progress: &Progress,
) -> Parsed {
    let mut out = Parsed {
        first_rec,
        ..Parsed::default()
    };
    let count = buf.len() / rs;
    out.recs.reserve(count);
    out.names.reserve(count);
    out.text.reserve(count * 12);
    let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);

    for i in 0..count {
        let rec = &mut buf[i * rs..(i + 1) * rs];
        let idx = first_rec + i as u32;
        let Some(r) = parse_record(rec, idx, sector, true, &mut out.text, &mut out.names) else {
            continue;
        };
        if r.base == NONE {
            if r.flags & flags::DIR != 0 {
                dirs += 1;
            } else {
                files += 1;
            }
        }
        bytes += r.size;
        out.recs.push(r);
    }

    progress.add(files, dirs, bytes);
    progress.done.fetch_add(count as u64, Ordering::Relaxed);
    out
}

/// Parse one file record. Names are appended to `text`/`names`. Returns `None` for
/// unused or damaged records.
fn parse_record(
    rec: &mut [u8],
    idx: u32,
    sector: usize,
    fixup: bool,
    text: &mut String,
    names: &mut Vec<Name>,
) -> Option<Rec> {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return None;
    }
    let rflags = u16le(rec, 22);
    if rflags & 1 == 0 || (fixup && !apply_fixup(rec, sector)) {
        return None; // not in use, or torn
    }
    let seq = u16le(rec, 16);
    let base_ref = u64le(rec, 32);
    let base = if base_ref & 0xFFFF_FFFF_FFFF == 0 {
        NONE
    } else {
        base_ref as u32
    };
    let owner = if base == NONE { idx } else { base };
    let is_dir = rflags & 2 != 0;

    let mut r = Rec {
        idx,
        base,
        seq,
        flags: if is_dir { flags::DIR } else { 0 },
        mtime: 0,
        size: 0,
        alloc: 0,
    };

    for a in Attrs::new(rec) {
        match a.kind {
            AT_STANDARD_INFORMATION if !a.nonresident => {
                let v = a.value();
                if v.len() >= 0x24 {
                    r.mtime = filetime_to_unix(u64le(v, 8));
                    let attrs = u32le(v, 0x20);
                    // NTFS keeps extra internal bits here, so only trust the
                    // plain DOS-style ones.
                    r.flags |= attr_flags(attrs & 0x0FFF) & !(flags::DIR | flags::OFFLINE);
                }
            }
            AT_FILE_NAME if !a.nonresident => {
                let v = a.value();
                if v.len() < 66 {
                    continue;
                }
                let parent_ref = u64le(v, 0);
                let nlen = v[64] as usize;
                let ns = v[65];
                if v.len() < 66 + nlen * 2 {
                    continue;
                }
                let off = text.len() as u32;
                let units = (0..nlen).map(|k| u16le(v, 66 + k * 2));
                for c in char::decode_utf16(units) {
                    text.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                let len = (text.len() as u32 - off).min(u16::MAX as u32) as u16;
                names.push(Name {
                    rec: owner,
                    parent: parent_ref as u32,
                    parent_seq: (parent_ref >> 48) as u16,
                    ns,
                    off,
                    len,
                });
            }
            AT_DATA => {
                if a.nonresident {
                    if a.start_vcn() == 0 {
                        r.size += a.real_size();
                        r.alloc += a.on_disk_size();
                    }
                } else {
                    r.size += a.value_len() as u64;
                }
            }
            AT_REPARSE_POINT => {
                r.flags |= flags::REPARSE;
                // IO_REPARSE_TAG_CLOUD_*: a OneDrive-style placeholder.
                let tag = if a.nonresident {
                    0
                } else {
                    u32le(a.value(), 0)
                };
                if tag & 0xFFFF_0FFF == 0x9000_001A {
                    r.flags |= flags::OFFLINE;
                }
            }
            AT_END => break,
            _ => {}
        }
    }

    if owner == BADCLUS_RECORD {
        (r.size, r.alloc) = (0, 0);
    }
    if base == NONE && idx < FIRST_USER_RECORD {
        r.flags |= flags::META;
    }
    Some(r)
}

/// Current state of one file, read straight from its MFT record.
#[derive(Clone, Debug, PartialEq)]
pub struct FileState {
    /// Full reference (sequence << 48 | record).
    pub frn: u64,
    pub flags: u16,
    pub size: u64,
    pub alloc: u64,
    pub mtime: u32,
    /// (parent reference, name) for each hard link, DOS short names dropped.
    pub names: Vec<(u64, String)>,
}

impl Volume {
    pub(crate) fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> std::io::Result<u32> {
        let mut ret = 0u32;
        unsafe {
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            let ok = DeviceIoControl(
                self.handle,
                code,
                input.as_ptr() as *const _,
                input.len() as u32,
                output.as_mut_ptr() as *mut _,
                output.len() as u32,
                &mut ret,
                &mut ov,
            ) != 0
                || GetOverlappedResult(self.handle, &ov, &mut ret, 1) != 0;
            let err = std::io::Error::last_os_error();
            CloseHandle(ov.hEvent);
            if !ok {
                return Err(err);
            }
        }
        Ok(ret)
    }

    /// Read a file's current state by reference. Returns `None` if the record is no
    /// longer in use or now belongs to a different file.
    pub fn file_state(&self, frn: u64) -> anyhow::Result<Option<FileState>> {
        const FSCTL_GET_NTFS_FILE_RECORD: u32 = 0x0009_0068;
        let mut out = vec![0u8; 12 + self.record_size as usize];
        self.ioctl(FSCTL_GET_NTFS_FILE_RECORD, &frn.to_le_bytes(), &mut out)?;
        // Windows returns the nearest in-use record at or below the one asked for.
        let got = u64le(&out, 0);
        if got & 0xFFFF_FFFF_FFFF != frn & 0xFFFF_FFFF_FFFF {
            return Ok(None);
        }
        let len = (u32le(&out, 8) as usize).min(out.len() - 12);
        let rec = &mut out[12..12 + len];
        let idx = (frn & 0xFFFF_FFFF_FFFF) as u32;
        // Records from this call may already have their fixups applied; only undo
        // them if the sector ends still hold the check value.
        let sector = self.sector as usize;
        let usa = u16le(rec, 4) as usize;
        let fixup = rec.len() >= sector
            && usa + 2 <= rec.len()
            && rec[sector - 2..sector] == rec[usa..usa + 2];
        let (mut text, mut names) = (String::new(), Vec::new());
        let Some(r) = parse_record(rec, idx, sector, fixup, &mut text, &mut names) else {
            return Ok(None);
        };
        if r.base != NONE || r.seq != (frn >> 48) as u16 {
            return Ok(None);
        }
        let has_long = names.iter().any(|n| n.ns != NS_DOS);
        Ok(Some(FileState {
            frn,
            flags: r.flags,
            size: if r.flags & flags::DIR != 0 { 0 } else { r.size },
            alloc: if r.flags & flags::DIR != 0 {
                0
            } else {
                r.alloc
            },
            mtime: r.mtime,
            names: names
                .iter()
                .filter(|n| !(has_long && n.ns == NS_DOS))
                .map(|n| {
                    let parent = ((n.parent_seq as u64) << 48) | n.parent as u64;
                    let name = text[n.off as usize..n.off as usize + n.len as usize].to_string();
                    (parent, name)
                })
                .collect(),
        }))
    }
}

fn build_tree(results: Vec<Parsed>, total: usize, letter: char) -> Tree {
    let mut info = vec![Merged::default(); total.max(ROOT_RECORD as usize + 1)];
    let mut name_count = 0;
    let mut text_len = 0;

    for p in &results {
        name_count += p.names.len();
        text_len += p.text.len();
        for r in &p.recs {
            let owner = if r.base == NONE { r.idx } else { r.base } as usize;
            if owner >= info.len() {
                continue;
            }
            let m = &mut info[owner];
            if r.base == NONE {
                m.in_use = true;
                m.seq = r.seq;
                m.mtime = r.mtime;
            }
            m.flags |= r.flags;
            m.size += r.size;
            m.alloc += r.alloc;
        }
        for n in &p.names {
            if n.ns != NS_DOS && (n.rec as usize) < info.len() {
                info[n.rec as usize].has_long_name = true;
            }
        }
    }

    let mut tree = Tree::with_capacity(
        &format!("{}:", letter.to_ascii_uppercase()),
        name_count,
        text_len,
    );
    let root = tree.root;
    let mut node_of = vec![NONE; info.len()];
    node_of[ROOT_RECORD as usize] = root;
    {
        let r = info[ROOT_RECORD as usize];
        let n = tree.get_mut(root);
        n.mtime = r.mtime;
        n.frn = ((r.seq as u64) << 48) | ROOT_RECORD as u64;
    }

    // Pass 1: one node per name. A record's first name is its primary node; any
    // further names are hard links.
    let mut pending: Vec<(NodeId, u32, u16)> = Vec::with_capacity(name_count);
    for p in &results {
        for n in &p.names {
            let rec = n.rec as usize;
            if rec >= info.len() || rec == ROOT_RECORD as usize {
                continue;
            }
            let m = info[rec];
            if !m.in_use || (n.ns == NS_DOS && m.has_long_name) {
                continue;
            }
            let name = &p.text[n.off as usize..n.off as usize + n.len as usize];
            let mut f = m.flags;
            let (size, alloc) = if f & flags::DIR != 0 {
                (0, 0)
            } else {
                (m.size, m.alloc)
            };
            if node_of[rec] != NONE {
                f |= flags::EXTRA_LINK;
            }
            let frn = ((m.seq as u64) << 48) | rec as u64;
            let id = tree.push_node(name, f, size, alloc, m.mtime, frn);
            if node_of[rec] == NONE {
                node_of[rec] = id;
            }
            pending.push((id, n.parent, n.parent_seq));
        }
    }

    // Pass 2: attach to parents. Anything whose parent is gone lands on the root.
    for (id, parent, parent_seq) in pending {
        let p = parent as usize;
        let target = if p < info.len()
            && node_of[p] != NONE
            && info[p].flags & flags::DIR != 0
            && (parent_seq == 0 || info[p].seq == parent_seq)
            && node_of[p] != id
        {
            node_of[p]
        } else {
            root
        };
        tree.link(id, target);
    }

    tree.aggregate();
    tree
}

/// Replace the update sequence array placeholders at the end of each sector.
/// Returns false if the record is torn.
fn apply_fixup(rec: &mut [u8], sector: usize) -> bool {
    let usa_off = u16le(rec, 4) as usize;
    let usa_count = u16le(rec, 6) as usize;
    if usa_count == 0 || usa_off + usa_count * 2 > rec.len() {
        return false;
    }
    let check = [rec[usa_off], rec[usa_off + 1]];
    for i in 1..usa_count {
        let end = i * sector;
        if end > rec.len() {
            break;
        }
        if rec[end - 2..end] != check {
            return false;
        }
        rec[end - 2] = rec[usa_off + i * 2];
        rec[end - 1] = rec[usa_off + i * 2 + 1];
    }
    true
}

/// Decode a non-resident attribute's mapping pairs into (lcn, cluster count).
/// Sparse runs have no lcn.
fn decode_runs(mut b: &[u8]) -> Vec<(Option<u64>, u64)> {
    let mut out = Vec::new();
    let mut lcn: i64 = 0;
    while let Some(&h) = b.first() {
        if h == 0 {
            break;
        }
        let (ls, os) = ((h & 0xF) as usize, (h >> 4) as usize);
        if ls == 0 || b.len() < 1 + ls + os || ls > 8 || os > 8 {
            break;
        }
        let mut len = 0u64;
        for i in 0..ls {
            len |= (b[1 + i] as u64) << (i * 8);
        }
        if os == 0 {
            out.push((None, len));
        } else {
            let mut off = 0i64;
            for i in 0..os {
                off |= (b[1 + ls + i] as i64) << (i * 8);
            }
            let shift = 64 - os * 8;
            off = (off << shift) >> shift; // sign extend
            lcn += off;
            out.push((Some(lcn as u64), len));
        }
        b = &b[1 + ls + os..];
    }
    out
}

struct Attr<'a> {
    kind: u32,
    nonresident: bool,
    name_len: u8,
    flags: u16,
    raw: &'a [u8],
}

impl<'a> Attr<'a> {
    fn value_len(&self) -> u32 {
        u32le(self.raw, 16)
    }
    fn value(&self) -> &'a [u8] {
        let off = u16le(self.raw, 20) as usize;
        let len = self.value_len() as usize;
        self.raw.get(off..off + len).unwrap_or(&[])
    }
    fn start_vcn(&self) -> u64 {
        u64le(self.raw, 16)
    }
    fn runlist(&self) -> &'a [u8] {
        let off = u16le(self.raw, 32) as usize;
        self.raw.get(off..).unwrap_or(&[])
    }
    fn real_size(&self) -> u64 {
        u64le(self.raw, 48)
    }
    fn on_disk_size(&self) -> u64 {
        // Compressed and sparse streams record what's actually allocated separately.
        if self.flags & (0x0001 | 0x8000) != 0 && self.raw.len() >= 72 {
            u64le(self.raw, 64)
        } else {
            u64le(self.raw, 40)
        }
    }
}

struct Attrs<'a> {
    rec: &'a [u8],
    pos: usize,
}

impl<'a> Attrs<'a> {
    fn new(rec: &'a [u8]) -> Attrs<'a> {
        let first = u16le(rec, 20) as usize;
        let used = (u32le(rec, 24) as usize).min(rec.len());
        Attrs {
            rec: &rec[..used],
            pos: first,
        }
    }
}

impl<'a> Iterator for Attrs<'a> {
    type Item = Attr<'a>;
    fn next(&mut self) -> Option<Attr<'a>> {
        if self.pos + 16 > self.rec.len() {
            return None;
        }
        let kind = u32le(self.rec, self.pos);
        if kind == AT_END {
            return None;
        }
        let len = u32le(self.rec, self.pos + 4) as usize;
        if len < 16 || self.pos + len > self.rec.len() {
            return None;
        }
        let raw = &self.rec[self.pos..self.pos + len];
        self.pos += len;
        Some(Attr {
            kind,
            nonresident: raw[8] != 0,
            name_len: raw[9],
            flags: u16le(raw, 12),
            raw,
        })
    }
}

#[inline]
fn u16le(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
#[inline]
fn u32le(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4)
        .map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}
#[inline]
fn u64le(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8)
        .map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runlist_decoding() {
        // 0x21: 1-byte length, 2-byte offset. Second run goes backwards; third is sparse.
        let runs = [
            0x21, 0x10, 0x00, 0x01, 0x21, 0x08, 0x00, 0xFF, 0x01, 0x04, 0x00,
        ];
        let r = decode_runs(&runs);
        assert_eq!(
            r,
            vec![(Some(0x100), 0x10), (Some(0x100 - 0x100), 0x08), (None, 4)]
        );
    }

    #[test]
    fn fixup_detects_torn_records() {
        let mut rec = vec![0u8; 1024];
        rec[4] = 48; // usa offset
        rec[6] = 3; // usa count: check value + 2 sectors
        rec[48..50].copy_from_slice(&[0xAB, 0xCD]);
        rec[50..52].copy_from_slice(&[1, 2]);
        rec[52..54].copy_from_slice(&[3, 4]);
        rec[510..512].copy_from_slice(&[0xAB, 0xCD]);
        rec[1022..1024].copy_from_slice(&[0xAB, 0xCD]);
        let mut good = rec.clone();
        assert!(apply_fixup(&mut good, 512));
        assert_eq!(&good[510..512], &[1, 2]);
        assert_eq!(&good[1022..1024], &[3, 4]);
        rec[1022] = 0;
        assert!(!apply_fixup(&mut rec, 512));
    }
}
