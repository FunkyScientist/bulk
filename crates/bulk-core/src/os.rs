//! Small Windows pieces shared by the app and the CLI: the time, opening files,
//! showing them in Explorer, and finding the parts of a Bulk install.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::wide::wide;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Unix seconds.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Open a file or folder with its default app.
pub fn open(path: &str) -> anyhow::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let (verb, file) = (wide("open"), wide(path));
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
    // Values above 32 mean success.
    anyhow::ensure!(r as usize > 32, "couldn't open {path}");
    Ok(())
}

/// Show a file or folder selected in an Explorer window.
pub fn reveal(path: &str) -> anyhow::Result<()> {
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.replace('"', "")))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

/// The app's Tauri identifier; its single-instance lock is named after it.
const APP_ID: &str = "io.github.funkyscientist.bulk";

/// Whether the Bulk app is open (in this Windows session).
pub fn app_running() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::OpenMutexW;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let name = wide(format!("{APP_ID}-sim"));
    let h = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if h.is_null() {
        return false;
    }
    unsafe { CloseHandle(h) };
    true
}

/// Folder Bulk is installed in. The CLI lives in its `bin` subfolder, so only it
/// (not `Bulk.exe`) is on PATH; everything else sits beside `Bulk.exe`.
pub fn install_dir() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    match dir.file_name() {
        Some(n) if n.eq_ignore_ascii_case("bin") => {
            dir.parent().map(Path::to_path_buf).unwrap_or(dir)
        }
        _ => dir,
    }
}

/// `Bulk.exe`, if it's installed beside us.
pub fn app_exe() -> Option<PathBuf> {
    let p = install_dir().join("Bulk.exe");
    p.is_file().then_some(p)
}

/// Open `path` in the Bulk app: starts it, or hands the path to the open window.
pub fn open_in_app(path: &str) -> anyhow::Result<()> {
    let exe = app_exe()
        .ok_or_else(|| anyhow::anyhow!("couldn't find Bulk.exe in {}", install_dir().display()))?;
    std::process::Command::new(exe)
        .arg("--open")
        .arg(path)
        .spawn()?;
    Ok(())
}
