//! Treemap and sunburst layouts. Big folders are expanded first, so a fixed tile
//! budget always goes to the parts of the drive that are actually visible.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::tree::{NodeId, Tree, flags};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn area(&self) -> f32 {
        self.w * self.h
    }
}

pub mod tile_flags {
    pub const DIR: u8 = 1;
    /// Folder drawn with a name strip along its top edge.
    pub const HEADER: u8 = 2;
    /// Folder whose contents weren't laid out (too small, or over budget).
    pub const COLLAPSED: u8 = 4;
    /// Folder where much of the space is in items too small to draw.
    pub const MANY_SMALL: u8 = 8;
    /// In the cleanup cart, or inside a folder that is. Set by the app, not the layout.
    pub const IN_CART: u8 = 16;
    /// The item that was added to the cart itself, as opposed to something inside it.
    pub const CART_ROOT: u8 = 32;
    /// Deleted while the app watched; drawn greyed out. Set by the app.
    pub const GONE: u8 = 64;
    /// Appeared while the app watched; drawn highlighted. Set by the app.
    pub const NEW: u8 = 128;
}

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub id: NodeId,
    pub rect: Rect,
    pub depth: u8,
    pub flags: u8,
    pub category: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct TreemapOptions {
    pub width: f32,
    pub height: f32,
    pub max_tiles: usize,
    /// Folders smaller than this (px²) are drawn as one block.
    pub min_folder_area: f32,
    pub header: f32,
    pub pad: f32,
    pub use_alloc: bool,
    /// Tiles smaller than this (px²) aren't emitted: they're barely visible and
    /// would otherwise let one folder of tiny files use up the whole budget.
    pub min_tile_area: f32,
}

impl Default for TreemapOptions {
    fn default() -> Self {
        TreemapOptions {
            width: 800.0,
            height: 600.0,
            max_tiles: 24_000,
            min_folder_area: 120.0,
            header: 16.0,
            pad: 2.0,
            use_alloc: true,
            min_tile_area: 6.0,
        }
    }
}

struct Pending {
    area: f32,
    id: NodeId,
    rect: Rect,
    depth: u8,
    inset: bool,
    /// Index of this folder's own tile (usize::MAX for the root).
    tile: usize,
}

impl PartialEq for Pending {
    fn eq(&self, o: &Self) -> bool {
        self.area == o.area
    }
}
impl Eq for Pending {}
impl PartialOrd for Pending {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Pending {
    fn cmp(&self, o: &Self) -> Ordering {
        self.area.total_cmp(&o.area)
    }
}

pub fn treemap(tree: &Tree, root: NodeId, o: &TreemapOptions) -> Vec<Tile> {
    let value = |id: NodeId| {
        let n = tree.get(id);
        if o.use_alloc { n.alloc } else { n.size }
    };
    let mut tiles = Vec::with_capacity(o.max_tiles.min(32_768));
    let mut heap = BinaryHeap::new();
    let full = Rect {
        x: 0.0,
        y: 0.0,
        w: o.width,
        h: o.height,
    };
    heap.push(Pending {
        area: full.area(),
        id: root,
        rect: full,
        depth: 0,
        inset: false,
        tile: usize::MAX,
    });

    let mut kids: Vec<(NodeId, f64)> = Vec::new();
    let mut placed: Vec<(NodeId, Rect)> = Vec::new();

    while let Some(p) = heap.pop() {
        if tiles.len() >= o.max_tiles {
            break;
        }
        // Inner area of a folder: a little padding and, if there's room, a name strip.
        let mut inner = p.rect;
        if p.inset {
            inner = shrink(inner, o.pad);
            if p.rect.w >= 64.0 && p.rect.h >= o.header * 2.5 {
                inner.y += o.header - o.pad;
                inner.h -= o.header - o.pad;
            }
        }
        if inner.w < 2.0 || inner.h < 2.0 {
            continue;
        }

        kids.clear();
        kids.extend(
            tree.children_all(p.id)
                .filter(|&c| !tree.get(c).has(flags::EXTRA_LINK))
                .map(|c| (c, value(c) as f64))
                .filter(|&(_, v)| v > 0.0),
        );
        kids.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        placed.clear();
        squarify(&kids, inner, &mut placed);

        let mut hidden = 0.0f32;
        for &(id, rect) in &placed {
            if rect.w < 1.0 || rect.h < 1.0 || rect.area() < o.min_tile_area {
                hidden += rect.area();
                continue;
            }
            let n = tree.get(id);
            let is_dir = n.is_dir() && tree.has_children(id);
            let mut f = 0;
            if n.is_dir() {
                f |= tile_flags::DIR;
            }
            let expand = is_dir && rect.area() >= o.min_folder_area;
            if expand && rect.w >= 64.0 && rect.h >= o.header * 2.5 {
                f |= tile_flags::HEADER;
            }
            if is_dir && !expand {
                f |= tile_flags::COLLAPSED;
            }
            let idx = tiles.len();
            tiles.push(Tile {
                id,
                rect,
                depth: p.depth.saturating_add(1),
                flags: f,
                category: if n.is_dir() {
                    0
                } else {
                    category(tree.name(id))
                },
            });
            if expand {
                heap.push(Pending {
                    area: rect.area(),
                    id,
                    rect,
                    depth: p.depth.saturating_add(1),
                    inset: true,
                    tile: idx,
                });
            }
        }
        if hidden > inner.area() * 0.2
            && let Some(t) = tiles.get_mut(p.tile)
        {
            t.flags |= tile_flags::MANY_SMALL;
        }
    }

    // Anything left in the queue never got its children drawn.
    for p in heap {
        if let Some(t) = tiles.get_mut(p.tile) {
            t.flags |= tile_flags::COLLAPSED;
            t.flags &= !tile_flags::HEADER;
        }
    }
    tiles
}

#[derive(Clone, Copy, Debug)]
pub struct Arc {
    pub id: NodeId,
    /// Start and end angle in radians, 0 at 12 o'clock, clockwise.
    pub a0: f32,
    pub a1: f32,
    /// Ring, 1 = innermost.
    pub depth: u8,
    pub flags: u8,
    pub category: u8,
}

/// Sunburst rings below `root`, `max_depth` rings deep. Arcs thinner than
/// `min_angle` are dropped, and their folders aren't opened.
pub fn sunburst(
    tree: &Tree,
    root: NodeId,
    max_depth: u8,
    min_angle: f32,
    max_arcs: usize,
    use_alloc: bool,
) -> Vec<Arc> {
    let value = |id: NodeId| {
        let n = tree.get(id);
        if use_alloc { n.alloc } else { n.size }
    };
    let mut out = Vec::new();
    let mut stack = vec![(root, 0.0f64, std::f64::consts::TAU, 0u8)];
    let mut kids: Vec<(NodeId, u64)> = Vec::new();
    while let Some((id, a0, a1, depth)) = stack.pop() {
        if depth >= max_depth || out.len() >= max_arcs {
            continue;
        }
        kids.clear();
        kids.extend(
            tree.children_all(id)
                .filter(|&c| !tree.get(c).has(flags::EXTRA_LINK))
                .map(|c| (c, value(c)))
                .filter(|&(_, v)| v > 0),
        );
        // Deleted items still on screen aren't in the folder's total; make room.
        let total = (value(id) as f64).max(kids.iter().map(|k| k.1 as f64).sum());
        if total <= 0.0 {
            continue;
        }
        kids.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        let span = a1 - a0;
        let mut a = a0;
        for &(c, v) in kids.iter() {
            let s = span * v as f64 / total;
            if (s as f32) < min_angle {
                break; // sorted, so the rest are smaller still
            }
            let n = tree.get(c);
            let opens = n.is_dir() && tree.has_children(c) && depth + 1 < max_depth;
            let mut f = 0;
            if n.is_dir() {
                f |= tile_flags::DIR;
                if !opens {
                    f |= tile_flags::COLLAPSED;
                }
            }
            out.push(Arc {
                id: c,
                a0: a as f32,
                a1: (a + s) as f32,
                depth: depth + 1,
                flags: f,
                category: if n.is_dir() {
                    0
                } else {
                    category(tree.name(c))
                },
            });
            if opens {
                stack.push((c, a, a + s, depth + 1));
            }
            a += s;
        }
    }
    // Inner rings first, so outer arcs draw over any antialiasing seams.
    out.sort_by_key(|a| a.depth);
    out
}

fn shrink(r: Rect, by: f32) -> Rect {
    Rect {
        x: r.x + by,
        y: r.y + by,
        w: (r.w - by * 2.0).max(0.0),
        h: (r.h - by * 2.0).max(0.0),
    }
}

/// Squarified treemap (Bruls, Huizing, van Wijk). `items` must be sorted by value,
/// largest first.
pub fn squarify(items: &[(NodeId, f64)], rect: Rect, out: &mut Vec<(NodeId, Rect)>) {
    let total: f64 = items.iter().map(|i| i.1).sum();
    if total <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let scale = (rect.w as f64 * rect.h as f64) / total;
    let (mut x, mut y, mut w, mut h) = (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64);
    let mut i = 0;
    while i < items.len() {
        let short = w.min(h);
        if short <= 0.0 {
            break;
        }
        let a0 = items[i].1 * scale;
        let (mut end, mut sum, mut mn, mut mx) = (i + 1, a0, a0, a0);
        let mut worst = worst_ratio(sum, mn, mx, short);
        while end < items.len() {
            let a = items[end].1 * scale;
            let s2 = sum + a;
            let w2 = worst_ratio(s2, mn.min(a), mx.max(a), short);
            if w2 > worst {
                break;
            }
            (worst, sum, mn, mx) = (w2, s2, mn.min(a), mx.max(a));
            end += 1;
        }
        // Once the remaining items are sub-pixel, stop spending time on them.
        let last_row = end == items.len() || sum < 0.25;
        if w >= h {
            let cw = sum / h;
            let mut yy = y;
            for it in &items[i..end] {
                let ih = it.1 * scale / cw;
                out.push((
                    it.0,
                    Rect {
                        x: x as f32,
                        y: yy as f32,
                        w: cw as f32,
                        h: ih as f32,
                    },
                ));
                yy += ih;
            }
            x += cw;
            w -= cw;
        } else {
            let rh = sum / w;
            let mut xx = x;
            for it in &items[i..end] {
                let iw = it.1 * scale / rh;
                out.push((
                    it.0,
                    Rect {
                        x: xx as f32,
                        y: y as f32,
                        w: iw as f32,
                        h: rh as f32,
                    },
                ));
                xx += iw;
            }
            y += rh;
            h -= rh;
        }
        if last_row && end < items.len() {
            break;
        }
        i = end;
    }
}

#[inline]
fn worst_ratio(sum: f64, min: f64, max: f64, short: f64) -> f64 {
    let s2 = short * short;
    let sum2 = sum * sum;
    (s2 * max / sum2).max(sum2 / (s2 * min))
}

/// Rough file type from the extension, used for colouring.
pub fn category(name: &str) -> u8 {
    let ext = match name.rfind('.') {
        Some(i) if i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => return 1,
    };
    match ext.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "m4v" | "flv" | "mpg" | "mpeg"
        | "m2ts" => 2,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "heic" | "tif" | "tiff" | "raw"
        | "cr2" | "nef" | "psd" | "ai" | "svg" | "ico" | "dng" | "arw" => 3,
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" => 4,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "cab" | "msi" | "msix"
        | "appx" | "nupkg" | "whl" | "jar" | "apk" => 5,
        "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "c" | "cpp" | "h" | "hpp" | "cs" | "java"
        | "kt" | "go" | "rb" | "php" | "swift" | "json" | "xml" | "yaml" | "yml" | "toml"
        | "html" | "css" | "scss" | "svelte" | "vue" | "map" | "pdb" | "lock" | "md" => 6,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "rtf" | "odt"
        | "csv" | "epub" | "pst" | "ost" => 7,
        "exe" | "dll" | "sys" | "so" | "lib" | "a" | "o" | "obj" | "node" | "bin" | "drv"
        | "ocx" => 8,
        "vhd" | "vhdx" | "vmdk" | "vdi" | "iso" | "img" | "qcow2" | "wim" | "esd" => 9,
        "db" | "sqlite" | "sqlite3" | "mdf" | "ldf" | "edb" | "dat" | "log" | "etl" | "evtx"
        | "cache" | "tmp" | "dmp" => 10,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_fills_rect_exactly() {
        let items: Vec<(NodeId, f64)> = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0]
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as NodeId, v))
            .collect();
        let mut out = Vec::new();
        squarify(
            &items,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 6.0,
                h: 4.0,
            },
            &mut out,
        );
        assert_eq!(out.len(), 7);
        let area: f32 = out.iter().map(|(_, r)| r.area()).sum();
        assert!((area - 24.0).abs() < 1e-3);
        for (_, r) in &out {
            assert!(r.x >= -1e-4 && r.y >= -1e-4 && r.x + r.w <= 6.0001 && r.y + r.h <= 4.0001);
        }
    }

    #[test]
    fn sunburst_angles_are_proportional() {
        let mut t = Tree::new("C:");
        let a = t.add(t.root, "a", flags::DIR, 0, 0, 0, 0);
        t.add(a, "x.bin", 0, 0, 300, 0, 0);
        t.add(t.root, "b.bin", 0, 0, 100, 0, 0);
        t.aggregate();
        let arcs = sunburst(&t, t.root, 4, 0.0, 100, true);
        assert_eq!(arcs.len(), 3);
        let top: Vec<_> = arcs.iter().filter(|a| a.depth == 1).collect();
        assert!((top[0].a1 - top[0].a0 - std::f32::consts::TAU * 0.75).abs() < 1e-4);
        let inner = arcs.iter().find(|a| a.depth == 2).unwrap();
        assert!((inner.a1 - top[0].a1).abs() < 1e-4);
    }

    #[test]
    fn treemap_respects_budget_and_bounds() {
        let mut t = Tree::new("C:");
        for d in 0..20 {
            let dir = t.add(t.root, &format!("d{d}"), flags::DIR, 0, 0, 0, 0);
            for f in 0..200 {
                t.add(dir, &format!("f{f}.mp4"), 0, 1000 + f, 4096 * (f + 1), 0, 0);
            }
        }
        t.aggregate();
        let o = TreemapOptions {
            width: 400.0,
            height: 300.0,
            max_tiles: 500,
            ..Default::default()
        };
        let tiles = treemap(&t, t.root, &o);
        assert!(tiles.len() <= 500 + 200);
        assert!(
            tiles
                .iter()
                .all(|t| t.rect.x >= 0.0 && t.rect.x + t.rect.w <= 400.01)
        );
        assert!(tiles.iter().any(|t| t.category == 2));
    }
}
