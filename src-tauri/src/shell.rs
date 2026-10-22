//! Explorer integration: the "Analyze with Bulk" right-click entry (per-user, no
//! admin needed) and handling the `--open <path>` it launches us with.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};

/// Folder, inside-a-folder background, and drive.
const TARGETS: [&str; 3] = [
    r"Software\Classes\Directory\shell\Bulk",
    r"Software\Classes\Directory\Background\shell\Bulk",
    r"Software\Classes\Drive\shell\Bulk",
];

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn set_value(key: &str, name: Option<&str>, value: &str) -> std::io::Result<()> {
    unsafe {
        let mut h: HKEY = std::ptr::null_mut();
        let k = w(key);
        let r = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut h,
            std::ptr::null_mut(),
        );
        if r != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(r as i32));
        }
        let n = name.map(w);
        let v = w(value);
        let r = RegSetValueExW(
            h,
            n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            v.as_ptr() as *const u8,
            (v.len() * 2) as u32,
        );
        RegCloseKey(h);
        if r != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(r as i32));
        }
    }
    Ok(())
}

pub fn set_explorer_menu(enabled: bool) -> std::io::Result<()> {
    if !enabled {
        for key in TARGETS {
            let k = w(key);
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
        }
        return Ok(());
    }
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    for key in TARGETS {
        set_value(key, Some("MUIVerb"), "Analyze with Bulk")?;
        set_value(key, Some("Icon"), &format!("\"{exe}\",0"))?;
        set_value(
            &format!(r"{key}\command"),
            None,
            &format!("\"{exe}\" --open \"%V\""),
        )?;
    }
    Ok(())
}

/// Path passed with `--open`, waiting for the UI to pick it up.
#[derive(Default)]
pub struct OpenState(pub Mutex<Option<String>>);

pub fn open_arg(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--open")?;
    let p = args.get(i + 1)?.trim_matches('"').to_string();
    // Only absolute drive paths, e.g. C:\Users or D:\.
    let b = p.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then_some(p)
}

/// A second launch (e.g. from Explorer) while Bulk is already open.
pub fn on_second_instance(app: &AppHandle, args: Vec<String>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(p) = open_arg(&args) {
        let _ = app.emit("open-path", p);
    }
}

#[tauri::command]
pub fn take_open_path(state: State<OpenState>) -> Option<String> {
    state.0.lock().unwrap().take()
}
