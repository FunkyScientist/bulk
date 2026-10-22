//! Time the duplicate finder on a folder.
//!
//!     cargo run --release -p bulk-core --example dupebench -- "C:\Users\me"

use std::time::Instant;

use bulk_core::dupes::{self, DupeProgress};
use bulk_core::scan::{self, Progress};

fn main() -> anyhow::Result<()> {
    let folder = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"C:\Users".into());
    let t0 = Instant::now();
    let tree = scan::walk::walk(&folder, 4096, &Progress::default())?;
    let cands = dupes::candidates(&tree, tree.root, 1 << 20, true);
    println!(
        "walk + list: {:.2}s, {} candidates",
        t0.elapsed().as_secs_f64(),
        cands.len()
    );

    let p = DupeProgress::default();
    let t1 = Instant::now();
    let groups = dupes::find(cands, &p);
    println!(
        "find: {:.2}s, {} groups, {:.1} MB hashed in full",
        t1.elapsed().as_secs_f64(),
        groups.len(),
        p.bytes_done.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6
    );
    Ok(())
}
