//! Compare MFT results against the directory walker for a folder. Run elevated.
//!
//!     cargo run --release -p bulk-core --example compare -- C:\Windows

use std::collections::HashMap;

use bulk_core::scan::{self, Progress};

fn main() -> anyhow::Result<()> {
    let folder = std::env::args().nth(1).unwrap_or_else(|| r"C:\Windows".into());
    let letter = folder.chars().next().unwrap();
    let vol = bulk_core::volume::query(letter).unwrap();

    let mft = scan::mft::scan(letter, &Progress::default())?;
    let walk = scan::walk::walk(&folder, vol.cluster as u64, &Progress::default())?;
    let base = mft.find_path(&folder).expect("folder not in mft tree");

    let mut walked: HashMap<String, (u64, u64)> = HashMap::new();
    for id in 1..walk.len() as u32 {
        let n = walk.get(id);
        if !n.is_dir() {
            walked.insert(walk.path(id).to_ascii_lowercase(), (n.size, n.alloc));
        }
    }

    let mut diffs: Vec<(i64, String, u64, u64, u64, u64, u16)> = Vec::new();
    let mut stack = vec![base];
    while let Some(id) = stack.pop() {
        stack.extend(mft.children(id));
        let n = mft.get(id);
        if n.is_dir() {
            continue;
        }
        let p = mft.path(id);
        if let Some(&(ws, wa)) = walked.get(&p.to_ascii_lowercase()) {
            if wa != n.alloc || ws != n.size {
                diffs.push((wa as i64 - n.alloc as i64, p, n.size, n.alloc, ws, wa, n.flags));
            }
        }
    }
    diffs.sort_by_key(|d| -d.0.abs());
    let total: i64 = diffs.iter().map(|d| d.0).sum();
    println!(
        "mft {} alloc {}  walk alloc {}  differing files {}  net walk-mft {}",
        folder,
        mft.get(base).alloc,
        walk.get(walk.root).alloc,
        diffs.len(),
        total
    );
    for d in diffs.iter().take(25) {
        println!(
            "{:>12}  mft size {} alloc {}  walk size {} alloc {}  flags {:#x}  {}",
            d.0, d.2, d.3, d.4, d.5, d.6, d.1
        );
    }
    Ok(())
}
