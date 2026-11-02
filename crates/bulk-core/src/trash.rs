//! Deleting files: to the Recycle Bin (undoable) or permanently, and restoring
//! recycled items without going through Explorer.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NO_UI, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
    FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW, SHFileOperationW,
};

use crate::wide::wide;

/// Move `path` to the Recycle Bin. If Windows can't recycle it (too big for the
/// bin), it asks the user before deleting permanently instead of doing so silently.
pub fn recycle(path: &Path) -> anyhow::Result<()> {
    shell_delete(
        path,
        (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING)
            as u16,
    )
}

/// Delete `path` for good, folders included.
pub fn delete_permanently(path: &Path) -> anyhow::Result<()> {
    shell_delete(path, FOF_NO_UI as u16)
}

fn shell_delete(path: &Path, flags: u16) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.is_absolute() && path.parent().is_some(),
        "refusing to delete {path:?}"
    );
    // pFrom is a list of paths, each null-terminated, ending with an extra null.
    let mut from = wide(path);
    from.push(0);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = flags;
    let code = unsafe { SHFileOperationW(&mut op) };
    anyhow::ensure!(
        code == 0,
        "couldn't delete {} (error {code:#x})",
        path.display()
    );
    anyhow::ensure!(op.fAnyOperationsAborted == 0, "cancelled");
    anyhow::ensure!(!path.exists(), "{} is still there", path.display());
    Ok(())
}

/// One item in a Recycle Bin: `$I…` holds the metadata, `$R…` the data.
#[derive(Debug, Clone)]
pub struct Recycled {
    pub info: PathBuf,
    pub data: PathBuf,
    pub original: PathBuf,
    pub deleted: SystemTime,
}

/// Recycle Bin entries on `path`'s drive whose original location is `path` and
/// that were deleted at or after `since`.
pub fn find_recycled(path: &Path, since: SystemTime) -> Vec<Recycled> {
    let Some(drive) = path.to_str().and_then(|s| s.get(..2)) else {
        return Vec::new();
    };
    let Ok(sid) = crate::helper::current_user_sid() else {
        return Vec::new();
    };
    let bin = PathBuf::from(format!("{drive}\\$Recycle.Bin\\{sid}"));
    let Ok(dir) = std::fs::read_dir(&bin) else {
        return Vec::new();
    };
    let want = long_path(path).to_string_lossy().to_lowercase();
    dir.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let suffix = name.strip_prefix("$I")?;
            let (original, deleted) = parse_info(&std::fs::read(e.path()).ok()?)?;
            (original.to_string_lossy().to_lowercase() == want && deleted >= since).then(|| {
                Recycled {
                    info: e.path(),
                    data: bin.join(format!("$R{suffix}")),
                    original,
                    deleted,
                }
            })
        })
        .collect()
}

/// The Recycle Bin records long names, so expand any 8.3 short names (`HANANS~1`)
/// in the parent folder, which still exists after the item was deleted.
fn long_path(path: &Path) -> PathBuf {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    let w = wide(parent);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetLongPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n > buf.len() {
        return path.to_path_buf();
    }
    PathBuf::from(String::from_utf16_lossy(&buf[..n])).join(name)
}

/// Parse a `$I` file (format 1 or 2) into (original path, deletion time).
fn parse_info(b: &[u8]) -> Option<(PathBuf, SystemTime)> {
    let version = u64::from_le_bytes(b.get(0..8)?.try_into().ok()?);
    let ft = u64::from_le_bytes(b.get(16..24)?.try_into().ok()?);
    let units: Vec<u16> = match version {
        1 => b
            .get(24..24 + 520)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect(),
        2 => {
            let n = u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize;
            b.get(28..28 + n * 2)?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect()
        }
        _ => return None,
    };
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    let path = PathBuf::from(String::from_utf16_lossy(&units[..end]));
    // FILETIME is 100ns ticks since 1601.
    let unix_100ns = ft.checked_sub(116_444_736_000_000_000)?;
    let deleted = SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(unix_100ns * 100);
    Some((path, deleted))
}

/// Put a recycled item back where it came from.
pub fn restore(item: &Recycled) -> anyhow::Result<()> {
    anyhow::ensure!(
        !item.original.exists(),
        "{} already exists",
        item.original.display()
    );
    if let Some(parent) = item.original.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&item.data, &item.original)?;
    let _ = std::fs::remove_file(&item.info);
    Ok(())
}

/// Why `path` mustn't be deleted from the command line, if it mustn't: Windows
/// itself, installed programs, a drive or a profile's top-level folders, and
/// Bulk's own data. `path` should be absolute.
pub fn protected(path: &Path) -> Option<&'static str> {
    let lower = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches('\\')
            .to_ascii_lowercase()
            .replace('/', "\\")
    };
    let p = lower(path);
    let is_or_in = |dir: &str| p == dir || p.starts_with(&format!("{dir}\\"));
    let env = |k: &str| std::env::var_os(k).map(|v| lower(Path::new(&v)));

    if p.len() <= 2 {
        return Some("a whole drive");
    }
    for (var, why) in [
        ("SystemRoot", "part of Windows"),
        ("ProgramFiles", "an installed program"),
        ("ProgramFiles(x86)", "an installed program"),
        ("ProgramW6432", "an installed program"),
        ("ProgramData", "shared program data"),
    ] {
        if env(var).is_some_and(|d| is_or_in(&d)) {
            return Some(why);
        }
    }
    let local = env("LOCALAPPDATA");
    if local
        .as_ref()
        .is_some_and(|l| is_or_in(&format!("{l}\\bulk")))
    {
        return Some("Bulk's own data");
    }
    if is_or_in(&lower(&crate::os::install_dir())) {
        return Some("part of Bulk");
    }

    // Folders that hold everything else: the folder itself is protected, what's in
    // it isn't.
    let name = p.rsplit('\\').next().unwrap_or("");
    let parent = p.rsplit_once('\\').map_or("", |(a, _)| a);
    if parent.len() <= 2
        && (name.starts_with('$')
            || matches!(
                name,
                "users"
                    | "system volume information"
                    | "recovery"
                    | "boot"
                    | "pagefile.sys"
                    | "hiberfil.sys"
                    | "swapfile.sys"
                    | "bootmgr"
            ))
    {
        return Some("needed by Windows");
    }
    if let Some(profile) = env("USERPROFILE") {
        let users = profile.rsplit_once('\\').map_or("", |(a, _)| a);
        if parent == users {
            return Some("a user's profile folder");
        }
        const TOP: &[&str] = &[
            "appdata",
            "appdata\\local",
            "appdata\\locallow",
            "appdata\\roaming",
            "desktop",
            "documents",
            "downloads",
            "music",
            "pictures",
            "videos",
            "onedrive",
        ];
        if let Some(rest) = p.strip_prefix(&format!("{profile}\\"))
            && TOP.contains(&rest)
        {
            return Some("one of your profile's main folders");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycle_and_restore_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bulk-trash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("victim.txt");
        std::fs::write(&file, b"hello").unwrap();
        let before = SystemTime::now() - std::time::Duration::from_secs(2);

        recycle(&file).unwrap();
        assert!(!file.exists());
        let found = find_recycled(&file, before);
        assert_eq!(found.len(), 1, "{found:?}");
        restore(&found[0]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"hello");

        delete_permanently(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn protected_paths() {
        let profile = std::env::var("USERPROFILE").unwrap();
        let windows = std::env::var("SystemRoot").unwrap();
        for p in [
            "C:\\".to_string(),
            "C:".to_string(),
            format!("{windows}\\System32"),
            "C:\\Users".to_string(),
            profile.clone(),
            format!("{profile}\\Downloads"),
            format!("{profile}\\AppData\\Local"),
            "C:\\$Recycle.Bin".to_string(),
            "C:\\pagefile.sys".to_string(),
        ] {
            assert!(
                protected(Path::new(&p)).is_some(),
                "{p} should be protected"
            );
        }
        for p in [
            format!("{profile}\\Downloads\\setup.exe"),
            format!("{profile}\\AppData\\Local\\Temp"),
            "D:\\Games\\old".to_string(),
            format!("{profile}\\projects\\app\\node_modules"),
        ] {
            assert!(
                protected(Path::new(&p)).is_none(),
                "{p} shouldn't be protected"
            );
        }
    }
}
