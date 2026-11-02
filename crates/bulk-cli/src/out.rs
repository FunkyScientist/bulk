//! Printing results: sizes, times, tables and JSON.

use std::io::{IsTerminal, Write};

use ratatui::style::Color;
use serde::Serialize;

use crate::theme;

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

/// Whether to colour stdout: a terminal that understands colour codes (this also
/// turns them on in older consoles), and NO_COLOR isn't set.
pub fn colour() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && ratatui::crossterm::ansi_support::supports_ansi()
    })
}

fn style(code: &str, s: &str) -> String {
    if colour() && !s.is_empty() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn dim(s: &str) -> String {
    style("2", s)
}

pub fn bold(s: &str) -> String {
    style("1", s)
}

/// `s` in a template colour.
pub fn paint(s: &str, c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => style(&format!("38;2;{r};{g};{b}"), s),
        _ => s.to_string(),
    }
}

/// A size, louder the bigger it is: bold from a gigabyte, faint under a megabyte.
pub fn size_cell(b: u64) -> String {
    let s = size(b);
    if b >= 1 << 30 {
        bold(&paint(&s, theme::get().warn))
    } else if b < 1 << 20 {
        dim(&s)
    } else {
        s
    }
}

/// A change in size, in the template's grew and shrank colours.
pub fn delta_cell(d: i64) -> String {
    let t = theme::get();
    paint(&delta(d), if d >= 0 { t.grew } else { t.shrank })
}

/// A path with its folders faint and its name bright; folders in the folder colour.
pub fn path_cell(path: &str, dir: bool) -> String {
    let (parent, name) = match path.rsplit_once('\\') {
        Some((p, n)) if !n.is_empty() => (format!("{p}\\"), n),
        _ => (String::new(), path),
    };
    let name = if dir {
        paint(&format!("{name}\\"), theme::get().file("folder"))
    } else {
        name.to_string()
    };
    format!("{}{name}", dim(&parent))
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

/// A share bar in a colour, on a faint track.
pub fn bar_in(frac: f64, width: usize, c: Color) -> String {
    let b = bar(frac, width);
    let filled = b.trim_end().to_string();
    let rest = width.saturating_sub(filled.chars().count());
    format!("{}{}", paint(&filled, c), dim(&"·".repeat(rest)))
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

/// `s` padded on the left to `w` columns, not counting colour codes.
pub fn rjust(s: &str, w: usize) -> String {
    let n = visible(s).chars().count();
    format!("{}{s}", " ".repeat(w.saturating_sub(n)))
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
