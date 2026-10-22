//! Time a scan and print a summary.
//!
//!     cargo run --release -p bulk-core --example bench -- C: [walk|mft]

use std::time::Instant;

use bulk_core::scan::{self, Progress};
use bulk_core::tree::SortKey;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let target = args.next().unwrap_or_else(|| "C:".into());
    let method = args.next().unwrap_or_else(|| "walk".into());

    let letter = target.chars().next().unwrap();
    let vol = bulk_core::volume::query(letter).ok_or_else(|| anyhow::anyhow!("no such drive"))?;
    println!("{} {} ({}) {:?}", vol.root, vol.label, vol.fs, vol.kind);

    let progress = Progress::default();
    let t0 = Instant::now();
    let tree = match method.as_str() {
        "walk" => scan::walk::walk(&target, vol.cluster as u64, &progress)?,
        "mft" => scan::mft::scan(letter, &progress)?,
        m => anyhow::bail!("unknown method {m}"),
    };
    let elapsed = t0.elapsed();

    let root = tree.get(tree.root);
    println!(
        "{method}: {:.2}s  nodes {}  files {}  dirs {}  size {}  on disk {}",
        elapsed.as_secs_f64(),
        tree.len(),
        root.files,
        tree.dir_count(),
        human(root.size),
        human(root.alloc),
    );
    for id in tree.sorted_children(tree.root, SortKey::Alloc).into_iter().take(12) {
        let n = tree.get(id);
        println!("  {:>10}  {}", human(n.alloc), tree.name(id));
    }
    Ok(())
}

fn human(b: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}
