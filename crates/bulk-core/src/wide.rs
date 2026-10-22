use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// Null-terminated UTF-16 copy of `s` for passing to Win32.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Decode a UTF-16 buffer up to its first null.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
