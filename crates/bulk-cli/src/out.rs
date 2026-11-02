//! Printing results: sizes, times, tables and JSON.

use std::io::{IsTerminal, Write};

use serde::Serialize;

pub fn size(b: u64) -> String {
    bulk_core::export::human(b)
}

/// A size change, with its sign.
pub fn delta(d: i64) -> String {
    if d >= 0 {
        format!("+{}", size(d as u64))
    } else {
        format!("-{}", size(d.unsigned_abs()))
    }
}

pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "just now", "5 min ago", "3 days ago"...
pub fn ago(unix: u64) -> String {
    let s = bulk_core::os::now().saturating_sub(unix);
    let (n, unit) = match s {
        0..60 => return "just now".into(),
        60..3600 => (s / 60, "min"),
        3600..86_400 => (s / 3600, "hour"),
        86_400..2_592_000 => (s / 86_400, "day"),
        2_592_000..31_536_000 => (s / 2_592_000, "month"),
        _ => (s / 31_536_000, "year"),
    };
    let plural = if n == 1 || unit == "min" { "" } else { "s" };
    format!("{n} {unit}{plural} ago")
}

/// `2024-03-09` from unix seconds; empty for 0.
pub fn date(unix: u32) -> String {
    if unix == 0 {
        return String::new();
    }
    // Civil from days (Howard Hinnant).
    let z = unix as i64 / 86_400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Whether to colour stdout: a terminal, and NO_COLOR isn't set.
pub fn colour() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn dim(s: &str) -> String {
    if colour() {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    if colour() {
        format!("\x1b[1m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// A share bar `width` cells wide.
pub fn bar(frac: f64, width: usize) -> String {
    let cells = (frac.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let mut s = "█".repeat(cells / 8);
    if !cells.is_multiple_of(8) {
        s.push(['▏', '▎', '▍', '▌', '▋', '▊', '▉'][cells % 8 - 1]);
    }
    let used = s.chars().count();
    s.push_str(&" ".repeat(width.saturating_sub(used)));
    s
}

pub fn json(v: &impl Serialize) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, v)?;
    writeln!(out)?;
    Ok(())
}

/// Columns padded to line up. Right-aligned columns are for numbers; the last
/// column is never padded.
pub struct Table {
    right: Vec<bool>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(right: &[bool]) -> Table {
        Table {
            right: right.to_vec(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn print(&self) {
        let width = |s: &str| visible(s).chars().count();
        let n = self.right.len();
        let mut w = vec![0; n];
        for r in &self.rows {
            for (i, c) in r.iter().enumerate().take(n) {
                w[i] = w[i].max(width(c));
            }
        }
        let mut out = std::io::stdout().lock();
        for r in &self.rows {
            let mut line = String::new();
            for (i, c) in r.iter().enumerate() {
                if i > 0 {
                    line.push_str("  ");
                }
                let pad = " ".repeat(w[i].saturating_sub(width(c)));
                if self.right[i] {
                    line.push_str(&pad);
                    line.push_str(c);
                } else {
                    line.push_str(c);
                    if i + 1 < r.len() {
                        line.push_str(&pad);
                    }
                }
            }
            let _ = writeln!(out, "{}", line.trim_end());
        }
    }
}

/// `s` without ANSI colour codes, for measuring.
fn visible(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in s.chars() {
        match (esc, c) {
            (false, '\x1b') => esc = true,
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, c) => out.push(c),
        }
    }
    out
}

/// "… and 412 more" after a cut-off list.
pub fn more(shown: usize, total: usize) {
    if total > shown {
        println!(
            "{}",
            dim(&format!("… and {} more", count((total - shown) as u64)))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_counts() {
        assert_eq!(date(0), "");
        assert_eq!(date(1_709_942_400), "2024-03-09");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(count(999), "999");
        assert_eq!(delta(-2048), "-2.0 KB");
        assert_eq!(bar(0.5, 4).chars().count(), 4);
    }
}
