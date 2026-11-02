//! Commands that only look: drives, top, search, types.

use std::path::PathBuf;

use bulk_core::filter::Filter;
use bulk_core::search;
use bulk_core::tree::{NodeId, Tree};
use bulk_core::volume::{self, DriveKind};
use clap::Args;
use serde::Serialize;

use crate::load::{self, AsOf, Freshness, Loader};
use crate::out::{self, Table};

/// A file or folder in JSON output.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub path: String,
    pub name: String,
    pub dir: bool,
    /// Size on disk, bytes.
    pub alloc: u64,
    pub size: u64,
    /// Files at or below it.
    pub files: u32,
    pub modified: String,
}

pub fn item(tree: &Tree, id: NodeId) -> Item {
    let n = tree.get(id);
    Item {
        path: tree.path(id),
        name: tree.name(id).to_string(),
        dir: n.is_dir(),
        alloc: n.alloc,
        size: n.size,
        files: n.files,
        modified: bulk_core::export::iso(n.mtime),
    }
}

/// Name as listed: folders end in `\`.
fn shown_name(tree: &Tree, id: NodeId) -> String {
    let name = tree.name(id);
    if tree.get(id).is_dir() {
        format!("{name}\\")
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct DrivesArgs {
    /// Print JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriveRow {
    letter: char,
    label: String,
    fs: String,
    kind: DriveKind,
    total: u64,
    free: u64,
    /// When Bulk last saved this drive's tree; null if it never scanned it.
    saved: Option<u64>,
    files: Option<u32>,
}

/// Drives with free space, and when Bulk last saved each one. Reads no trees.
pub fn drives(a: DrivesArgs) -> anyhow::Result<()> {
    let rows: Vec<DriveRow> = volume::list()
        .into_iter()
        .map(|v| {
            let meta =
                bulk_core::snapshot::load_meta(&bulk_core::snapshot::current_path(v.serial)).ok();
            DriveRow {
                letter: v.letter,
                label: v.label,
                fs: v.fs,
                kind: v.kind,
                total: v.total,
                free: v.free,
                saved: meta.as_ref().map(|m| m.taken),
                files: meta.map(|m| m.files),
            }
        })
        .collect();
    if a.json {
        return out::json(&rows);
    }
    let mut t = Table::new(&[false, false, true, true, false, false]);
    for d in &rows {
        let used = d.total.saturating_sub(d.free);
        let frac = if d.total > 0 {
            used as f64 / d.total as f64
        } else {
            0.0
        };
        let label = if d.label.is_empty() {
            match d.kind {
                DriveKind::Removable => "USB drive".to_string(),
                DriveKind::Virtual => "Cloud drive".to_string(),
                DriveKind::Network => "Network drive".to_string(),
                _ => "Local disk".to_string(),
            }
        } else {
            d.label.clone()
        };
        t.row(vec![
            out::bold(&format!("{}:", d.letter)),
            label,
            format!("{} free", out::size(d.free)),
            format!("of {}", out::size(d.total)),
            out::bar(frac, 12),
            match (d.saved, d.files) {
                (Some(s), Some(f)) => out::dim(&format!(
                    "{} files · saved {}",
                    out::count(f as u64),
                    out::ago(s)
                )),
                _ => out::dim("not scanned"),
            },
        ]);
    }
    t.print();
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct TopArgs {
    /// Folder or drive (default: the current folder)
    #[arg(default_value = ".")]
    path: PathBuf,
    /// How many to list
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
    /// Biggest files anywhere below, instead of what's directly inside
    #[arg(long, conflicts_with = "folders")]
    files: bool,
    /// Biggest folders anywhere below, instead of what's directly inside
    #[arg(long)]
    folders: bool,
    /// Only count what matches this search, e.g. "ext:mp4 older:1y"
    #[arg(long, short = 'f')]
    filter: Option<String>,
    #[command(flatten)]
    fresh: Freshness,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TopResult {
    as_of: AsOf,
    #[serde(flatten)]
    folder: Item,
    filter: Option<String>,
    /// How many there are in all; `items` is cut to the limit.
    total: usize,
    items: Vec<Item>,
}

/// What's biggest in a folder: its contents, or files or folders anywhere below.
pub fn top(a: TopArgs) -> anyhow::Result<()> {
    let path = load::absolute(&a.path)?;
    let loader = Loader::new(&a.fresh);
    let (l, root) = loader.path(&path)?;
    let tree = &l.tree;
    let now = bulk_core::os::now();

    // Sizes to show: filtered ones when there's a filter.
    let filter = a.filter.as_deref().map(|q| Filter::new(tree, q, now));
    let val = |id: NodeId| match &filter {
        Some(f) => f.get(id),
        None => {
            let n = tree.get(id);
            bulk_core::filter::Val {
                size: n.size,
                alloc: n.alloc,
                files: n.files,
            }
        }
    };

    let (ids, total): (Vec<NodeId>, usize) = if a.files || a.folders {
        let kind = if a.files { "type:file" } else { "type:dir" };
        let q = search::parse(
            &format!("{kind} sort:size {}", a.filter.as_deref().unwrap_or("")),
            now,
        );
        let (hits, total) = search::search(tree, &q, Some(root), a.limit);
        (hits.into_iter().map(|h| h.id).collect(), total)
    } else {
        let mut kids: Vec<NodeId> = tree
            .children(root)
            .filter(|&c| !tree.get(c).has(bulk_core::tree::flags::GONE))
            .filter(|&c| filter.as_ref().is_none_or(|f| f.shows(c)))
            .collect();
        kids.sort_by(|&x, &y| val(y).alloc.cmp(&val(x).alloc));
        let total = kids.len();
        kids.truncate(a.limit);
        (kids, total)
    };

    let whole = val(root);
    if a.fresh.json {
        let filtered = |id| {
            let mut it = item(tree, id);
            let v = val(id);
            (it.alloc, it.size, it.files) = (v.alloc, v.size, v.files);
            it
        };
        return out::json(&TopResult {
            as_of: load::as_of(&l),
            folder: filtered(root),
            filter: a.filter.clone(),
            total,
            items: ids.iter().map(|&id| filtered(id)).collect(),
        });
    }

    let mut head = format!(
        "{} · {} on disk · {} files · {}",
        out::bold(&tree.path(root)),
        out::size(whole.alloc),
        out::count(whole.files as u64),
        load::freshness(&l)
    );
    if let Some(q) = &a.filter {
        head.push_str(&format!(" · only {q}"));
    }
    println!("{head}");
    let mut t = Table::new(&[true, false, true, true, false]);
    for &id in &ids {
        let v = val(id);
        let frac = if whole.alloc > 0 {
            v.alloc as f64 / whole.alloc as f64
        } else {
            0.0
        };
        let n = tree.get(id);
        let name = if a.files || a.folders {
            rel(tree, root, id)
        } else {
            shown_name(tree, id)
        };
        t.row(vec![
            out::size(v.alloc),
            out::bar(frac, 10),
            out::dim(&format!("{:.1}%", frac * 100.0)),
            out::dim(&if n.is_dir() {
                format!("{} files", out::count(v.files as u64))
            } else {
                out::date(n.mtime)
            }),
            name,
        ]);
    }
    t.print();
    out::more(ids.len(), total);
    Ok(())
}

/// Path of `id` below `base`, e.g. `Videos\clip.mp4`.
pub fn rel(tree: &Tree, base: NodeId, id: NodeId) -> String {
    let mut parts = vec![shown_name(tree, id)];
    for a in tree.ancestors(id) {
        if a == base {
            break;
        }
        parts.push(tree.name(a).to_string());
    }
    parts.reverse();
    parts.join("\\")
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct SearchArgs {
    /// What to look for; see `bulk search --help` for the syntax. Quote parts with
    /// `|`, `>` or `<`, and leave things out with `!` (`!in:temp`), or put `--`
    /// before a query that starts with `-`
    #[arg(required = true, num_args = 1..)]
    query: Vec<String>,
    /// Only look in this folder or drive
    #[arg(long = "in", value_name = "PATH")]
    within: Option<PathBuf>,
    /// How many to list
    #[arg(short = 'n', long, default_value_t = 50)]
    limit: usize,
    #[command(flatten)]
    fresh: Freshness,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    as_of: Vec<AsOf>,
    query: String,
    total: usize,
    hits: Vec<Item>,
}

pub fn search(a: SearchArgs) -> anyhow::Result<()> {
    let text = a.query.join(" ");
    let now = bulk_core::os::now();
    let q = search::parse(&text, now);
    let loader = Loader::new(&a.fresh);

    let within = a.within.as_deref().map(load::absolute).transpose()?;
    let loaded = match &within {
        Some(p) => vec![loader.path(p)?.0],
        None => loader.drives(&q.drives),
    };
    let mut hits: Vec<(u8, usize, NodeId)> = Vec::new();
    let mut total = 0;
    for (i, l) in loaded.iter().enumerate() {
        let scope = within
            .as_ref()
            .map(|p| l.tree.find_path(p).unwrap_or(l.tree.root));
        let (h, n) = search::search(&l.tree, &q, scope, a.limit);
        total += n;
        hits.extend(h.into_iter().map(|h| (h.score, i, h.id)));
    }
    let rank = |&(score, i, id): &(u8, usize, NodeId)| {
        let t = &loaded[i].tree;
        let n = t.get(id);
        search::Rank {
            score,
            alloc: n.alloc,
            mtime: n.mtime,
            files: n.files,
            name: t.name(id),
        }
    };
    hits.sort_by(|a, b| search::compare(q.sort, &rank(a), &rank(b)));
    hits.truncate(a.limit);

    if a.fresh.json {
        return out::json(&SearchResult {
            as_of: loaded.iter().map(load::as_of).collect(),
            query: text,
            total,
            hits: hits
                .iter()
                .map(|&(_, i, id)| item(&loaded[i].tree, id))
                .collect(),
        });
    }
    let mut t = Table::new(&[true, false, false]);
    for &(_, i, id) in &hits {
        let tree = &loaded[i].tree;
        let n = tree.get(id);
        let path = tree.path(id);
        t.row(vec![
            out::size(n.alloc),
            out::dim(&out::date(n.mtime)),
            if n.is_dir() {
                format!("{path}\\")
            } else {
                path
            },
        ]);
    }
    t.print();
    if hits.is_empty() {
        println!("{}", out::dim("Nothing matches."));
    }
    out::more(hits.len(), total);
    Ok(())
}

// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct TypesArgs {
    /// Folder or drive (default: the current folder)
    #[arg(default_value = ".")]
    path: PathBuf,
    /// One line per extension instead of per kind of file
    #[arg(long)]
    ext: bool,
    /// How many to list
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
    #[command(flatten)]
    fresh: Freshness,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TypesResult {
    as_of: AsOf,
    path: String,
    #[serde(flatten)]
    breakdown: bulk_core::kinds::Breakdown,
}

/// Space by kind of file (videos, archives…) or by extension.
pub fn types(a: TypesArgs) -> anyhow::Result<()> {
    let path = load::absolute(&a.path)?;
    let loader = Loader::new(&a.fresh);
    let (l, root) = loader.path(&path)?;
    let mut b = bulk_core::kinds::breakdown(&l.tree, root);
    if a.fresh.json {
        b.exts.truncate(a.limit.max(500));
        return out::json(&TypesResult {
            as_of: load::as_of(&l),
            path: l.tree.path(root),
            breakdown: b,
        });
    }
    println!(
        "{} · {} in {} files · {}",
        out::bold(&l.tree.path(root)),
        out::size(b.alloc),
        out::count(b.files),
        load::freshness(&l)
    );
    let frac = |x: u64| {
        if b.alloc > 0 {
            x as f64 / b.alloc as f64
        } else {
            0.0
        }
    };
    let mut t = Table::new(&[false, true, false, true, false]);
    if a.ext {
        for e in b.exts.iter().take(a.limit) {
            t.row(vec![
                if e.ext.is_empty() {
                    "(none)".into()
                } else {
                    format!(".{}", e.ext)
                },
                out::size(e.alloc),
                out::bar(frac(e.alloc), 10),
                out::dim(&format!("{} files", out::count(e.files))),
                out::dim(e.kind),
            ]);
        }
        t.print();
        out::more(a.limit.min(b.exts.len()), b.exts.len());
    } else {
        for k in b.kinds.iter().take(a.limit) {
            let exts: Vec<String> = k.exts.iter().take(5).map(|e| format!(".{e}")).collect();
            t.row(vec![
                k.label.to_string(),
                out::size(k.alloc),
                out::bar(frac(k.alloc), 10),
                out::dim(&format!("{} files", out::count(k.files))),
                out::dim(&exts.join(" ")),
            ]);
        }
        t.print();
    }
    Ok(())
}
