//! Getting the trees a command needs, with progress on stderr, and turning the
//! paths people type into nodes.

use std::cell::OnceCell;
use std::io::{IsTerminal, Write};
use std::path::Path;

use bulk_core::drive::{self, Fresh, Loaded, Options, Step};
use bulk_core::helper::Client;
use bulk_core::tree::NodeId;
use bulk_core::volume::{self, DriveKind};
use clap::Args;
use serde::Serialize;

use crate::out;

/// How fresh the data has to be; shared by every command that reads drives.
#[derive(Args, Clone, Debug, Default)]
pub struct Freshness {
    /// Print JSON instead of a table
    #[arg(long)]
    pub json: bool,
    /// Use the saved snapshot as it is; never scan
    #[arg(long, conflicts_with = "rescan")]
    pub no_scan: bool,
    /// Scan again, even if the snapshot can be brought up to date
    #[arg(long)]
    pub rescan: bool,
}

pub struct Loader {
    opts: Options,
    helper: OnceCell<Option<Client>>,
    progress: bool,
    /// Where progress goes instead of stderr (`bulk find` draws its own screen).
    status: Option<Status>,
}

/// The latest progress message, for a UI to show.
pub type Status = std::sync::Arc<std::sync::Mutex<String>>;

impl Loader {
    pub fn new(f: &Freshness) -> Loader {
        Loader {
            opts: Options {
                scan: !f.no_scan,
                rescan: f.rescan,
            },
            helper: OnceCell::new(),
            progress: std::io::stderr().is_terminal(),
            status: None,
        }
    }

    /// A loader that reports into `status` and prints nothing.
    pub fn quiet(f: &Freshness, status: Status) -> Loader {
        Loader {
            status: Some(status),
            progress: false,
            ..Loader::new(f)
        }
    }

    fn note(&self, msg: &str) {
        match &self.status {
            Some(s) => *s.lock().unwrap() = msg.to_string(),
            None => eprintln!("{}", out::dim(msg)),
        }
    }

    fn helper(&self) -> Option<&Client> {
        self.helper
            .get_or_init(|| {
                let c = drive::helper();
                if c.is_none() {
                    self.note(
                        "Fast mode isn't available, so drives can't be brought up to date \
                         quickly. Allow it in the Bulk app.",
                    );
                }
                c
            })
            .as_ref()
    }

    /// Load one drive. Saves it back afterwards if it changed, so the next run (or
    /// the app) starts from here.
    pub fn drive(&self, letter: char) -> anyhow::Result<Loaded> {
        let vol =
            volume::query(letter).ok_or_else(|| anyhow::anyhow!("there's no drive {letter}:"))?;
        let helper = if vol.is_ntfs() { self.helper() } else { None };
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let l = drive::load(&vol, helper, &self.opts, |s| {
            if !(self.progress || self.status.is_some()) || last.elapsed().as_millis() < 100 {
                return;
            }
            last = std::time::Instant::now();
            let msg = match s {
                Step::Loading => format!("Loading {letter}:…"),
                Step::CatchingUp => format!("Bringing {letter}: up to date…"),
                Step::Scanning(p) => format!(
                    "Scanning {letter}: {} files, {}",
                    out::count(p.files),
                    out::size(p.bytes)
                ),
            };
            if let Some(status) = &self.status {
                *status.lock().unwrap() = msg;
                return;
            }
            eprint!("\r\x1b[2K{msg}");
            let _ = std::io::stderr().flush();
        });
        if self.progress {
            eprint!("\r\x1b[2K");
        }
        let l = l?;
        if l.fresh == Fresh::Snapshot && self.opts.scan && l.meta.journal.is_some() {
            self.note(&format!(
                "{letter}: is a snapshot from {}; pass --rescan to scan it now.",
                out::ago(l.meta.taken)
            ));
        }
        let _ = drive::save(&l);
        Ok(l)
    }

    /// Load the drive `path` is on and find it there.
    pub fn path(&self, path: &str) -> anyhow::Result<(Loaded, NodeId)> {
        let letter = letter_of(path)?;
        let l = self.drive(letter)?;
        let id = l.tree.find_path(path).ok_or_else(|| {
            anyhow::anyhow!(
                "{path} isn't in {letter}:'s scan from {}",
                out::ago(l.meta.taken)
            )
        })?;
        Ok((l, id))
    }

    /// Drives worth searching: local ones that have been scanned before, or `only`.
    pub fn drives(&self, only: &[char]) -> Vec<Loaded> {
        let letters: Vec<char> = if only.is_empty() {
            volume::list()
                .into_iter()
                .filter(|v| matches!(v.kind, DriveKind::Fixed | DriveKind::Removable))
                .filter(|v| bulk_core::snapshot::current_path(v.serial).exists())
                .map(|v| v.letter)
                .collect()
        } else {
            only.to_vec()
        };
        letters
            .into_iter()
            .filter_map(|l| match self.drive(l) {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!("{l}: {e:#}");
                    None
                }
            })
            .collect()
    }
}

/// Absolute Windows path for what was typed: `.`, `..\x`, `D:`, `c:/users/`.
pub fn absolute(p: &Path) -> anyhow::Result<String> {
    let s = p.to_string_lossy().replace('/', "\\");
    let b = s.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Ok(format!("{}:\\", s[..1].to_ascii_uppercase()));
    }
    let abs = std::path::absolute(Path::new(&s))?;
    let mut out = abs.to_string_lossy().into_owned();
    if let Some(rest) = out.strip_prefix(r"\\?\") {
        out = rest.to_string();
    }
    if out.len() > 3 {
        out = out.trim_end_matches('\\').to_string();
    }
    // Drive letters in upper case, like the trees have them.
    if out.as_bytes().get(1) == Some(&b':') {
        out.replace_range(..1, &out[..1].to_ascii_uppercase());
    }
    Ok(out)
}

pub fn letter_of(path: &str) -> anyhow::Result<char> {
    let b = path.as_bytes();
    anyhow::ensure!(
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':',
        "{path} isn't on a drive with a letter"
    );
    Ok(b[0].to_ascii_uppercase() as char)
}

/// Where a result's numbers come from, for JSON output.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsOf {
    pub drive: char,
    /// Unix seconds.
    pub taken: u64,
    pub fresh: Fresh,
}

pub fn as_of(l: &Loaded) -> AsOf {
    AsOf {
        drive: l.vol.letter,
        taken: l.meta.taken,
        fresh: l.fresh,
    }
}

/// "live", "scanned just now" or "snapshot from 3 days ago".
pub fn freshness(l: &Loaded) -> String {
    match l.fresh {
        Fresh::Live => "live".into(),
        Fresh::Scanned => "scanned just now".into(),
        Fresh::Snapshot => format!("snapshot from {}", out::ago(l.meta.taken)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths() {
        assert_eq!(absolute(Path::new("d:")).unwrap(), "D:\\");
        assert_eq!(absolute(Path::new("c:/users/")).unwrap(), "C:\\users");
        assert_eq!(absolute(Path::new("C:\\")).unwrap(), "C:\\");
        assert_eq!(absolute(Path::new("C:\\a\\..\\b")).unwrap(), "C:\\b");
        assert_eq!(letter_of("d:\\x").unwrap(), 'D');
    }
}
