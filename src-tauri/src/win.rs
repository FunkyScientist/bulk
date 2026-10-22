//! Window backdrop. Mica needs Windows 11; elsewhere the UI paints a solid background.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::WebviewWindow;
use tauri::window::{Effect, EffectsBuilder};

static MICA: AtomicBool = AtomicBool::new(false);

pub fn apply_backdrop(window: &WebviewWindow) {
    if windows_build() >= 22000
        && window
            .set_effects(EffectsBuilder::new().effect(Effect::Mica).build())
            .is_ok()
    {
        MICA.store(true, Ordering::Relaxed);
    }
}

pub fn has_mica() -> bool {
    MICA.load(Ordering::Relaxed)
}

fn windows_build() -> u32 {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    unsafe { RtlGetVersion(&mut info) };
    info.dwBuildNumber
}
