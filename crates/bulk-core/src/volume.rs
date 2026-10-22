//! Drive discovery.

use serde::Serialize;
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    GetVolumeNameForVolumeMountPointW,
};

use crate::wide::{from_wide, wide};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    /// Mounted by a driver rather than backed by a disk volume, e.g. Google Drive.
    Virtual,
    Optical,
    Ram,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    /// `C`
    pub letter: char,
    /// `C:\`
    pub root: String,
    pub label: String,
    /// `NTFS`, `exFAT`, `FAT32`, ...
    pub fs: String,
    pub serial: u32,
    pub kind: DriveKind,
    pub total: u64,
    pub free: u64,
    pub cluster: u32,
}

impl Volume {
    pub fn is_ntfs(&self) -> bool {
        self.fs.eq_ignore_ascii_case("NTFS")
    }

    /// Whether Bulk scans this drive automatically.
    pub fn auto_scan(&self) -> bool {
        matches!(self.kind, DriveKind::Fixed | DriveKind::Removable)
    }

    /// `\\.\C:` for raw volume access.
    pub fn device_path(&self) -> String {
        format!("\\\\.\\{}:", self.letter)
    }
}

pub fn list() -> Vec<Volume> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| query((b'A' + i) as char))
        .collect()
}

pub fn query(letter: char) -> Option<Volume> {
    let letter = letter.to_ascii_uppercase();
    let root = format!("{letter}:\\");
    let wroot = wide(&root);
    let kind = match unsafe { GetDriveTypeW(wroot.as_ptr()) } {
        DRIVE_FIXED => DriveKind::Fixed,
        DRIVE_REMOVABLE => DriveKind::Removable,
        DRIVE_REMOTE => DriveKind::Network,
        DRIVE_CDROM => DriveKind::Optical,
        DRIVE_RAMDISK => DriveKind::Ram,
        1 => return None, // no root dir
        _ => DriveKind::Unknown,
    };

    let mut label = [0u16; 261];
    let mut fs = [0u16; 261];
    let mut serial = 0u32;
    let ok = unsafe {
        GetVolumeInformationW(
            wroot.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        // Empty card reader / optical drive.
        return None;
    }

    let kind = if matches!(kind, DriveKind::Fixed | DriveKind::Removable) && !is_disk_volume(&wroot)
    {
        DriveKind::Virtual
    } else {
        kind
    };

    let (mut total, mut free) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(wroot.as_ptr(), std::ptr::null_mut(), &mut total, &mut free) };

    let (mut spc, mut bps, mut _f, mut _t) = (0u32, 0u32, 0u32, 0u32);
    let cluster = if unsafe {
        GetDiskFreeSpaceW(wroot.as_ptr(), &mut spc, &mut bps, &mut _f, &mut _t)
    } != 0
    {
        spc * bps
    } else {
        4096
    };

    Some(Volume {
        letter,
        root,
        label: from_wide(&label),
        fs: from_wide(&fs),
        serial,
        kind,
        total,
        free,
        cluster,
    })
}

/// Real disk volumes have a `\\?\Volume{GUID}\` name; driver-mounted drives don't.
fn is_disk_volume(wroot: &[u16]) -> bool {
    let mut name = [0u16; 64];
    unsafe {
        GetVolumeNameForVolumeMountPointW(wroot.as_ptr(), name.as_mut_ptr(), name.len() as u32) != 0
    }
}
