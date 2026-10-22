//! Elevated helper for Bulk. Reads raw NTFS metadata (never file contents) and hands
//! it to the app over a named pipe that only the installing user can open.
//!
//!     bulk-helper --install --user <SID>   register the scheduled task (run elevated)
//!     bulk-helper --uninstall              remove it
//!     bulk-helper --serve --user <SID>     what the task runs

#![windows_subsystem = "windows"]

use std::fs::File;
use std::io::Write;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use bulk_core::helper::{
    self, Event, FRAME_TREE, PROTOCOL, ProgressInfo, Request, TASK_NAME, read_frame,
    write_frame_header, write_json,
};
use bulk_core::scan::{self, Progress};
use bulk_core::usn;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let user = args
        .iter()
        .position(|a| a == "--user")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let result = match args.get(1).map(String::as_str) {
        Some("--serve") => serve(user),
        Some("--install") => install(user),
        Some("--uninstall") => uninstall(),
        _ => Err(anyhow::anyhow!(
            "usage: bulk-helper --serve|--install|--uninstall"
        )),
    };
    if let Err(e) = result {
        log(&format!("error: {e:#}"));
        std::process::exit(1);
    }
}

fn log(msg: &str) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Bulk")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("helper.log"))
    {
        let _ = writeln!(f, "{msg}");
    }
}

fn valid_sid(s: &str) -> bool {
    s.starts_with("S-1-") && s[4..].chars().all(|c| c.is_ascii_digit() || c == '-')
}

fn install(user: Option<String>) -> anyhow::Result<()> {
    let sid = user.context("--install needs --user <SID>")?;
    anyhow::ensure!(valid_sid(&sid), "bad SID");
    let exe = std::env::current_exe()?;
    let xml = task_xml(&sid, &exe);

    // schtasks wants the XML as UTF-16 with a BOM.
    let path = std::env::temp_dir().join(format!("bulk-task-{}.xml", std::process::id()));
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&path, bytes)?;
    let out = Command::new("schtasks")
        .args(["/Create", "/TN", TASK_NAME, "/F", "/XML"])
        .arg(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::fs::remove_file(&path);
    let out = out?;
    anyhow::ensure!(
        out.status.success(),
        "schtasks failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // An older helper may still own the pipe; stop it so the new one takes over.
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "bulk-helper.exe", "/FI"])
        .arg(format!("PID ne {}", std::process::id()))
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    Ok(())
}

fn uninstall() -> anyhow::Result<()> {
    let out = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    anyhow::ensure!(out.status.success(), "schtasks /Delete failed");
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn task_xml(sid: &str, exe: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    // Priority 4 keeps normal I/O priority; the default (7) makes scans much slower.
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Lets Bulk read drive metadata quickly without asking for admin rights each time.</Description>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>--serve --user {sid}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

struct Activity {
    active: AtomicUsize,
    last: AtomicU64,
    start: Instant,
}

impl Activity {
    fn touch(&self) {
        self.last
            .store(self.start.elapsed().as_secs(), Ordering::Relaxed);
    }
}

fn serve(user: Option<String>) -> anyhow::Result<()> {
    let sid = match user {
        Some(s) => s,
        None => helper::current_user_sid()?,
    };
    anyhow::ensure!(valid_sid(&sid), "bad SID");
    let name: Vec<u16> = helper::pipe_name(&sid).encode_utf16().chain([0]).collect();

    // Only the user (and SYSTEM) may connect; the medium label lets the
    // non-elevated app write to a pipe owned by this elevated process.
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)S:(ML;;NW;;;ME)")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    };
    anyhow::ensure!(ok != 0, "bad security descriptor");
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };

    let activity = Arc::new(Activity {
        active: AtomicUsize::new(0),
        last: AtomicU64::new(0),
        start: Instant::now(),
    });
    {
        let a = activity.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let idle = a.start.elapsed().as_secs() - a.last.load(Ordering::Relaxed);
                if a.active.load(Ordering::Relaxed) == 0 && idle >= IDLE_EXIT.as_secs() {
                    std::process::exit(0);
                }
            }
        });
    }

    let mut first = true;
    loop {
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX
                    | if first {
                        FILE_FLAG_FIRST_PIPE_INSTANCE
                    } else {
                        0
                    },
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                1 << 20,
                1 << 16,
                0,
                &sa,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            if first {
                // Another helper already owns the pipe.
                unsafe { LocalFree(sd as _) };
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        first = false;

        let connected = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            unsafe { CloseHandle(h) };
            continue;
        }

        let a = activity.clone();
        a.active.fetch_add(1, Ordering::Relaxed);
        a.touch();
        let conn = unsafe { File::from_raw_handle(h as _) };
        let raw = h as isize;
        std::thread::spawn(move || {
            if let Err(e) = handle(conn, raw) {
                log(&format!("connection: {e:#}"));
            }
            a.touch();
            a.active.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

/// Only Bulk executables installed alongside this helper may use it.
fn client_allowed(pipe: isize) -> bool {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe as _, &mut pid) } == 0 {
        return false;
    }
    let proc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if proc.is_null() {
        return false;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(proc, 0, buf.as_mut_ptr(), &mut len) } != 0;
    unsafe { CloseHandle(proc) };
    if !ok {
        return false;
    }
    let client = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    let Ok(me) = std::env::current_exe() else {
        return false;
    };
    let same_dir = match (client.parent(), me.parent()) {
        (Some(a), Some(b)) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
        _ => false,
    };
    same_dir && client != me
}

fn handle(mut conn: File, raw: isize) -> anyhow::Result<()> {
    if !client_allowed(raw) {
        write_json(
            &mut conn,
            &Event::Error {
                message: "client not allowed".into(),
            },
        )?;
        return Ok(());
    }
    let (_, body) = read_frame(&mut conn)?;
    let req: Request = serde_json::from_slice(&body)?;
    if !matches!(req, Request::Hello) {
        log(&format!("request: {req:?}"));
    }
    match req {
        Request::Hello => write_json(
            &mut conn,
            &Event::Hello {
                protocol: PROTOCOL,
                version: env!("CARGO_PKG_VERSION").into(),
                exe: std::env::current_exe()?.to_string_lossy().into_owned(),
            },
        )?,
        Request::Scan { letter } => scan_drive(&mut conn, letter)?,
        Request::Changes {
            letter,
            journal,
            from,
        } => send_changes(&mut conn, letter, journal, from, false)?,
        Request::Watch {
            letter,
            journal,
            from,
        } => send_changes(&mut conn, letter, journal, from, true)?,
    }
    Ok(())
}

fn open_volume(conn: &mut File, letter: char) -> anyhow::Result<Option<scan::mft::Volume>> {
    if !letter.is_ascii_alphabetic() {
        write_json(
            conn,
            &Event::Error {
                message: "bad drive letter".into(),
            },
        )?;
        return Ok(None);
    }
    match scan::mft::Volume::open(letter) {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            write_json(
                conn,
                &Event::Error {
                    message: format!("{e:#}"),
                },
            )?;
            Ok(None)
        }
    }
}

/// Batches stay well under the pipe's JSON frame limit.
const CHANGE_BATCH: usize = 4000;

fn send_changes(
    conn: &mut File,
    letter: char,
    journal: u64,
    from: i64,
    watch: bool,
) -> anyhow::Result<()> {
    let Some(vol) = open_volume(conn, letter)? else {
        return Ok(());
    };
    let start = Instant::now();
    let mut next = from;
    loop {
        let wait = watch.then_some(Duration::from_secs(2));
        let (frns, n) = match usn::changed_since(&vol, journal, next, wait) {
            Ok(r) => r,
            Err(usn::JournalError::Gap) => {
                write_json(conn, &Event::Gap)?;
                return Ok(());
            }
            Err(e) => {
                write_json(
                    conn,
                    &Event::Error {
                        message: e.to_string(),
                    },
                )?;
                return Ok(());
            }
        };
        next = n;
        if frns.is_empty() {
            // Heartbeat while watching; a failed write means the app is gone.
            if watch {
                write_json(
                    conn,
                    &Event::Changes {
                        changes: Vec::new(),
                        next,
                    },
                )?;
            }
        }
        for chunk in frns.chunks(CHANGE_BATCH) {
            write_json(
                conn,
                &Event::Changes {
                    changes: usn::states(&vol, chunk),
                    next,
                },
            )?;
        }
        if !watch {
            break;
        }
        // Let bursts (copies, installs) settle into fewer, larger batches.
        std::thread::sleep(Duration::from_millis(400));
    }
    write_json(
        conn,
        &Event::Done {
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    )?;
    Ok(())
}

fn scan_drive(conn: &mut File, letter: char) -> anyhow::Result<()> {
    let Some(vol) = open_volume(conn, letter)? else {
        return Ok(());
    };
    // Note the journal position first, so nothing that changes mid-scan is missed.
    if let Ok(j) = usn::query(&vol) {
        write_json(conn, &Event::Journal(j))?;
    }
    let start = Instant::now();
    let progress = Arc::new(Progress::default());
    let (tx, rx) = mpsc::channel();
    {
        let p = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(scan::mft::scan_volume(&vol, letter, &p));
        });
    }

    let result = loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(r) => break r,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // A failed write means the app went away: stop scanning.
                if write_json(conn, &Event::Progress(ProgressInfo::of(&progress))).is_err() {
                    progress.cancel();
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("scan thread died"),
        }
    };

    match result {
        Ok(tree) => {
            write_json(conn, &Event::Progress(ProgressInfo::of(&progress)))?;
            let len = bulk_core::wire::encoded_len(&tree);
            let mut w = std::io::BufWriter::with_capacity(1 << 20, &mut *conn);
            write_frame_header(&mut w, FRAME_TREE, len)?;
            bulk_core::wire::write_tree(&mut w, &tree)?;
            w.flush()?;
            drop(w);
            write_json(
                conn,
                &Event::Done {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            )?;
        }
        Err(e) => write_json(
            conn,
            &Event::Error {
                message: format!("{e:#}"),
            },
        )?,
    }
    Ok(())
}
