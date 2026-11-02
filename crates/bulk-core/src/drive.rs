//! A drive's tree without the app: the last snapshot, caught up from the change
//! journal through the helper, or a fresh scan when that isn't possible.

use std::time::Duration;

use serde::Serialize;

use crate::helper::{CatchUp, Client, ProgressInfo};
use crate::scan::{Progress, walk};
use crate::snapshot::{self, Meta};
use crate::tree::Tree;
use crate::usn::{self, FrnIndex};
use crate::volume::Volume;

/// How current a loaded tree is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Fresh {
    /// Caught up from the change journal just now.
    Live,
    /// Scanned just now.
    Scanned,
    /// The saved snapshot as it was; nothing could bring it up to date.
    Snapshot,
}

pub struct Loaded {
    pub vol: Volume,
    pub meta: Meta,
    pub tree: Tree,
    pub fresh: Fresh,
    /// Differs from what's saved in `current.bulk`.
    pub changed: bool,
}

/// What `load` is doing, for progress output.
pub enum Step {
    Loading,
    CatchingUp,
    Scanning(ProgressInfo),
}

pub struct Options {
    /// Scan when the snapshot is missing or can't be caught up.
    pub scan: bool,
    /// Scan even if the snapshot could be used.
    pub rescan: bool,
}

/// The helper, if fast mode is on and it's installed. Starts it when needed; that
/// needs no prompt once its task exists.
pub fn helper() -> Option<Client> {
    if !crate::settings::load().fast_mode {
        return None;
    }
    let c = Client::new().ok()?;
    c.ensure_running(Duration::from_secs(5)).then_some(c)
}

pub fn load(
    vol: &Volume,
    helper: Option<&Client>,
    opts: &Options,
    mut step: impl FnMut(Step),
) -> anyhow::Result<Loaded> {
    let path = snapshot::current_path(vol.serial);
    let mut saved = None;
    if !opts.rescan && path.exists() {
        step(Step::Loading);
        saved = snapshot::load(&path).ok();
    }
    let fast = vol.is_ntfs() && helper.is_some();

    if let Some((mut meta, mut tree)) = saved {
        let journal = meta.journal.filter(|_| fast);
        let Some(j) = journal else {
            // Nothing can catch it up (folder-walked drive, or no helper).
            return Ok(Loaded {
                vol: vol.clone(),
                meta,
                tree,
                fresh: Fresh::Snapshot,
                changed: false,
            });
        };
        step(Step::CatchingUp);
        match helper.unwrap().changes(vol.letter, j.id, j.next)? {
            CatchUp::Changes(changes, next) => {
                let mut index = FrnIndex::build(&tree);
                let n = usn::apply(&mut tree, &mut index, &changes);
                meta.journal = Some(usn::Journal { next, ..j });
                meta.taken = crate::os::now();
                restamp(&mut meta, &tree);
                return Ok(Loaded {
                    vol: vol.clone(),
                    meta,
                    tree,
                    fresh: Fresh::Live,
                    changed: n > 0 || next != j.next,
                });
            }
            CatchUp::Gap if !opts.scan => {
                return Ok(Loaded {
                    vol: vol.clone(),
                    meta,
                    tree,
                    fresh: Fresh::Snapshot,
                    changed: false,
                });
            }
            CatchUp::Gap => {}
        }
    } else if !opts.scan {
        anyhow::bail!(
            "{}: hasn't been scanned yet (run without --no-scan to scan it)",
            vol.letter
        );
    }

    let (tree, journal, method) = if fast {
        let (t, j) = helper.unwrap().scan(vol.letter, |p| {
            step(Step::Scanning(p));
            true
        })?;
        (t, j, "mft")
    } else {
        let p = Progress::default();
        let t = std::thread::scope(|s| {
            let job = s.spawn(|| walk::walk(&vol.root, vol.cluster as u64, &p));
            while !job.is_finished() {
                step(Step::Scanning(ProgressInfo::of(&p)));
                std::thread::sleep(Duration::from_millis(100));
            }
            job.join().unwrap()
        })?;
        (t, None, "walk")
    };
    let mut meta = Meta {
        letter: vol.letter,
        serial: vol.serial,
        label: vol.label.clone(),
        taken: crate::os::now(),
        method: method.into(),
        files: 0,
        alloc: 0,
        size: 0,
        journal,
    };
    restamp(&mut meta, &tree);
    Ok(Loaded {
        vol: vol.clone(),
        meta,
        tree,
        fresh: Fresh::Scanned,
        changed: true,
    })
}

/// Totals in `meta` from the tree's root.
fn restamp(meta: &mut Meta, tree: &Tree) {
    let root = tree.get(tree.root);
    meta.files = root.files;
    meta.alloc = root.alloc;
    meta.size = root.size;
}

/// Write the tree back to `current.bulk` (and history, after a full scan) so the
/// next run starts from it. Skipped while the app is open, since it saves its own
/// copy of the same drive.
pub fn save(l: &Loaded) -> anyhow::Result<bool> {
    if !l.changed || crate::os::app_running() {
        return Ok(false);
    }
    let tree = l.tree.compact();
    snapshot::save_current(&l.meta, &tree)?;
    if l.fresh == Fresh::Scanned {
        let keep = crate::settings::load().history_keep.clamp(1, 365);
        snapshot::save_history(&l.meta, &tree, keep)?;
    }
    Ok(true)
}
