//! Name search over a whole tree, with a filter language. Every term must match
//! (AND); `|` or `OR` gives alternatives; `-` or `!` in front excludes.
//!
//! - `holiday 2023` words anywhere in the name; `"my notes"` keeps spaces
//! - `IMG_2024*.jpg` wildcards (`*`, `?`) match the whole name
//! - `downloads/report` path pieces, folders in order (`C:/Users/…` starts at a drive)
//! - `ext:mp4,mkv` / `ext:xls*` extension, `ext:none` files without one
//! - `kind:video` (or `type:video`) a group of extensions, see `kinds`
//! - `size:>1GB`, `size:<10MB`, `size:1GB..5GB` size on disk
//! - `modified:2024`, `modified:>2024-06`, `modified:<30d`, `older:1y`, `newer:7d`,
//!   `before:2023-06`, `after:2024`
//! - `type:file` / `type:dir`; `is:empty`, `is:hidden`, `is:system`, `is:new`
//! - `files:>1000` folders holding that many files
//! - `in:downloads` some parent folder's name contains the text
//! - `under:"C:\Users\me\Downloads"` inside that exact folder
//! - `drive:c,d` only those drives
//! - `sort:size`, `sort:small`, `sort:name`, `sort:newest`, `sort:oldest`, `sort:files`

use std::collections::HashMap;

use memchr::memmem::Finder;
use rayon::prelude::*;

use crate::kinds;
use crate::tree::{NodeId, Tree, flags};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Relevance,
    Size,
    Small,
    Name,
    Newest,
    Oldest,
    Files,
}

#[derive(Debug, Default, Clone)]
pub struct Query {
    terms: Vec<Term>,
    pub sort: Sort,
    /// Drives to search, from `drive:` or a path starting with `C:`. Empty is all.
    pub drives: Vec<char>,
}

#[derive(Debug, Clone)]
struct Term {
    neg: bool,
    /// Alternatives; any one matching is enough.
    any: Vec<Cond>,
}

#[derive(Debug, Clone)]
enum Cond {
    Name(Pat),
    Path {
        segs: Vec<Pat>,
        anchored: bool,
    },
    Ext(Vec<Pat>),
    NoExt,
    Kind(Vec<usize>),
    Size(Option<u64>, Option<u64>),
    /// Modified in `[after, before)`, unix seconds.
    Time(Option<u32>, Option<u32>),
    Files(Option<u64>, Option<u64>),
    Dir(bool),
    Flag(u16),
    Empty,
    Within(Pat),
    Under(String),
    /// A filter that can never match, e.g. an unknown kind.
    Never,
}

#[derive(Debug, Clone)]
enum Pat {
    /// Anywhere in the text.
    Sub(Finder<'static>, usize),
    /// The whole text, with `*` and `?`.
    Glob(Vec<u8>),
    Exact(Vec<u8>),
    Any,
}

impl Pat {
    fn new(text: &str, exact: bool) -> Pat {
        let t = text.to_ascii_lowercase();
        if t.is_empty() {
            Pat::Any
        } else if t.contains(['*', '?']) {
            Pat::Glob(t.into_bytes())
        } else if exact {
            Pat::Exact(t.into_bytes())
        } else {
            let len = t.len();
            Pat::Sub(Finder::new(t.as_bytes()).into_owned(), len)
        }
    }

    /// How well `name` matches: 4 exact, 3 prefix, 2 word start, 1 elsewhere.
    fn score(&self, name: &[u8]) -> Option<u8> {
        match self {
            Pat::Any => Some(1),
            Pat::Exact(e) => (name == e.as_slice()).then_some(4),
            Pat::Glob(g) => glob(g, name).then_some(3),
            Pat::Sub(f, len) => {
                let pos = f.find(name)?;
                Some(if name.len() == *len {
                    4
                } else if pos == 0 {
                    3
                } else if !name[pos - 1].is_ascii_alphanumeric() {
                    2
                } else {
                    1
                })
            }
        }
    }

    fn matches(&self, name: &[u8]) -> bool {
        self.score(name).is_some()
    }
}

/// Case-sensitive wildcard match of the whole `text` (callers lowercase both).
fn glob(pat: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|&c| c == b'*')
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

/// Split on spaces outside double quotes, dropping the quotes.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Keys whose values can be listed after `|` without repeating the key.
fn listable(key: &str) -> bool {
    matches!(key, "ext" | "kind" | "type" | "in" | "is" | "name")
}

/// Parse a query. `now` is unix seconds, used for relative ages.
pub fn parse(text: &str, now: u64) -> Query {
    let mut q = Query::default();
    let mut join = false;
    for tok in tokens(text) {
        if tok == "|" || tok == "OR" {
            join = !q.terms.is_empty();
            continue;
        }
        let (neg, body) = match tok.strip_prefix(['-', '!']) {
            Some(rest) if !rest.is_empty() => (true, rest),
            _ => (false, tok.as_str()),
        };
        let mut any = Vec::new();
        let mut last_key: Option<String> = None;
        for part in body.split('|').filter(|p| !p.is_empty()) {
            let (key, val) = match part.split_once(':') {
                Some((k, v)) if is_key(&k.to_ascii_lowercase()) => {
                    (Some(k.to_ascii_lowercase()), v)
                }
                _ => match &last_key {
                    Some(k) if listable(k) => (Some(k.clone()), part),
                    _ => (None, part),
                },
            };
            match key {
                Some(k) => {
                    if let Some(c) = keyed(&mut q, &k, val, now) {
                        any.push(c);
                    }
                    last_key = Some(k);
                }
                None => any.push(bare(&mut q, val, neg)),
            }
        }
        if any.is_empty() {
            continue;
        }
        if join && let Some(prev) = q.terms.last_mut() {
            prev.any.extend(any);
        } else {
            q.terms.push(Term { neg, any });
        }
        join = false;
    }
    // Cheap checks on the node itself go first; ancestor walks only for survivors.
    q.terms.sort_by_key(|t| {
        t.any
            .iter()
            .any(|c| matches!(c, Cond::Path { .. } | Cond::Within(_) | Cond::Under(_)))
    });
    q
}

fn is_key(k: &str) -> bool {
    matches!(
        k,
        "ext"
            | "kind"
            | "size"
            | "older"
            | "newer"
            | "before"
            | "after"
            | "modified"
            | "date"
            | "type"
            | "is"
            | "in"
            | "under"
            | "path"
            | "name"
            | "files"
            | "drive"
            | "sort"
    )
}

/// A plain word, wildcard or path.
fn bare(q: &mut Query, text: &str, neg: bool) -> Cond {
    if !text.contains(['/', '\\']) {
        return Cond::Name(Pat::new(text, false));
    }
    let raw: Vec<&str> = text.split(['/', '\\']).collect();
    let anchored = raw[0].len() == 2 && raw[0].ends_with(':');
    if anchored && !neg {
        let l = raw[0].chars().next().unwrap().to_ascii_uppercase();
        if !q.drives.contains(&l) {
            q.drives.push(l);
        }
    }
    let last = raw.len() - 1;
    let segs = raw
        .iter()
        .enumerate()
        // Folders in a path starting at a drive are whole names.
        .filter(|(i, s)| !s.is_empty() || *i == last)
        .map(|(i, s)| Pat::new(s, anchored && i != last))
        .collect();
    Cond::Path { segs, anchored }
}

fn keyed(q: &mut Query, key: &str, val: &str, now: u64) -> Option<Cond> {
    let lower = val.to_ascii_lowercase();
    Some(match key {
        "ext" if lower == "none" => Cond::NoExt,
        "ext" => Cond::Ext(
            val.split(',')
                .map(|e| e.trim_start_matches('.'))
                .filter(|e| !e.is_empty())
                .map(|e| Pat::new(e, true))
                .collect(),
        ),
        "kind" => kind(&lower),
        "type" | "is" => match lower.as_str() {
            "dir" | "dirs" | "folder" | "folders" | "d" => Cond::Dir(true),
            "file" | "files" | "f" => Cond::Dir(false),
            "empty" => Cond::Empty,
            "hidden" => Cond::Flag(flags::HIDDEN),
            "system" => Cond::Flag(flags::SYSTEM),
            "new" => Cond::Flag(flags::NEW),
            "compressed" => Cond::Flag(flags::COMPRESSED),
            "link" | "reparse" => Cond::Flag(flags::REPARSE),
            _ => kind(&lower),
        },
        "size" => {
            let (a, b) = range(&lower, parse_size)?;
            Cond::Size(a, b)
        }
        "files" => {
            let (a, b) = range(&lower, |s| s.parse().ok())?;
            Cond::Files(a, b)
        }
        "older" => Cond::Time(None, Some(now.saturating_sub(parse_age(&lower)?) as u32)),
        "newer" => Cond::Time(Some(now.saturating_sub(parse_age(&lower)?) as u32), None),
        "before" => Cond::Time(None, Some(parse_date(&lower)?.0)),
        "after" => Cond::Time(Some(parse_date(&lower)?.0), None),
        "modified" | "date" => modified(&lower, now)?,
        "in" => Cond::Within(Pat::new(val, false)),
        "under" => Cond::Under(
            val.trim_end_matches(['\\', '/'])
                .replace('/', "\\")
                .to_ascii_lowercase(),
        ),
        "path" => bare(q, &format!("{}/", val.trim_end_matches(['/', '\\'])), false),
        "name" => Cond::Name(Pat::new(val, false)),
        "drive" => {
            for l in lower.split(',').filter_map(|d| d.chars().next()) {
                let l = l.to_ascii_uppercase();
                if l.is_ascii_alphabetic() && !q.drives.contains(&l) {
                    q.drives.push(l);
                }
            }
            return None;
        }
        "sort" => {
            q.sort = match lower.as_str() {
                "size" | "big" | "biggest" | "largest" => Sort::Size,
                "-size" | "small" | "smallest" => Sort::Small,
                "name" => Sort::Name,
                "date" | "new" | "newest" | "recent" | "modified" => Sort::Newest,
                "old" | "oldest" => Sort::Oldest,
                "files" | "count" => Sort::Files,
                _ => Sort::Relevance,
            };
            return None;
        }
        _ => return None,
    })
}

fn kind(text: &str) -> Cond {
    let ks: Vec<usize> = text.split(',').filter_map(kinds::find_kind).collect();
    if ks.is_empty() {
        Cond::Never
    } else {
        Cond::Kind(ks)
    }
}

/// `>X`, `>=X`, `<X`, `<=X`, `X..Y`, or `X` (at least X).
fn range<T>(s: &str, f: impl Fn(&str) -> Option<T>) -> Option<(Option<T>, Option<T>)> {
    Some(if let Some((a, b)) = s.split_once("..") {
        (f(a), f(b))
    } else if let Some(v) = s.strip_prefix(">=").or(s.strip_prefix('>')) {
        (f(v), None)
    } else if let Some(v) = s.strip_prefix("<=").or(s.strip_prefix('<')) {
        (None, f(v))
    } else {
        (Some(f(s)?), None)
    })
}

/// `modified:` takes a date (`2024`, `>2024-06`, `2024..2025-03`) or an age (`<30d`
/// is within the last 30 days, `>1y` more than a year ago).
fn modified(s: &str, now: u64) -> Option<Cond> {
    let age = |v: &str| parse_age(v).map(|a| now.saturating_sub(a) as u32);
    if let Some(v) = s.strip_prefix(">=").or(s.strip_prefix('>')) {
        return Some(match age(v) {
            Some(t) => Cond::Time(None, Some(t)),
            None => Cond::Time(Some(parse_date(v)?.1), None),
        });
    }
    if let Some(v) = s.strip_prefix("<=").or(s.strip_prefix('<')) {
        return Some(match age(v) {
            Some(t) => Cond::Time(Some(t), None),
            None => Cond::Time(None, Some(parse_date(v)?.0)),
        });
    }
    if let Some((a, b)) = s.split_once("..") {
        return Some(Cond::Time(
            parse_date(a).map(|d| d.0),
            parse_date(b).map(|d| d.1),
        ));
    }
    if let Some(t) = age(s) {
        return Some(Cond::Time(Some(t), None));
    }
    let (a, b) = parse_date(s)?;
    Some(Cond::Time(Some(a), Some(b)))
}

/// `1.5GB`, `200mb`, `4096` (bytes).
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mul = match unit.trim_end_matches("ib").trim_end_matches('b') {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024f64.powi(2),
        "g" => 1024f64.powi(3),
        "t" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mul) as u64)
}

/// `30d`, `2w`, `6m`, `1y`, `12h` -> seconds.
fn parse_age(s: &str) -> Option<u64> {
    let (num, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: u64 = num.parse().ok()?;
    let day = 86_400;
    Some(
        n * match unit {
            "h" => 3600,
            "d" => day,
            "w" => 7 * day,
            "m" => 30 * day,
            "y" => 365 * day,
            _ => return None,
        },
    )
}

/// `2023`, `2023-06`, `2023-06-15` -> unix seconds at the start of that period and
/// at the start of the next one.
fn parse_date(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    let y: i64 = parts.first()?.parse().ok()?;
    let m: i64 = parts.get(1).map_or(Some(1), |v| v.parse().ok())?;
    let d: i64 = parts.get(2).map_or(Some(1), |v| v.parse().ok())?;
    if !(1970..=2105).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let start = days(y, m, d);
    let end = match parts.len() {
        1 => days(y + 1, 1, 1),
        2 if m == 12 => days(y + 1, 1, 1),
        2 => days(y, m + 1, 1),
        _ => start + 1,
    };
    Some(((start * 86_400) as u32, (end * 86_400) as u32))
}

/// Days from civil (Howard Hinnant).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub id: NodeId,
    pub score: u8,
}

/// Per-tree lookups: `under:` folders found by path.
struct Ctx<'a> {
    tree: &'a Tree,
    under: HashMap<String, Option<NodeId>>,
    scope: Option<NodeId>,
}

/// Find matching nodes under `scope` (or the whole tree), best first. Returns at
/// most `limit` hits and the total number of matches.
pub fn search(tree: &Tree, q: &Query, scope: Option<NodeId>, limit: usize) -> (Vec<Hit>, usize) {
    if q.is_empty() {
        return (Vec::new(), 0);
    }
    let mut under = HashMap::new();
    for t in &q.terms {
        for c in &t.any {
            if let Cond::Under(p) = c {
                under.insert(p.clone(), tree.find_path(p));
            }
        }
    }
    // A small folder: look only at what's in it. A big one is quicker to search
    // whole, checking each match's ancestors.
    let small = |s: NodeId| (tree.get(s).files as usize) < tree.len() / 8;
    let within: Option<Vec<NodeId>> = scope.filter(|&s| s != tree.root && small(s)).map(|s| {
        let mut ids = Vec::new();
        let mut stack = vec![s];
        while let Some(id) = stack.pop() {
            for c in tree.children_all(id) {
                ids.push(c);
                stack.push(c);
            }
        }
        ids
    });
    let ctx = Ctx {
        tree,
        under,
        scope: scope.filter(|_| within.is_none()),
    };
    let ids: Box<dyn Fn(usize) -> NodeId + Sync> = match &within {
        Some(v) => Box::new(|i| v[i]),
        None => Box::new(|i| (i + 1) as NodeId),
    };
    let n = within
        .as_ref()
        .map_or(tree.len().saturating_sub(1), Vec::len);

    let keep = limit.max(1);
    let (mut hits, total) = (0..n)
        .into_par_iter()
        .with_min_len(4096)
        .fold(
            || (Vec::new(), 0usize),
            |(mut acc, mut count), i| {
                let id = ids(i);
                if let Some(score) = matches(&ctx, id, q) {
                    count += 1;
                    acc.push(Hit { id, score });
                    // Keep per-thread lists small: cut to the best `limit` now and then,
                    // without sorting them.
                    if acc.len() >= keep * 4 {
                        best(tree, q.sort, &mut acc, keep);
                    }
                }
                (acc, count)
            },
        )
        .reduce(
            || (Vec::new(), 0),
            |(mut a, ca), (b, cb)| {
                a.extend(b);
                (a, ca + cb)
            },
        );
    best(tree, q.sort, &mut hits, keep);
    rank(tree, q.sort, &mut hits);
    hits.truncate(limit);
    (hits, total)
}

/// Every node matching `q`, unordered.
pub fn matching(tree: &Tree, q: &Query) -> Vec<NodeId> {
    if q.is_empty() {
        return Vec::new();
    }
    let ctx = Ctx {
        tree,
        under: q
            .terms
            .iter()
            .flat_map(|t| &t.any)
            .filter_map(|c| match c {
                Cond::Under(p) => Some((p.clone(), tree.find_path(p))),
                _ => None,
            })
            .collect(),
        scope: None,
    };
    (1..tree.len())
        .into_par_iter()
        .with_min_len(4096)
        .filter_map(|i| matches(&ctx, i as NodeId, q).map(|_| i as NodeId))
        .collect()
}

/// What hits from several trees are merged by (see `compare`).
pub struct Rank<'a> {
    pub score: u8,
    pub alloc: u64,
    pub mtime: u32,
    pub files: u32,
    pub name: &'a str,
}

/// Order for hits from different drives, matching the order within one.
pub fn compare(sort: Sort, a: &Rank, b: &Rank) -> std::cmp::Ordering {
    match sort {
        Sort::Relevance => b.score.cmp(&a.score).then(b.alloc.cmp(&a.alloc)),
        Sort::Size => b.alloc.cmp(&a.alloc),
        Sort::Small => a.alloc.cmp(&b.alloc),
        Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        Sort::Newest => b.mtime.cmp(&a.mtime),
        Sort::Oldest => a.mtime.cmp(&b.mtime),
        Sort::Files => b.files.cmp(&a.files),
    }
}

/// Keep the best `n` of `hits`, in no particular order.
fn best(tree: &Tree, sort: Sort, hits: &mut Vec<Hit>, n: usize) {
    if hits.len() > n {
        hits.select_nth_unstable_by(n - 1, |a, b| order(tree, sort, a, b));
        hits.truncate(n);
    }
}

fn rank(tree: &Tree, sort: Sort, hits: &mut [Hit]) {
    hits.sort_unstable_by(|a, b| order(tree, sort, a, b));
}

fn order(tree: &Tree, sort: Sort, a: &Hit, b: &Hit) -> std::cmp::Ordering {
    let size = |h: &Hit| tree.get(h.id).alloc;
    let mtime = |h: &Hit| tree.get(h.id).mtime;
    {
        match sort {
            Sort::Relevance => b.score.cmp(&a.score).then_with(|| size(b).cmp(&size(a))),
            Sort::Size => size(b).cmp(&size(a)),
            Sort::Small => size(a).cmp(&size(b)),
            Sort::Name => tree.name_lower(a.id).cmp(tree.name_lower(b.id)),
            Sort::Newest => mtime(b).cmp(&mtime(a)),
            Sort::Oldest => mtime(a).cmp(&mtime(b)),
            Sort::Files => tree.get(b.id).files.cmp(&tree.get(a.id).files),
        }
        .then(a.id.cmp(&b.id))
    }
}

fn matches(ctx: &Ctx, id: NodeId, q: &Query) -> Option<u8> {
    let tree = ctx.tree;
    let n = tree.get(id);
    if n.has(flags::DELETED) || n.has(flags::GONE) || n.has(flags::EXTRA_LINK) {
        return None;
    }
    let mut score = 1u8;
    for t in &q.terms {
        let best = t.any.iter().filter_map(|c| cond(ctx, id, c)).max();
        match (best, t.neg) {
            (Some(s), false) => score = score.max(s),
            (None, true) => {}
            _ => return None,
        }
    }
    if let Some(s) = ctx.scope
        && !tree.is_ancestor(s, id)
    {
        return None;
    }
    Some(score)
}

/// Whether `id` meets one condition, and how well (for name matches).
fn cond(ctx: &Ctx, id: NodeId, c: &Cond) -> Option<u8> {
    let tree = ctx.tree;
    let n = tree.get(id);
    let name = tree.name_lower(id);
    let yes = |b: bool| b.then_some(1);
    match c {
        Cond::Name(p) => p.score(name),
        Cond::Path { segs, anchored } => path(tree, id, segs, *anchored),
        Cond::Ext(pats) => {
            let ext = kinds::ext_of(name).filter(|_| !n.is_dir())?;
            yes(pats.iter().any(|p| p.matches(ext)))
        }
        Cond::NoExt => yes(!n.is_dir() && kinds::ext_of(name).is_none()),
        Cond::Kind(ks) => yes(!n.is_dir() && ks.contains(&kinds::kind_of(name))),
        Cond::Size(a, b) => yes(a.is_none_or(|m| n.alloc >= m) && b.is_none_or(|m| n.alloc <= m)),
        Cond::Time(a, b) => yes(a.is_none_or(|t| n.mtime >= t) && b.is_none_or(|t| n.mtime < t)),
        Cond::Files(a, b) => yes(n.is_dir()
            && a.is_none_or(|m| n.files as u64 >= m)
            && b.is_none_or(|m| n.files as u64 <= m)),
        Cond::Dir(d) => yes(n.is_dir() == *d),
        Cond::Flag(f) => yes(n.has(*f)),
        Cond::Empty => yes(if n.is_dir() {
            n.files == 0
        } else {
            n.size == 0
        }),
        Cond::Within(p) => yes(tree.ancestors(id).any(|a| p.matches(tree.name_lower(a)))),
        Cond::Under(p) => {
            let folder = ctx.under.get(p).copied().flatten()?;
            yes(folder != id && tree.is_ancestor(folder, id))
        }
        Cond::Never => None,
    }
}

/// Last piece against the name; the folders before it against ancestors, in order
/// (nearest last). Anchored paths start at the drive root and match whole names.
fn path(tree: &Tree, id: NodeId, segs: &[Pat], anchored: bool) -> Option<u8> {
    let (last, folders) = segs.split_last()?;
    let score = last.score(tree.name_lower(id))?;
    if anchored {
        let mut chain: Vec<NodeId> = tree.ancestors(id).collect();
        chain.reverse();
        let ok = chain.len() >= folders.len()
            && folders
                .iter()
                .zip(&chain)
                .all(|(p, &a)| p.matches(tree.name_lower(a)));
        return ok.then_some(score);
    }
    let mut want = folders.len();
    for a in tree.ancestors(id) {
        if want == 0 {
            break;
        }
        if folders[want - 1].matches(tree.name_lower(a)) {
            want -= 1;
        }
    }
    (want == 0).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Tree {
        let mut t = Tree::new("C:");
        let users = t.add(t.root, "Users", flags::DIR, 0, 0, 0, 0);
        let dl = t.add(users, "Downloads", flags::DIR, 0, 0, 0, 0);
        let v = t.add(t.root, "Videos", flags::DIR, 0, 0, 0, 0);
        t.add(v, "holiday.mp4", 0, 10, 100, 1_700_000_000, 0);
        t.add(v, "my holiday clip.mp4", 0, 10, 900, 1_700_000_000, 0);
        t.add(t.root, "holiday.txt", 0, 10, 5000, 1_600_000_000, 0);
        t.add(dl, "labels_2026_01.pdf", 0, 10, 300, 0, 0);
        t.add(dl, "labels_2025_12.pdf", 0, 10, 300, 0, 0);
        t.add(dl, "labels_2026_02.docx", 0, 10, 300, 0, 0);
        t.add(dl, "report final.xlsx", 0, 10, 300, 0, 0);
        t.add(t.root, "empty", flags::DIR, 0, 0, 0, 0);
        t.aggregate();
        t
    }

    fn names(t: &Tree, q: &str) -> Vec<String> {
        let (hits, _) = search(t, &parse(q, 2_000_000_000), None, 50);
        let mut v: Vec<String> = hits.iter().map(|h| t.name(h.id).to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn parses_filters() {
        let q = parse(
            "holiday ext:mp4,.MKV size:>1.5GB older:30d type:file in:Videos sort:size drive:d",
            100 * 86_400,
        );
        assert_eq!(q.terms.len(), 6);
        assert_eq!(q.sort, Sort::Size);
        assert_eq!(q.drives, vec!['D']);
        assert_eq!(parse_date("2024"), Some((1_704_067_200, 1_735_689_600)));
        assert!(parse("sort:size", 0).is_empty());
    }

    #[test]
    fn finds_and_ranks() {
        let t = tree();
        let (hits, total) = search(&t, &parse("holiday ext:mp4", 0), None, 10);
        assert_eq!(total, 2);
        assert_eq!(t.name(hits[0].id), "holiday.mp4"); // prefix beats bigger mid-name match
        assert_eq!(names(&t, "holiday in:videos").len(), 2);
        // Videos is 1000 bytes on disk, so only the text file and itself clear 1 KB.
        assert_eq!(names(&t, "size:>1KB type:file"), vec!["holiday.txt"]);
    }

    #[test]
    fn globs_paths_and_logic() {
        let t = tree();
        assert_eq!(names(&t, "labels_2026*.pdf"), vec!["labels_2026_01.pdf"]);
        assert_eq!(
            names(&t, "labels_202?_*"),
            vec![
                "labels_2025_12.pdf",
                "labels_2026_01.pdf",
                "labels_2026_02.docx"
            ]
        );
        assert_eq!(names(&t, "Downloads/report").len(), 1);
        assert_eq!(names(&t, r"users\downloads\labels").len(), 3);
        assert_eq!(names(&t, "C:/Users/Downloads/labels_2026").len(), 2);
        assert_eq!(names(&t, "C:/Downloads/labels").len(), 0);
        assert_eq!(names(&t, "downloads/").len(), 4);
        assert_eq!(names(&t, "labels -docx").len(), 2);
        assert_eq!(
            names(&t, "labels ext:pdf|docx -2025"),
            vec!["labels_2026_01.pdf", "labels_2026_02.docx"]
        );
        assert_eq!(names(&t, "holiday OR report").len(), 4);
        assert_eq!(names(&t, "\"report final\""), vec!["report final.xlsx"]);
        assert_eq!(names(&t, "kind:spreadsheet"), vec!["report final.xlsx"]);
        assert_eq!(
            names(&t, "type:video"),
            vec!["holiday.mp4", "my holiday clip.mp4"]
        );
        assert_eq!(names(&t, "is:empty type:dir"), vec!["empty"]);
        assert_eq!(names(&t, "modified:2023 holiday").len(), 2);
        assert_eq!(names(&t, "under:C:/Users ext:pdf").len(), 2);
        assert_eq!(names(&t, "kind:nonsense").len(), 0);
        assert_eq!(names(&t, "ext:none"), Vec::<String>::new());
    }

    #[test]
    fn scope_and_sort() {
        let t = tree();
        let dl = t.find_path(r"C:\Users\Downloads").unwrap();
        let (hits, total) = search(&t, &parse("labels sort:name", 0), Some(dl), 10);
        assert_eq!(total, 3);
        assert_eq!(t.name(hits[0].id), "labels_2025_12.pdf");
        assert_eq!(search(&t, &parse("holiday", 0), Some(dl), 10).1, 0);
    }

    #[test]
    fn small_and_big_folders_find_the_same() {
        let mut t = Tree::new("C:");
        let big = t.add(t.root, "big", flags::DIR, 0, 0, 0, 0);
        for i in 0..200 {
            t.add(big, &format!("f{i}.txt"), 0, 1, 1, 0, 0);
        }
        let small = t.add(t.root, "small", flags::DIR, 0, 0, 0, 0);
        let sub = t.add(small, "sub", flags::DIR, 0, 0, 0, 0);
        t.add(sub, "f1.txt", 0, 1, 1, 0, 0);
        t.add(small, "other.txt", 0, 1, 1, 0, 0);
        t.aggregate();
        let q = parse("f1", 0);
        // A small folder is searched by walking it; a big one by ancestor checks.
        let (hits, total) = search(&t, &q, Some(small), 10);
        assert_eq!(total, 1);
        assert_eq!(t.path(hits[0].id), r"C:\small\sub\f1.txt");
        let (_, total) = search(&t, &q, Some(big), 1000);
        assert_eq!(total, 111, "f1, f10..f19 and f100..f199");
        assert!(search(&t, &q, Some(big), 3).0.len() == 3);
    }
}
