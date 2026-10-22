//! Name search over a whole tree, with a small filter language:
//!
//! - plain words must all appear in the name (`holiday 2023`)
//! - `ext:mp4,mkv` file extension
//! - `size:>1GB`, `size:<10MB`, `size:1GB..5GB` size on disk
//! - `older:90d` / `newer:7d` not modified within / modified within (d, w, m, y)
//! - `before:2023-06` / `after:2024` modification date
//! - `type:file` / `type:dir`
//! - `in:downloads` some parent folder's name contains the text

use rayon::prelude::*;

use crate::tree::{NodeId, Tree, flags};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Query {
    pub words: Vec<Vec<u8>>,
    pub exts: Vec<Vec<u8>>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Unix seconds bounds on mtime.
    pub after: Option<u32>,
    pub before: Option<u32>,
    pub dirs: Option<bool>,
    pub within: Vec<Vec<u8>>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }
}

/// Parse a query. `now` is unix seconds, used for relative ages.
pub fn parse(text: &str, now: u64) -> Query {
    let mut q = Query::default();
    for tok in text.split_whitespace() {
        let (key, val) = match tok.split_once(':') {
            Some((k, v)) if !v.is_empty() => (k.to_ascii_lowercase(), v),
            _ => {
                q.words.push(tok.to_ascii_lowercase().into_bytes());
                continue;
            }
        };
        match key.as_str() {
            "ext" => q.exts.extend(
                val.split(',')
                    .map(|e| e.trim_start_matches('.').to_ascii_lowercase().into_bytes())
                    .filter(|e| !e.is_empty()),
            ),
            "size" => {
                if let Some((a, b)) = val.split_once("..") {
                    q.min_size = parse_size(a);
                    q.max_size = parse_size(b);
                } else if let Some(v) = val.strip_prefix(">=").or(val.strip_prefix('>')) {
                    q.min_size = parse_size(v);
                } else if let Some(v) = val.strip_prefix("<=").or(val.strip_prefix('<')) {
                    q.max_size = parse_size(v);
                } else {
                    q.min_size = parse_size(val);
                }
            }
            "older" => q.before = parse_age(val).map(|a| now.saturating_sub(a) as u32),
            "newer" => q.after = parse_age(val).map(|a| now.saturating_sub(a) as u32),
            "before" => q.before = parse_date(val),
            "after" => q.after = parse_date(val),
            "type" | "is" => match val.to_ascii_lowercase().as_str() {
                "dir" | "folder" | "d" => q.dirs = Some(true),
                "file" | "f" => q.dirs = Some(false),
                _ => {}
            },
            "in" => q.within.push(val.to_ascii_lowercase().into_bytes()),
            // Unknown key: treat the whole token as a word (e.g. "C:" or "a:b").
            _ => q.words.push(tok.to_ascii_lowercase().into_bytes()),
        }
    }
    q
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

/// `30d`, `2w`, `6m`, `1y` -> seconds.
fn parse_age(s: &str) -> Option<u64> {
    let s = s.to_ascii_lowercase();
    let (num, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: u64 = num.parse().ok()?;
    let day = 86_400;
    Some(
        n * match unit {
            "d" => day,
            "w" => 7 * day,
            "m" => 30 * day,
            "y" => 365 * day,
            _ => return None,
        },
    )
}

/// `2023`, `2023-06`, `2023-06-15` -> unix seconds at the start of that period.
fn parse_date(s: &str) -> Option<u32> {
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next().map_or(Some(1), |v| v.parse().ok())?;
    let d: i64 = it.next().map_or(Some(1), |v| v.parse().ok())?;
    if !(1970..=2105).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400) as u32)
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub id: NodeId,
    pub score: u8,
}

/// Find matching nodes, best first (name match quality, then size on disk).
/// Returns at most `limit` hits and the total number of matches.
pub fn search(tree: &Tree, q: &Query, limit: usize) -> (Vec<Hit>, usize) {
    if q.is_empty() {
        return (Vec::new(), 0);
    }
    let finders: Vec<memchr::memmem::Finder> =
        q.words.iter().map(memchr::memmem::Finder::new).collect();
    let n = tree.len();

    let (mut hits, total) = (1..n)
        .into_par_iter()
        .with_min_len(4096)
        .fold(
            || (Vec::new(), 0usize),
            |(mut acc, mut count), i| {
                let id = i as NodeId;
                if let Some(score) = matches(tree, id, q, &finders) {
                    count += 1;
                    acc.push(Hit { id, score });
                    // Keep per-thread lists small: trim to the best `limit` now and then.
                    if acc.len() >= limit * 4 {
                        rank(tree, &mut acc);
                        acc.truncate(limit);
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
    rank(tree, &mut hits);
    hits.truncate(limit);
    (hits, total)
}

fn rank(tree: &Tree, hits: &mut [Hit]) {
    hits.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| tree.get(b.id).alloc.cmp(&tree.get(a.id).alloc))
            .then(a.id.cmp(&b.id))
    });
}

fn matches(tree: &Tree, id: NodeId, q: &Query, finders: &[memchr::memmem::Finder]) -> Option<u8> {
    let n = tree.get(id);
    if n.has(flags::DELETED) || n.has(flags::EXTRA_LINK) {
        return None;
    }
    let is_dir = n.is_dir();
    if q.dirs.is_some_and(|d| d != is_dir) {
        return None;
    }
    if q.min_size.is_some_and(|m| n.alloc < m) || q.max_size.is_some_and(|m| n.alloc > m) {
        return None;
    }
    if q.after.is_some_and(|t| n.mtime < t) || q.before.is_some_and(|t| n.mtime >= t) {
        return None;
    }
    let name = tree.name_lower(id);
    if !q.exts.is_empty() {
        if is_dir {
            return None;
        }
        let dot = name.iter().rposition(|&b| b == b'.')?;
        let ext = &name[dot + 1..];
        if !q.exts.iter().any(|e| e == ext) {
            return None;
        }
    }
    let mut score = 1u8;
    for (w, f) in q.words.iter().zip(finders) {
        let pos = f.find(name)?;
        let s = if name.len() == w.len() {
            4 // exact
        } else if pos == 0 {
            3 // prefix
        } else if !name[pos - 1].is_ascii_alphanumeric() {
            2 // start of a word
        } else {
            1
        };
        score = score.max(s);
    }
    if !q.within.is_empty()
        && !q.within.iter().all(|w| {
            let f = memchr::memmem::Finder::new(w);
            tree.ancestors(id)
                .any(|a| f.find(tree.name_lower(a)).is_some())
        })
    {
        return None;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filters() {
        let q = parse(
            "holiday ext:mp4,.MKV size:>1.5GB older:30d type:file in:Videos",
            100 * 86_400,
        );
        assert_eq!(q.words, vec![b"holiday".to_vec()]);
        assert_eq!(q.exts, vec![b"mp4".to_vec(), b"mkv".to_vec()]);
        assert_eq!(q.min_size, Some((1.5 * 1024f64.powi(3)) as u64));
        assert_eq!(q.before, Some(70 * 86_400));
        assert_eq!(q.dirs, Some(false));
        assert_eq!(q.within, vec![b"videos".to_vec()]);
        assert_eq!(parse("after:2024", 0).after, Some(1_704_067_200));
        assert_eq!(parse("size:1MB..2MB", 0).max_size, Some(2 * 1024 * 1024));
    }

    #[test]
    fn finds_and_ranks() {
        let mut t = Tree::new("C:");
        let v = t.add(t.root, "Videos", flags::DIR, 0, 0, 0, 0);
        t.add(v, "holiday.mp4", 0, 10, 100, 0, 0);
        t.add(v, "my holiday clip.mp4", 0, 10, 900, 0, 0);
        t.add(t.root, "holiday.txt", 0, 10, 5000, 0, 0);
        t.aggregate();

        let (hits, total) = search(&t, &parse("holiday ext:mp4", 0), 10);
        assert_eq!(total, 2);
        assert_eq!(t.name(hits[0].id), "holiday.mp4"); // prefix beats bigger mid-name match

        let (hits, _) = search(&t, &parse("holiday in:videos", 0), 10);
        assert_eq!(hits.len(), 2);
        // Videos is 1000 bytes on disk, so only the text file clears 1 KB.
        let (hits, _) = search(&t, &parse("size:>1KB", 0), 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(t.name(hits[0].id), "holiday.txt");
    }
}
