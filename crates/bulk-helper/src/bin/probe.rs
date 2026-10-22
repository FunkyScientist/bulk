//! Dev tool: install the helper task if needed, then time a scan through it.
//! Must sit next to `bulk-helper.exe` (the helper only serves clients there).
//!
//!     cargo run --release -p bulk-helper --bin probe -- C

#![windows_subsystem = "console"]

use std::time::{Duration, Instant};

use bulk_core::helper::{self, Client};

fn main() -> anyhow::Result<()> {
    let letter = std::env::args().nth(1).and_then(|s| s.chars().next()).unwrap_or('C');
    let client = Client::new()?;

    let t0 = Instant::now();
    if !client.ensure_running(Duration::from_secs(5)) {
        println!("helper not running; installing task (UAC prompt)...");
        anyhow::ensure!(helper::install_elevated(&helper::helper_exe())?, "install declined or failed");
        anyhow::ensure!(client.ensure_running(Duration::from_secs(5)), "helper did not start");
    }
    println!("helper up in {:?}: {:?}", t0.elapsed(), client.hello()?);

    let t1 = Instant::now();
    let mut last = 0;
    let tree = client.scan(letter, |p| {
        if p.files / 250_000 != last {
            last = p.files / 250_000;
            println!("  {:>5.2}s  {} files", t1.elapsed().as_secs_f64(), p.files);
        }
        true
    })?;
    let root = tree.get(tree.root);
    println!(
        "scan {}: {:.2}s  nodes {}  files {}  on disk {:.1} GB",
        letter,
        t1.elapsed().as_secs_f64(),
        tree.len(),
        root.files,
        root.alloc as f64 / (1u64 << 30) as f64
    );
    Ok(())
}
