//! Deleting from the command line: to the Recycle Bin by default, permanently only
//! with a flag spelled out in full, and never anything Windows or Bulk needs. Every
//! run is logged so recycled items can be put back with `bulk restore`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bulk_core::trash;
use bulk_core::tree::{NodeId, flags};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::load::{self, Freshness, Loader};
use crate::out::{self, Table};

/// Most items one query may select.
const QUERY_MAX: usize = 10_000;

#[derive(Args)]
pub struct DeleteArgs {
    /// Files or folders to delete
    #[arg(required_unless_present = "query", conflicts_with = "query")]
    paths: Vec<PathBuf>,
    /// Delete everything a search finds instead (needs --in)
    #[arg(long, requires = "within")]
    query: Option<String>,
    /// Folder the --query search is limited to
    #[arg(long = "in", value_name = "PATH")]
    within: Option<PathBuf>,
    /// Actually delete. Without it, only lists what would be deleted
    #[arg(long, short = 'y')]
    yes: bool,
    /// Delete for good instead of moving to the Recycle Bin. Can't be undone
    #[arg(long)]
    permanently_delete: bool,
    /// Refuse when the total is bigger than this
    #[arg(long, default_value = "50GB", value_parser = parse_size)]
    max: u64,
    #[command(flatten)]
    fresh: Freshness,
}

fn parse_size(s: &str) -> Result<u64, String> {
    bulk_core::search::parse_size(s).ok_or_else(|| format!("not a size: {s} (try 50GB)"))
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Target {
    path: String,
    dir: bool,
    /// Size on disk, when the scan knows it.
    alloc: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Refused {
    path: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteResult {
    dry_run: bool,
    permanent: bool,
    total: u64,
    items: Vec<Target>,
    refused: Vec<Refused>,
    failed: Vec<Refused>,
    /// For `bulk restore`; set once something went to the Recycle Bin.
    batch: Option<u32>,
}

pub fn delete(a: DeleteArgs) -> anyhow::Result<()> {
    let loader = Loader::new(&a.fresh);
    let (targets, mut refused) = match &a.query {
        Some(q) => from_query(&loader, q, a.within.as_deref().unwrap())?,
        None => from_paths(&loader, &a.paths)?,
    };
    let targets = topmost(targets);
    let total: u64 = targets.iter().filter_map(|t| t.alloc).sum();
    if total > a.max {
        refused.push(Refused {
            path: "(all)".into(),
            reason: format!(
                "{} is more than --max {}",
                out::size(total),
                out::size(a.max)
            ),
        });
    }

    let mut res = DeleteResult {
        dry_run: !a.yes || !refused.is_empty(),
        permanent: a.permanently_delete,
        total,
        items: targets,
        refused,
        failed: Vec::new(),
        batch: None,
    };
    if !res.dry_run && !res.items.is_empty() {
        let started = SystemTime::now();
        let mut done = Vec::new();
        for t in &res.items {
            let p = Path::new(&t.path);
            let r = if a.permanently_delete {
                trash::delete_permanently(p)
            } else {
                trash::recycle(p)
            };
            match r {
                Ok(()) => done.push(t.clone()),
                Err(e) => res.failed.push(Refused {
                    path: t.path.clone(),
                    reason: format!("{e:#}"),
                }),
            }
        }
        if !done.is_empty() {
            res.batch = Some(log_batch(started, a.permanently_delete, &done)?);
        }
    }

    if a.fresh.json {
        out::json(&res)?;
    } else {
        print(&res);
    }
    // Asked to delete, and something wasn't.
    if a.yes && !(res.failed.is_empty() && res.refused.is_empty()) {
        std::process::exit(1);
    }
    Ok(())
}

fn print(r: &DeleteResult) {
    if !r.refused.is_empty() {
        println!("{}", out::bold("Nothing was deleted:"));
        let mut t = Table::new(&[false, false]);
        for x in &r.refused {
            t.row(vec![x.path.clone(), out::dim(&x.reason)]);
        }
        t.print();
        return;
    }
    if r.items.is_empty() {
        println!("{}", out::dim("Nothing to delete."));
        return;
    }
    let mut t = Table::new(&[true, false]);
    for it in &r.items {
        t.row(vec![
            it.alloc.map(out::size).unwrap_or_else(|| "?".into()),
            if it.dir {
                format!("{}\\", it.path)
            } else {
                it.path.clone()
            },
        ]);
    }
    let n = r.items.len();
    let what = format!(
        "{} item{} ({})",
        out::count(n as u64),
        if n == 1 { "" } else { "s" },
        out::size(r.total)
    );
    if r.dry_run {
        t.print();
        let verb = if r.permanent {
            "would be deleted permanently"
        } else {
            "would go to the Recycle Bin"
        };
        println!("{}", out::bold(&format!("{what} {verb}.")));
        println!("{}", out::dim("Run again with --yes to do it."));
        return;
    }
    let ok = n - r.failed.len();
    if ok > 0 {
        let verb = if r.permanent {
            "Deleted permanently:"
        } else {
            "Moved to the Recycle Bin:"
        };
        println!("{verb} {what}");
    }
    for f in &r.failed {
        eprintln!("Couldn't delete {}: {}", f.path, f.reason);
    }
    if let (Some(id), false) = (r.batch, r.permanent) {
        println!("{}", out::dim(&format!("Undo with: bulk restore {id}")));
    }
}

/// Why `id` mustn't be deleted, beyond its path: the attributes Windows sets on
/// its own files.
fn node_refusal(l: &bulk_core::drive::Loaded, id: NodeId) -> Option<&'static str> {
    let n = l.tree.get(id);
    if n.has(flags::SYSTEM) {
        Some("marked as a system file")
    } else if n.has(flags::META) {
        Some("part of the file system")
    } else {
        None
    }
}

fn from_paths(loader: &Loader, paths: &[PathBuf]) -> anyhow::Result<(Vec<Target>, Vec<Refused>)> {
    let mut targets = Vec::new();
    let mut refused = Vec::new();
    let mut loaded: Vec<bulk_core::drive::Loaded> = Vec::new();
    for p in paths {
        let path = load::absolute(p)?;
        let refuse = |why: &str| Refused {
            path: path.clone(),
            reason: why.to_string(),
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            refused.push(refuse("doesn't exist"));
            continue;
        };
        if let Some(why) = trash::protected(Path::new(&path)) {
            refused.push(refuse(why));
            continue;
        }
        // Sizes (and system flags) from the scan, when it has the path.
        let letter = load::letter_of(&path)?;
        if !loaded.iter().any(|l| l.vol.letter == letter) {
            loaded.push(loader.drive(letter)?);
        }
        let l = loaded.iter().find(|l| l.vol.letter == letter).unwrap();
        let id = l.tree.find_path(&path);
        if let Some(why) = id.and_then(|id| node_refusal(l, id)) {
            refused.push(refuse(why));
            continue;
        }
        targets.push(Target {
            alloc: id
                .map(|id| l.tree.get(id).alloc)
                .or_else(|| meta.is_file().then(|| meta.len())),
            path,
            dir: meta.is_dir(),
        });
    }
    Ok((targets, refused))
}

fn from_query(
    loader: &Loader,
    query: &str,
    within: &Path,
) -> anyhow::Result<(Vec<Target>, Vec<Refused>)> {
    let path = load::absolute(within)?;
    if let Some(why) = trash::protected(Path::new(&path))
        && why == "a whole drive"
    {
        anyhow::bail!("--in has to be a folder, not a whole drive");
    }
    let (l, scope) = loader.path(&path)?;
    let q = bulk_core::search::parse(query, bulk_core::os::now());
    let (hits, total) = bulk_core::search::search(&l.tree, &q, Some(scope), QUERY_MAX);
    anyhow::ensure!(
        total <= QUERY_MAX,
        "the search finds {} items; narrow it to at most {}",
        out::count(total as u64),
        out::count(QUERY_MAX as u64)
    );
    let mut targets = Vec::new();
    let mut refused = Vec::new();
    for h in hits {
        if h.id == scope {
            continue;
        }
        let p = l.tree.path(h.id);
        let why = trash::protected(Path::new(&p)).or_else(|| node_refusal(&l, h.id));
        match why {
            Some(why) => refused.push(Refused {
                path: p,
                reason: why.into(),
            }),
            None => {
                let n = l.tree.get(h.id);
                targets.push(Target {
                    path: p,
                    dir: n.is_dir(),
                    alloc: Some(n.alloc),
                })
            }
        }
    }
    Ok((targets, refused))
}

/// Drop anything inside another target: deleting the folder covers it.
fn topmost(mut targets: Vec<Target>) -> Vec<Target> {
    targets.sort_by_key(|t| t.path.to_ascii_lowercase());
    let mut out: Vec<Target> = Vec::new();
    for t in targets {
        let lower = t.path.to_ascii_lowercase();
        let inside = out.last().is_some_and(|p| {
            let p = p.path.to_ascii_lowercase();
            lower == p || lower.starts_with(&format!("{}\\", p.trim_end_matches('\\')))
        });
        if !inside {
            out.push(t);
        }
    }
    out
}

// ---------------------------------------------------------------------------

/// One `bulk delete` run, as kept in `deleted.jsonl`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    id: u32,
    /// Unix seconds when it started.
    at: u64,
    permanent: bool,
    items: Vec<Target>,
}

fn log_path() -> PathBuf {
    bulk_core::snapshot::root().with_file_name("deleted.jsonl")
}

fn batches() -> Vec<Batch> {
    let Ok(f) = std::fs::File::open(log_path()) else {
        return Vec::new();
    };
    std::io::BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect()
}

fn log_batch(started: SystemTime, permanent: bool, items: &[Target]) -> anyhow::Result<u32> {
    let id = batches().last().map_or(1, |b| b.id + 1);
    let b = Batch {
        id,
        at: started
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        permanent,
        items: items.to_vec(),
    };
    let path = log_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{}", serde_json::to_string(&b)?)?;
    Ok(id)
}

#[derive(Args)]
pub struct RestoreArgs {
    /// Number from `bulk delete`'s output; leave out to list recent deletes
    id: Option<u32>,
    /// Print JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreResult {
    restored: Vec<String>,
    failed: Vec<Refused>,
}

pub fn restore(a: RestoreArgs) -> anyhow::Result<()> {
    let all = batches();
    let Some(id) = a.id else {
        let recent: Vec<&Batch> = all.iter().rev().take(15).collect();
        if a.json {
            return out::json(&recent);
        }
        if recent.is_empty() {
            println!(
                "{}",
                out::dim("Nothing has been deleted with bulk delete yet.")
            );
            return Ok(());
        }
        let mut t = Table::new(&[true, false, true, false, false]);
        for b in recent {
            let total: u64 = b.items.iter().filter_map(|i| i.alloc).sum();
            let first = &b.items[0].path;
            let rest = if b.items.len() > 1 {
                format!(" and {} more", b.items.len() - 1)
            } else {
                String::new()
            };
            t.row(vec![
                out::bold(&b.id.to_string()),
                out::dim(&out::ago(b.at)),
                out::size(total),
                if b.permanent {
                    out::dim("permanent")
                } else {
                    "recycled".into()
                },
                format!("{first}{rest}"),
            ]);
        }
        t.print();
        return Ok(());
    };

    let b = all
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| anyhow::anyhow!("there's no delete number {id}"))?;
    anyhow::ensure!(
        !b.permanent,
        "delete {id} was permanent, so there's nothing to restore"
    );
    // The Recycle Bin's clock and ours can differ by a moment.
    let since = SystemTime::UNIX_EPOCH + Duration::from_secs(b.at.saturating_sub(5));
    let mut res = RestoreResult {
        restored: Vec::new(),
        failed: Vec::new(),
    };
    for it in &b.items {
        let p = Path::new(&it.path);
        let mut found = trash::find_recycled(p, since);
        found.sort_by_key(|r| r.deleted);
        let r = match found.last() {
            Some(item) => trash::restore(item),
            None => Err(anyhow::anyhow!("not in the Recycle Bin any more")),
        };
        match r {
            Ok(()) => res.restored.push(it.path.clone()),
            Err(e) => res.failed.push(Refused {
                path: it.path.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    if a.json {
        out::json(&res)?;
    } else {
        for p in &res.restored {
            println!("Restored {p}");
        }
        for f in &res.failed {
            eprintln!("Couldn't restore {}: {}", f.path, f.reason);
        }
    }
    if !res.failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}
