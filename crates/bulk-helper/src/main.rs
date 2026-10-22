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
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use bulk_core::helper::{
    self, Event, FRAME_TREE, PROTOCOL, ProgressInfo, Request, SNAPSHOT_TASK, TASK_NAME, read_frame,
    write_frame_header, write_json,
};
use bulk_core::scan::{self, Progress};
use bulk_core::{snapshot, usn};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE, LocalFree,
    WAIT_OBJECT_0,
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
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, WaitForSingleObject,
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
        Some("--snapshot-all") => snapshot_all(),
        _ => Err(anyhow::anyhow!(
            "usage: bulk-helper --serve|--install|--uninstall|--snapshot-all"
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
    // The snapshot task is only created once automatic snapshots are turned on.
    register_task(TASK_NAME, &task_xml(&sid, &exe))?;
    // An older helper may still own the pipe; stop it so the new one takes over.
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "bulk-helper.exe", "/FI"])
        .arg(format!("PID ne {}", std::process::id()))
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    Ok(())
}

fn register_task(name: &str, xml: &str) -> anyhow::Result<()> {
    // schtasks wants the XML as UTF-16 with a BOM.
    let path = std::env::temp_dir().join(format!("bulk-task-{}.xml", std::process::id()));
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&path, bytes)?;
    let out = Command::new("schtasks")
        .args(["/Create", "/TN", name, "/F", "/XML"])
        .arg(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::fs::remove_file(&path);
    let out = out?;
    anyhow::ensure!(
        out.status.success(),
        "schtasks failed for {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}

/// Delete a task; fine if it doesn't exist.
fn delete_task(name: &str) -> anyhow::Result<()> {
    if Command::new("schtasks")
        .args(["/Query", "/TN", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| !o.status.success())
    {
        return Ok(());
    }
    let out = Command::new("schtasks")
        .args(["/Delete", "/TN", name, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    anyhow::ensure!(out.status.success(), "couldn't delete {name}");
    Ok(())
}

fn uninstall() -> anyhow::Result<()> {
    let _ = delete_task(SNAPSHOT_TASK);
    let out = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    anyhow::ensure!(out.status.success(), "schtasks /Delete failed");
    Ok(())
}

/// What the daily task runs: snapshot every local drive that's due, per the
/// user's settings. Cheap when nothing is due, so the task can fire often.
fn snapshot_all() -> anyhow::Result<()> {
    let settings = bulk_core::settings::load();
    if !settings.auto_snapshots {
        return Ok(());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    for vol in bulk_core::volume::list()
        .into_iter()
        .filter(|v| v.auto_scan())
    {
        let last = snapshot::history(vol.serial)
            .first()
            .map_or(0, |e| e.meta.taken);
        if now.saturating_sub(last) < settings.snapshot_every.min_gap_secs() {
            continue;
        }
        let progress = Progress::default();
        let (tree, journal, method) = if vol.is_ntfs() {
            let v = scan::mft::Volume::open(vol.letter)?;
            let journal = usn::ensure(&v).ok();
            (
                scan::mft::scan_volume(&v, vol.letter, &progress)?,
                journal,
                "mft",
            )
        } else {
            (
                scan::walk::walk(&vol.root, vol.cluster as u64, &progress)?,
                None,
                "walk",
            )
        };
        let root = *tree.get(tree.root);
        let meta = snapshot::Meta {
            letter: vol.letter,
            serial: vol.serial,
            label: vol.label.clone(),
            taken: now,
            method: method.into(),
            files: root.files,
            alloc: root.alloc,
            size: root.size,
            journal,
        };
        snapshot::save_history(&meta, &tree, settings.history_keep.clamp(1, 365))?;
        snapshot::save_current(&meta, &tree)?;
        log(&format!("snapshot {}: {} files", vol.letter, root.files));
    }
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

/// Fires daily at noon and shortly after logon (so a PC that was off at noon still
/// gets one); `--snapshot-all` skips drives that already have a recent snapshot.
fn snapshot_task_xml(sid: &str, exe: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Saves a snapshot of your drives so Bulk can show what changed over time.</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2024-01-01T12:00:00</StartBoundary>
      <ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>
    </CalendarTrigger>
    <LogonTrigger>
      <UserId>{sid}</UserId>
      <Delay>PT10M</Delay>
    </LogonTrigger>
  </Triggers>
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
    <StartWhenAvailable>true</StartWhenAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <Enabled>true</Enabled>
    <ExecutionTimeLimit>PT2H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>--snapshot-all</Arguments>
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
    /// Handles of the Bulk processes that have connected, by process id.
    apps: Mutex<Vec<(u32, isize)>>,
}

impl Activity {
    fn touch(&self) {
        self.last
            .store(self.start.elapsed().as_secs(), Ordering::Relaxed);
    }

    fn watch(&self, pid: u32) {
        let mut apps = self.apps.lock().unwrap();
        if apps.iter().any(|&(p, _)| p == pid) {
            return;
        }
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if !h.is_null() {
            apps.push((pid, h as isize));
        }
    }

    /// True once every app that used this helper has exited, however it ended
    /// (closed, killed, or stopped with Ctrl+C under `tauri dev`).
    fn apps_gone(&self) -> bool {
        let apps = self.apps.lock().unwrap();
        !apps.is_empty()
            && apps
                .iter()
                .all(|&(_, h)| unsafe { WaitForSingleObject(h as _, 0) } == WAIT_OBJECT_0)
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
        apps: Mutex::new(Vec::new()),
    });
    {
        let a = activity.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let idle = a.start.elapsed().as_secs() - a.last.load(Ordering::Relaxed);
                if a.active.load(Ordering::Relaxed) == 0
                    && (idle >= IDLE_EXIT.as_secs() || a.apps_gone())
                {
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
        let sid = sid.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(conn, raw, &sid, &a) {
                log(&format!("connection: {e:#}"));
            }
            a.touch();
            a.active.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

/// Only Bulk executables installed alongside this helper may use it. Returns the
/// client's process id when it's allowed.
fn client_allowed(pipe: isize) -> Option<u32> {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe as _, &mut pid) } == 0 {
        return None;
    }
    let proc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if proc.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(proc, 0, buf.as_mut_ptr(), &mut len) } != 0;
    unsafe { CloseHandle(proc) };
    if !ok {
        return None;
    }
    let client = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    let me = std::env::current_exe().ok()?;
    let same_dir = match (client.parent(), me.parent()) {
        (Some(a), Some(b)) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
        _ => false,
    };
    (same_dir && client != me).then_some(pid)
}

fn handle(mut conn: File, raw: isize, sid: &str, activity: &Activity) -> anyhow::Result<()> {
    let Some(pid) = client_allowed(raw) else {
        write_json(
            &mut conn,
            &Event::Error {
                message: "client not allowed".into(),
            },
        )?;
        return Ok(());
    };
    activity.watch(pid);
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
        Request::Shutdown => {
            write_json(&mut conn, &Event::Done { elapsed_ms: 0 })?;
            std::process::exit(0);
        }
        Request::Scan { letter } => scan_drive(&mut conn, letter)?,
        Request::SnapshotTask { enabled } => {
            let result = if enabled {
                std::env::current_exe()
                    .map_err(Into::into)
                    .and_then(|exe| register_task(SNAPSHOT_TASK, &snapshot_task_xml(sid, &exe)))
            } else {
                delete_task(SNAPSHOT_TASK)
            };
            match result {
                Ok(()) => write_json(&mut conn, &Event::Done { elapsed_ms: 0 })?,
                Err(e) => write_json(
                    &mut conn,
                    &Event::Error {
                        message: format!("{e:#}"),
                    },
                )?,
            }
        }
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
    if let Ok(j) = usn::ensure(&vol) {
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
