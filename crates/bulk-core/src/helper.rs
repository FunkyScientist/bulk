//! Protocol and client for `bulk-helper.exe`, the elevated process that reads raw
//! NTFS volumes on behalf of the (non-admin) app.
//!
//! Each connection carries a single request. The helper answers with a stream of
//! frames: JSON events (`FRAME_EVENT`) and binary payloads (`FRAME_TREE`). Dropping
//! the connection cancels whatever the helper was doing for it.
//!
//! The helper is started through a Task Scheduler task registered once with a UAC
//! prompt (`install_elevated`), so later launches need no prompt.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::tree::Tree;
use crate::usn::{Change, Journal};
use crate::wire;

pub const PROTOCOL: u32 = 2;
pub const TASK_NAME: &str = r"Bulk\Helper";
pub const FRAME_EVENT: u8 = 0;
pub const FRAME_TREE: u8 = 1;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Limit for JSON frames. Tree frames are streamed and only bounded by u32.
const MAX_EVENT: u32 = 16 << 20;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    Hello,
    /// Full MFT scan of a drive. Answered with `Journal`, progress events, a tree
    /// frame, then `Done`.
    Scan {
        letter: char,
    },
    /// Everything that changed since `from`, as `Changes` batches, then `Done`.
    /// `Gap` if the journal no longer goes back that far.
    Changes {
        letter: char,
        journal: u64,
        from: i64,
    },
    /// Like `Changes`, but keeps the connection open and sends new batches as
    /// files change (an empty batch every couple of seconds as a heartbeat).
    Watch {
        letter: char,
        journal: u64,
        from: i64,
    },
    /// Exit, e.g. so the exe can be replaced by an update.
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Hello {
        protocol: u32,
        version: String,
        exe: String,
    },
    Progress(ProgressInfo),
    /// Journal position at the start of a scan.
    Journal(Journal),
    Changes {
        changes: Vec<Change>,
        next: i64,
    },
    Gap,
    Error {
        message: String,
    },
    Done {
        elapsed_ms: u64,
    },
}

/// Outcome of a journal catch-up.
pub enum CatchUp {
    Changes(Vec<Change>, i64),
    /// The journal can't cover the gap; rescan instead.
    Gap,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProgressInfo {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub done: u64,
    pub total: u64,
}

impl ProgressInfo {
    pub fn of(p: &crate::scan::Progress) -> ProgressInfo {
        use std::sync::atomic::Ordering::Relaxed;
        ProgressInfo {
            files: p.files.load(Relaxed),
            dirs: p.dirs.load(Relaxed),
            bytes: p.bytes.load(Relaxed),
            done: p.done.load(Relaxed),
            total: p.total.load(Relaxed),
        }
    }
}

pub fn write_frame_header(w: &mut impl Write, kind: u8, len: u64) -> std::io::Result<()> {
    let len = u32::try_from(len).map_err(|_| std::io::Error::other("frame too large"))?;
    let mut head = [0u8; 5];
    head[0] = kind;
    head[1..].copy_from_slice(&len.to_le_bytes());
    w.write_all(&head)
}

pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    write_frame_header(w, kind, payload.len() as u64)?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame_header(r: &mut impl Read) -> std::io::Result<(u8, u32)> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    Ok((head[0], u32::from_le_bytes(head[1..].try_into().unwrap())))
}

/// Read a whole frame into memory. Only for small (JSON) frames.
pub fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let (kind, len) = read_frame_header(r)?;
    if len > MAX_EVENT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok((kind, buf))
}

pub fn write_json(w: &mut impl Write, value: &impl Serialize) -> std::io::Result<()> {
    write_frame(w, FRAME_EVENT, &serde_json::to_vec(value)?)
}

/// String SID of the user running this process, e.g. `S-1-5-21-...`.
pub fn current_user_sid() -> anyhow::Result<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        anyhow::ensure!(
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) != 0,
            "OpenProcessToken failed"
        );
        let mut buf = vec![0u8; 256];
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr() as _,
            buf.len() as u32,
            &mut len,
        );
        CloseHandle(token);
        anyhow::ensure!(ok != 0, "GetTokenInformation failed");
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        anyhow::ensure!(
            ConvertSidToStringSidW(user.User.Sid, &mut s) != 0,
            "bad SID"
        );
        let n = (0..).take_while(|&i| *s.add(i) != 0).count();
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s as _);
        Ok(out)
    }
}

pub fn pipe_name(sid: &str) -> String {
    format!(r"\\.\pipe\bulk-helper-{sid}")
}

/// Path of `bulk-helper.exe`, expected next to the running executable.
pub fn helper_exe() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    dir.join("bulk-helper.exe")
}

pub struct Client {
    pipe: String,
}

impl Client {
    pub fn new() -> anyhow::Result<Client> {
        Ok(Client {
            pipe: pipe_name(&current_user_sid()?),
        })
    }

    fn connect(&self) -> std::io::Result<File> {
        const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
        const SECURITY_IDENTIFICATION: u32 = 1 << 16;
        OpenOptions::new()
            .read(true)
            .write(true)
            // Don't let the server impersonate us beyond identifying who we are.
            .security_qos_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)
            .open(&self.pipe)
    }

    /// Whether the helper is running and answering.
    pub fn hello(&self) -> anyhow::Result<Event> {
        let mut c = self.connect()?;
        write_json(&mut c, &Request::Hello)?;
        let (_, body) = read_frame(&mut c)?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// Ask a running helper to exit. Fine if none is running.
    pub fn shutdown(&self) {
        if let Ok(mut c) = self.connect() {
            let _ = write_json(&mut c, &Request::Shutdown);
            let _ = read_frame(&mut c);
        }
    }

    /// Make sure the helper is up, starting its scheduled task if needed.
    /// Returns false if the task isn't installed (or won't start).
    pub fn ensure_running(&self, wait: Duration) -> bool {
        if self.hello().is_ok() {
            return true;
        }
        if !run_task() {
            return false;
        }
        let start = Instant::now();
        while start.elapsed() < wait {
            std::thread::sleep(Duration::from_millis(60));
            if self.hello().is_ok() {
                return true;
            }
        }
        false
    }

    /// MFT scan through the helper. Return `false` from `on_progress` to cancel;
    /// dropping the connection stops the helper's scan too.
    pub fn scan(
        &self,
        letter: char,
        mut on_progress: impl FnMut(ProgressInfo) -> bool,
    ) -> anyhow::Result<(Tree, Option<Journal>)> {
        let mut c = self.connect()?;
        write_json(&mut c, &Request::Scan { letter })?;
        let (mut tree, mut journal) = (None, None);
        loop {
            let (kind, len) = read_frame_header(&mut c)?;
            if kind == FRAME_TREE {
                let mut body =
                    std::io::BufReader::with_capacity(1 << 20, (&mut c).take(len as u64));
                tree = Some(wire::read_tree(&mut body)?);
                continue;
            }
            match read_event(&mut c, len)? {
                Event::Progress(p) => {
                    if !on_progress(p) {
                        anyhow::bail!("scan cancelled");
                    }
                }
                Event::Journal(j) => journal = Some(j),
                Event::Error { message } => anyhow::bail!(message),
                Event::Done { .. } => break,
                _ => {}
            }
        }
        let tree = tree.ok_or_else(|| anyhow::anyhow!("helper returned no tree"))?;
        Ok((tree, journal))
    }

    /// Changes since `from` in one go.
    pub fn changes(&self, letter: char, journal: u64, from: i64) -> anyhow::Result<CatchUp> {
        let mut c = self.connect()?;
        write_json(
            &mut c,
            &Request::Changes {
                letter,
                journal,
                from,
            },
        )?;
        let (mut all, mut next) = (Vec::new(), from);
        loop {
            let (_, len) = read_frame_header(&mut c)?;
            match read_event(&mut c, len)? {
                Event::Changes { changes, next: n } => {
                    all.extend(changes);
                    next = n;
                }
                Event::Gap => return Ok(CatchUp::Gap),
                Event::Error { message } => anyhow::bail!(message),
                Event::Done { .. } => return Ok(CatchUp::Changes(all, next)),
                _ => {}
            }
        }
    }

    /// Stream changes as they happen until `on_batch` returns false or the helper
    /// goes away. Returns `Ok(CatchUp::Gap)` if the journal was reset.
    pub fn watch(
        &self,
        letter: char,
        journal: u64,
        from: i64,
        mut on_batch: impl FnMut(Vec<Change>, i64) -> bool,
    ) -> anyhow::Result<CatchUp> {
        let mut c = self.connect()?;
        write_json(
            &mut c,
            &Request::Watch {
                letter,
                journal,
                from,
            },
        )?;
        loop {
            let (_, len) = read_frame_header(&mut c)?;
            match read_event(&mut c, len)? {
                Event::Changes { changes, next } => {
                    if !on_batch(changes, next) {
                        return Ok(CatchUp::Changes(Vec::new(), next));
                    }
                }
                Event::Gap => return Ok(CatchUp::Gap),
                Event::Error { message } => anyhow::bail!(message),
                _ => {}
            }
        }
    }
}

fn read_event(c: &mut File, len: u32) -> anyhow::Result<Event> {
    anyhow::ensure!(len <= MAX_EVENT, "frame too large");
    let mut body = vec![0u8; len as usize];
    c.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

/// Start the helper's scheduled task. No prompt once the task exists.
pub fn run_task() -> bool {
    Command::new("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn task_installed() -> bool {
    Command::new("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run `bulk-helper.exe --install` elevated, showing one UAC prompt. Returns false
/// if the user declined.
pub fn install_elevated(helper: &Path) -> anyhow::Result<bool> {
    let sid = current_user_sid()?;
    let args = format!("--install --user {sid}");
    let code = shell_runas(helper, &args)?;
    Ok(code == Some(0))
}

/// Remove the scheduled tasks (one UAC prompt).
pub fn uninstall_elevated(helper: &Path) -> anyhow::Result<bool> {
    let code = shell_runas(helper, "--uninstall")?;
    Ok(code == Some(0))
}

fn shell_runas(exe: &Path, args: &str) -> anyhow::Result<Option<u32>> {
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };

    let verb = crate::wide::wide("runas");
    let file = crate::wide::wide(exe);
    let params = crate::wide::wide(args);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0;
    unsafe {
        if ShellExecuteExW(&mut info) == 0 {
            // ERROR_CANCELLED: the user said no.
            return Ok(None);
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        Ok(Some(code))
    }
}
