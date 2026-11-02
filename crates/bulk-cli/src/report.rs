//! Commands that look further: changes over time, snapshots, reclaimable space,
//! duplicates and exported reports.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bulk_core::diff;
use bulk_core::dupes::{self, DupeProgress};
use bulk_core::junk::{self, Safety};
use bulk_core::snapshot;
use bulk_core::tree::NodeId;
use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::load::{self, AsOf, Freshness, Loader};
use crate::out::{self, Table};

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct ChangesArgs {
    /// Folder or drive (default: the current folder)
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Compare with the snapshot from about this long ago (7d, 2w, 3m) or this
    /// date (2026-10-01). Default: the latest snapshot
    #[arg(long)]
    since: Option<String>,
    /// Changed files anywhere below, instead of what's directly inside
    #[arg(long)]
    files: bool,
    /// How many to list
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
    #[command(flatten)]
    fresh: Freshness,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChangesResult {
    as_of: AsOf,
    path: String,
    /// When the snapshot compared with was taken, unix seconds.
    since: u64,
    old_alloc: u64,
    new_alloc: u64,
    total: usize,
    changes: Vec<bulk_core::export::ChangeRow>,
}

/// Unix seconds for `7d`, `2w`, `3m`, `1y`, `12h`, or a `2026-10-01` date.
fn when(s: &str, now: u64) -> anyhow::Result<u64> {
    let s = s.trim();
    if let Some((n, unit)) = s
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| s.split_at(i))
        .filter(|(n, u)| !n.is_empty() && u.len() == 1)
    {
        let n: u64 = n.parse()?;
        let secs = match unit {
            "h" => 3600,
            "d" => 86_400,
            "w" => 7 * 86_400,
            "m" => 30 * 86_400,
            "y" => 365 * 86_400,
            _ => anyhow::bail!("unknown unit in {s}: use h, d, w, m or y"),
        };
        return Ok(now.saturating_sub(n * secs));
    }
    // A date: let the search parser read it, as in `before:2026-10-01`.
    let parts: Vec<u64> = s.split('-').filter_map(|p| p.parse().ok()).collect();
    anyhow::ensure!(
        (1..=3).contains(&parts.len()) && parts[0] > 1970,
        "{s} isn't a date (2026-10-01) or an age (7d)"
    );
    let (y, m, d) = (
        parts[0] as i64,
        *parts.get(1).unwrap_or(&1) as i64,
        *parts.get(2).unwrap_or(&1) as i64,
    );
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(((era * 146_097 + doe - 719_468) * 86_400) as u64)
}

/// What changed in a folder since an earlier snapshot.
pub fn changes(a: ChangesArgs) -> anyhow::Result<()> {
    let path = load::absolute(&a.path)?;
    let loader = Loader::new(&a.fresh);
    let (l, new_root) = loader.path(&path)?;
    let now = bulk_core::os::now();

    // Snapshots older than the tree we have now, newest first.
    let history: Vec<_> = snapshot::history(l.vol.serial)
        .into_iter()
        .filter(|e| e.meta.taken + 60 < l.meta.taken)
        .collect();
    anyhow::ensure!(
        !history.is_empty(),
        "{}: has no earlier snapshot to compare with. Bulk saves one each time it \
         scans; automatic snapshots can be turned on in the app's settings.",
        l.vol.letter
    );
    let entry = match &a.since {
        None => &history[0],
        Some(s) => {
            let t = when(s, now)?;
            history
                .iter()
                .find(|e| e.meta.taken <= t)
                .unwrap_or(history.last().unwrap())
        }
    };
    let (_, old) = snapshot::load(&entry.path)?;
    let old_root = old.find_path(&path);
    let new = &l.tree;

    let rows: Vec<bulk_core::export::ChangeRow> = if a.files {
        let mut all = bulk_core::export::collect_changes(
            &old,
            new,
            old_root,
            new_root,
            usize::MAX,
            diff::Filters::default(),
        );
        all.retain(|r| !r.dir);
        all.sort_by_key(|r| std::cmp::Reverse(r.delta.unsigned_abs()));
        all
    } else {
        let base = new.path(new_root);
        diff::children(
            &old,
            new,
            old_root,
            Some(new_root),
            usize::MAX,
            diff::Filters::default(),
        )
        .into_iter()
        .map(|it| bulk_core::export::ChangeRow {
            path: format!("{}\\{}", base.trim_end_matches('\\'), it.name),
            status: it.status,
            dir: it.dir,
            old_alloc: it.old_alloc,
            new_alloc: it.new_alloc,
            delta: it.delta,
            old_files: it.old_files,
            new_files: it.new_files,
        })
        .collect()
    };
    let total = rows.len();
    let rows: Vec<_> = rows.into_iter().take(a.limit).collect();
    let old_alloc = old_root.map_or(0, |id| old.get(id).alloc);
    let new_alloc = new.get(new_root).alloc;

    if a.fresh.json {
        return out::json(&ChangesResult {
            as_of: load::as_of(&l),
            path: new.path(new_root),
            since: entry.meta.taken,
            old_alloc,
            new_alloc,
            total,
            changes: rows,
        });
    }
    println!(
        "{} · since {} ({}): {} ({} → {})",
        out::bold(&new.path(new_root)),
        out::date(entry.meta.taken as u32),
        out::ago(entry.meta.taken),
        out::bold(&out::delta_cell(new_alloc as i64 - old_alloc as i64)),
        out::size(old_alloc),
        out::size(new_alloc)
    );
    let base = new_root;
    let mut t = Table::new(&[true, false, false]);
    for r in &rows {
        let status = match r.status {
            diff::Status::Added => "new",
            diff::Status::Removed => "gone",
            diff::Status::Changed => "",
        };
        let shown = r
            .path
            .strip_prefix(&new.path(base))
            .map(|p| p.trim_start_matches('\\').to_string())
            .unwrap_or_else(|| r.path.clone());
        let th = crate::theme::get();
        let status = match r.status {
            diff::Status::Added => out::paint(status, th.ok),
            diff::Status::Removed => out::paint(status, th.danger),
            diff::Status::Changed => String::new(),
        };
        t.row(vec![
            out::delta_cell(r.delta),
            status,
            out::path_cell(&shown, r.dir),
        ]);
    }
    t.print();
    if rows.is_empty() {
        println!("{}", out::dim("Nothing changed."));
    }
    out::more(rows.len(), total);
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct SnapshotsArgs {
    /// Drive (default: the current folder's)
    drive: Option<String>,
    /// Print JSON instead of a table
    #[arg(long)]
    json: bool,
}

/// The dated snapshots `bulk changes` can compare with.
pub fn snapshots(a: SnapshotsArgs) -> anyhow::Result<()> {
    let p = load::absolute(&PathBuf::from(a.drive.as_deref().unwrap_or(".")))?;
    let letter = load::letter_of(&p)?;
    let vol = bulk_core::volume::query(letter)
        .ok_or_else(|| anyhow::anyhow!("there's no drive {letter}:"))?;
    let list = snapshot::history(vol.serial);
    if a.json {
        return out::json(&list);
    }
    if list.is_empty() {
        println!(
            "{}",
            out::dim(&format!("{letter}: has no dated snapshots yet."))
        );
        return Ok(());
    }
    let mut t = Table::new(&[false, false, true, true]);
    for e in &list {
        t.row(vec![
            out::date(e.meta.taken as u32),
            out::dim(&out::ago(e.meta.taken)),
            out::size(e.meta.alloc),
            out::dim(&format!("{} files", out::count(e.meta.files as u64))),
        ]);
    }
    t.print();
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct JunkArgs {
    /// Folder or drive (default: every drive Bulk has scanned)
    path: Option<PathBuf>,
    /// List every item, not just the biggest few of each kind
    #[arg(long)]
    all: bool,
    #[command(flatten)]
    fresh: Freshness,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JunkResult {
    as_of: Vec<AsOf>,
    /// What's marked safe to delete, bytes.
    safe: u64,
    /// What's worth a look before deleting, bytes.
    review: u64,
    findings: Vec<junk::Finding>,
    /// Kinds and paths hidden in the app's Cleanup tab, left out here too.
    hidden: Vec<String>,
}

/// Space that can usually be got back, and how safe each kind is to remove.
pub fn junk(a: JunkArgs) -> anyhow::Result<()> {
    let loader = Loader::new(&a.fresh);
    let now = bulk_core::os::now();
    let (loaded, roots): (Vec<_>, Vec<NodeId>) = match &a.path {
        Some(p) => {
            let (l, id) = loader.path(&load::absolute(p)?)?;
            (vec![l], vec![id])
        }
        None => {
            let ls = loader.drives(&[]);
            let roots = ls.iter().map(|l| l.tree.root).collect();
            (ls, roots)
        }
    };
    let mut findings: Vec<junk::Finding> = Vec::new();
    for (l, root) in loaded.iter().zip(roots) {
        for f in junk::scan(&l.tree, root, now) {
            match findings.iter_mut().find(|x| x.key == f.key) {
                Some(x) => {
                    x.alloc += f.alloc;
                    x.items.extend(f.items);
                    x.items.sort_by(|a, b| b.alloc.cmp(&a.alloc));
                    if f.safety == Safety::Review {
                        x.safety = Safety::Review;
                    }
                }
                None => findings.push(f),
            }
        }
    }
    // What was hidden in the app's Cleanup tab stays hidden here.
    let hidden = bulk_core::settings::load().cleanup_ignore;
    let mut findings = junk::without(findings, &hidden);
    findings.sort_by(|a, b| b.alloc.cmp(&a.alloc));
    let sum = |s: Safety| {
        findings
            .iter()
            .filter(|f| f.safety == s)
            .map(|f| f.alloc)
            .sum::<u64>()
    };
    let (safe, review) = (sum(Safety::Safe), sum(Safety::Review));
    if !a.all {
        for f in &mut findings {
            f.items.truncate(if a.fresh.json { 50 } else { 3 });
        }
    }
    if a.fresh.json {
        return out::json(&JunkResult {
            as_of: loaded.iter().map(load::as_of).collect(),
            safe,
            review,
            findings,
            hidden,
        });
    }
    if findings.is_empty() {
        println!("{}", out::dim("Nothing obvious to clean up."));
        return Ok(());
    }
    let th = crate::theme::get();
    println!(
        "{} safe to delete · {} worth a look",
        out::bold(&out::paint(&out::size(safe), th.ok)),
        out::bold(&out::paint(&out::size(review), th.warn))
    );
    for f in &findings {
        let (tag, c) = match f.safety {
            Safety::Safe => (" safe ", th.ok),
            Safety::Review => (" check first ", th.warn),
            Safety::Info => (" good to know ", th.muted),
        };
        println!();
        println!(
            "{}  {}  {}",
            out::bold(&format!("{:>9}", out::size(f.alloc))),
            out::bold(f.label),
            out::paint(tag, c)
        );
        println!("           {}", out::dim(f.why));
        let mut t = Table::new(&[true, false]);
        for it in &f.items {
            t.row(vec![
                out::rjust(&out::size_cell(it.alloc), 9),
                out::path_cell(&it.path, false),
            ]);
        }
        t.print();
    }
    println!();
    if !hidden.is_empty() {
        println!(
            "{}",
            out::dim(&format!(
                "{} hidden in the app's Cleanup tab.",
                out::count(hidden.len() as u64)
            ))
        );
    }
    println!(
        "{}",
        out::dim("Delete with: bulk delete <path> (it lists first; add --yes to do it)")
    );
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct DupesArgs {
    /// Folder or drive (default: the current folder)
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Skip files smaller than this
    #[arg(long, default_value = "1MB", value_parser = parse_size)]
    min: u64,
    /// Look inside Windows and program folders too
    #[arg(long)]
    system: bool,
    /// How many sets to list
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
    #[command(flatten)]
    fresh: Freshness,
}

fn parse_size(s: &str) -> Result<u64, String> {
    bulk_core::search::parse_size(s).ok_or_else(|| format!("not a size: {s} (try 1MB)"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DupesResult {
    as_of: AsOf,
    path: String,
    /// Space all the extra copies take, bytes.
    wasted: u64,
    total: usize,
    groups: Vec<dupes::Group>,
}

/// Files with the same contents, compared by hash.
pub fn dupes(a: DupesArgs) -> anyhow::Result<()> {
    let path = load::absolute(&a.path)?;
    let loader = Loader::new(&a.fresh);
    let (l, root) = loader.path(&path)?;
    let cands = dupes::candidates(&l.tree, root, a.min, !a.system, None);
    let progress = DupeProgress::default();
    let show = std::io::stderr().is_terminal();
    let mut groups = std::thread::scope(|s| {
        let job = s.spawn(|| dupes::find(cands, &progress));
        while !job.is_finished() {
            if show {
                let phase = match progress.phase.load(Ordering::Relaxed) {
                    0 => "Listing",
                    1 => "Quick check",
                    _ => "Comparing",
                };
                eprint!(
                    "\r\x1b[2K{phase}: {} of {} files, {} of {}",
                    out::count(progress.files_done.load(Ordering::Relaxed)),
                    out::count(progress.files_total.load(Ordering::Relaxed)),
                    out::size(progress.bytes_done.load(Ordering::Relaxed)),
                    out::size(progress.bytes_total.load(Ordering::Relaxed)),
                );
                let _ = std::io::stderr().flush();
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        job.join().unwrap()
    });
    if show {
        eprint!("\r\x1b[2K");
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
    let wasted: u64 = groups.iter().map(|g| g.wasted()).sum();
    let total = groups.len();
    groups.truncate(a.limit);

    if a.fresh.json {
        return out::json(&DupesResult {
            as_of: load::as_of(&l),
            path: l.tree.path(root),
            wasted,
            total,
            groups,
        });
    }
    println!(
        "{} · {} sets of duplicates · {} in extra copies",
        out::bold(&l.tree.path(root)),
        out::count(total as u64),
        out::bold(&out::size(wasted))
    );
    for g in &groups {
        println!();
        println!(
            "{}",
            out::bold(&format!(
                "{} extra · {} copies of {}",
                out::size(g.wasted()),
                g.files.len(),
                out::size(g.size)
            ))
        );
        for f in &g.files {
            println!(
                "  {}  {}",
                out::dim(&out::date(f.mtime)),
                out::path_cell(&f.path, false)
            );
        }
    }
    out::more(groups.len(), total);
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Html,
    Csv,
    Json,
}

#[derive(Args)]
pub struct ExportArgs {
    /// Folder or drive (default: the current folder)
    #[arg(default_value = ".")]
    path: PathBuf,
    /// File to write; .html, .csv or .json picks the format
    #[arg(short, long)]
    output: PathBuf,
    /// Format, when the file name doesn't say
    #[arg(long, value_enum)]
    format: Option<Format>,
    /// Only include what matches this search
    #[arg(long, short = 'f')]
    filter: Option<String>,
    #[command(flatten)]
    fresh: Freshness,
}

/// A report like the app's Export button writes.
pub fn export(a: ExportArgs) -> anyhow::Result<()> {
    let format = match a.format {
        Some(f) => f,
        None => match a
            .output
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .as_deref()
        {
            Some("html" | "htm") => Format::Html,
            Some("csv") => Format::Csv,
            Some("json") => Format::Json,
            _ => anyhow::bail!("name the file .html, .csv or .json, or pass --format"),
        },
    };
    let path = load::absolute(&a.path)?;
    let loader = Loader::new(&a.fresh);
    let (l, root) = loader.path(&path)?;
    let now = bulk_core::os::now();

    let mut title = if root == l.tree.root {
        format!(
            "{}: {}",
            l.vol.letter,
            if l.vol.label.is_empty() {
                "Local disk"
            } else {
                &l.vol.label
            }
        )
    } else {
        l.tree.name(root).to_string()
    };
    let filtered;
    let (tree, root) = match &a.filter {
        Some(q) => {
            let f = bulk_core::filter::Filter::new(&l.tree, q, now);
            filtered = f.to_tree(&l.tree);
            let root = filtered
                .find_path(&path)
                .ok_or_else(|| anyhow::anyhow!("nothing in {path} matches {q}"))?;
            title = format!("{title} · only {q}");
            (&filtered, root)
        }
        None => (&l.tree, root),
    };
    let out_path = std::path::absolute(&a.output)?;
    let file = std::fs::File::create(&out_path)?;
    match format {
        Format::Csv => bulk_core::export::write_csv(tree, root, file)?,
        Format::Json => bulk_core::export::write_json(tree, root, file)?,
        Format::Html => bulk_core::export::write_html(
            tree,
            root,
            &title,
            now as u32,
            &crate::theme::report(),
            file,
        )?,
    }
    if a.fresh.json {
        return out::json(&serde_json::json!({ "path": out_path }));
    }
    println!("Saved {}", out_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since() {
        let now = 1_800_000_000;
        assert_eq!(when("7d", now).unwrap(), now - 7 * 86_400);
        assert_eq!(when("2026-10-01", now).unwrap(), 1_790_812_800);
        assert!(when("soon", now).is_err());
    }
}
