//! Space that can usually be got back: build output, caches, old temp files and
//! downloads, crash dumps. Each finding says how safe removing it is and why.

use std::path::Path;

use serde::Serialize;

use crate::tree::{NodeId, Tree, flags};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Safety {
    /// Rebuilt or downloaded again when needed.
    Safe,
    /// Usually fine, but check: it may be in use or slow to get back.
    Review,
    /// Worth knowing about; remove it with the tool named, not by deleting files.
    Info,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: NodeId,
    pub path: String,
    pub alloc: u64,
    /// Newest modification inside, unix seconds.
    pub mtime: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub key: &'static str,
    pub label: &'static str,
    pub safety: Safety,
    pub why: &'static str,
    pub alloc: u64,
    /// Biggest first.
    pub items: Vec<Item>,
}

struct Rule {
    key: &'static str,
    label: &'static str,
    safety: Safety,
    why: &'static str,
}

const NODE_MODULES: usize = 0;
const RUST_TARGET: usize = 1;
const PY_CACHE: usize = 2;
const PY_VENV: usize = 3;
const PKG_CACHE: usize = 4;
const TEMP: usize = 5;
const BROWSER: usize = 6;
const DUMPS: usize = 7;
const DOWNLOADS: usize = 8;
const RECYCLE: usize = 9;
const UPDATES: usize = 10;
const VDISKS: usize = 11;
const SYSFILES: usize = 12;

const RULES: [Rule; 13] = [
    Rule {
        key: "node_modules",
        label: "JavaScript packages (node_modules)",
        safety: Safety::Safe,
        why: "npm, pnpm or yarn install puts them back",
    },
    Rule {
        key: "rust-target",
        label: "Rust build output (target)",
        safety: Safety::Safe,
        why: "cargo build makes it again",
    },
    Rule {
        key: "python-cache",
        label: "Python caches",
        safety: Safety::Safe,
        why: "Python and its tools recreate them",
    },
    Rule {
        key: "python-venv",
        label: "Python virtual environments",
        safety: Safety::Review,
        why: "can be recreated from the project's requirements, if it has them",
    },
    Rule {
        key: "package-cache",
        label: "Package manager caches",
        safety: Safety::Review,
        why: "downloaded again when needed; slower installs until then",
    },
    Rule {
        key: "temp",
        label: "Temporary files older than a week",
        safety: Safety::Safe,
        why: "programs keep temporary files there and should have cleaned them up",
    },
    Rule {
        key: "browser-cache",
        label: "Browser caches",
        safety: Safety::Safe,
        why: "the browser downloads them again; close it first",
    },
    Rule {
        key: "crash-dumps",
        label: "Crash dumps",
        safety: Safety::Safe,
        why: "only useful for reporting a crash",
    },
    Rule {
        key: "old-downloads",
        label: "Installers and archives in Downloads, older than 3 months",
        safety: Safety::Review,
        why: "usually installed or unpacked long ago",
    },
    Rule {
        key: "recycle-bin",
        label: "Recycle Bin",
        safety: Safety::Info,
        why: "empty it from the Recycle Bin on the desktop",
    },
    Rule {
        key: "windows-update",
        label: "Windows Update downloads",
        safety: Safety::Info,
        why: "use Disk Cleanup > Clean up system files > Windows Update Cleanup",
    },
    Rule {
        key: "virtual-disks",
        label: "Virtual disks (WSL, Docker, VMs)",
        safety: Safety::Info,
        why: "shrink or remove them from the tool that made them; deleting breaks it",
    },
    Rule {
        key: "system-files",
        label: "Hibernation and page files",
        safety: Safety::Info,
        why: "Windows manages these; `powercfg /h off` removes the hibernation file",
    },
];

const DAY: u32 = 86_400;

/// Everything found under `root` (a whole drive, or a folder), biggest first.
pub fn scan(tree: &Tree, root: NodeId, now: u64) -> Vec<Finding> {
    let now = now as u32;
    let mut found: Vec<Vec<NodeId>> = vec![Vec::new(); RULES.len()];

    // Known places, by path.
    let env = |k: &str| std::env::var(k).ok();
    let profile = env("USERPROFILE");
    let local = env("LOCALAPPDATA");
    let mut places: Vec<(usize, String)> = Vec::new();
    if let Some(l) = &local {
        for p in [
            "npm-cache",
            "pip\\cache",
            "Yarn\\Cache",
            "pnpm\\store",
            "NuGet\\v3-cache",
            "CrashDumps",
        ] {
            let rule = if p == "CrashDumps" { DUMPS } else { PKG_CACHE };
            places.push((rule, format!("{l}\\{p}")));
        }
    }
    if let Some(p) = &profile {
        for sub in [
            ".cargo\\registry\\cache",
            ".gradle\\caches",
            ".nuget\\packages",
        ] {
            places.push((PKG_CACHE, format!("{p}\\{sub}")));
        }
    }
    if let Some(w) = env("SystemRoot") {
        places.push((UPDATES, format!("{w}\\SoftwareDistribution\\Download")));
    }
    let drive = tree.name(tree.root).to_string();
    places.push((RECYCLE, format!("{drive}\\$Recycle.Bin")));
    for (rule, p) in places {
        if let Some(id) = tree.find_path(&p)
            && tree.is_ancestor(root, id)
            && tree.get(id).alloc > 0
        {
            found[rule].push(id);
        }
    }
    // What's been in Temp for over a week.
    if let Some(id) = env("TEMP").and_then(|t| tree.find_path(&long(&t))) {
        for c in tree.children(id) {
            let n = tree.get(c);
            if tree.is_ancestor(root, c) && n.mtime + 7 * DAY < now && !gone(tree, c) {
                found[TEMP].push(c);
            }
        }
    }
    let downloads = profile
        .as_ref()
        .and_then(|p| tree.find_path(&format!("{p}\\Downloads")));

    // Everything else, by name, in one pass. Developer rules stay out of system
    // and app folders, where bundled node_modules and the like belong to programs.
    let mut stack: Vec<(NodeId, bool)> = vec![(root, in_app_area(tree, root))];
    while let Some((id, apps)) = stack.pop() {
        for c in tree.children(id) {
            if gone(tree, c) {
                continue;
            }
            let n = tree.get(c);
            let name = tree.name_lower(c);
            if n.is_dir() {
                let rule = match name {
                    b"node_modules" if !apps => Some(NODE_MODULES),
                    b"target" if !apps && has_child(tree, id, b"cargo.toml") => Some(RUST_TARGET),
                    b"__pycache__" | b".pytest_cache" | b".mypy_cache" | b".ruff_cache"
                        if !apps =>
                    {
                        Some(PY_CACHE)
                    }
                    b".venv" | b"venv" if !apps && has_child(tree, c, b"pyvenv.cfg") => {
                        Some(PY_VENV)
                    }
                    b"cache" | b"code cache" | b"gpucache" | b"cachestorage"
                        if under(tree, c, b"user data") =>
                    {
                        Some(BROWSER)
                    }
                    _ => None,
                };
                match rule {
                    Some(r) if !found.iter().flatten().any(|&f| f == c) => found[r].push(c),
                    Some(_) => {}
                    None => {
                        let apps = apps || app_area_name(tree, id, c);
                        stack.push((c, apps));
                    }
                }
            } else {
                let ext = crate::kinds::ext_of(name).unwrap_or_default();
                let rule = if ext == b"dmp" {
                    Some(DUMPS)
                } else if (ext == b"vhdx" || ext == b"vhd") && n.alloc >= 1 << 30 {
                    Some(VDISKS)
                } else if id == tree.root
                    && matches!(name, b"hiberfil.sys" | b"pagefile.sys" | b"swapfile.sys")
                {
                    Some(SYSFILES)
                } else if Some(id) == downloads
                    && n.mtime + 90 * DAY < now
                    && matches!(
                        ext,
                        b"exe" | b"msi" | b"msix" | b"iso" | b"zip" | b"7z" | b"rar"
                    )
                {
                    Some(DOWNLOADS)
                } else {
                    None
                };
                if let Some(r) = rule {
                    found[r].push(c);
                }
            }
        }
    }

    let mut out: Vec<Finding> = found
        .into_iter()
        .enumerate()
        .filter(|(_, ids)| !ids.is_empty())
        .map(|(i, ids)| {
            let r = &RULES[i];
            let mut items: Vec<Item> = ids
                .into_iter()
                .map(|id| {
                    let n = tree.get(id);
                    Item {
                        id,
                        path: tree.path(id),
                        alloc: n.alloc,
                        mtime: n.mtime,
                    }
                })
                .filter(|it| it.alloc > 0)
                .collect();
            items.sort_by(|a, b| b.alloc.cmp(&a.alloc));
            // Build output that's still being worked on is worth a second look.
            let recent = items.iter().any(|it| it.mtime + 30 * DAY > now);
            let safety = if matches!(i, NODE_MODULES | RUST_TARGET) && recent {
                Safety::Review
            } else {
                r.safety
            };
            Finding {
                key: r.key,
                label: r.label,
                safety,
                why: r.why,
                alloc: items.iter().map(|it| it.alloc).sum(),
                items,
            }
        })
        .filter(|f| !f.items.is_empty())
        .collect();
    out.sort_by(|a, b| b.alloc.cmp(&a.alloc));
    out
}

fn gone(tree: &Tree, id: NodeId) -> bool {
    tree.get(id).has(flags::GONE | flags::DELETED)
}

fn has_child(tree: &Tree, dir: NodeId, name: &[u8]) -> bool {
    tree.children(dir).any(|c| tree.name_lower(c) == name)
}

fn under(tree: &Tree, id: NodeId, name: &[u8]) -> bool {
    tree.ancestors(id).any(|a| tree.name_lower(a) == name)
}

/// Folders whose contents belong to installed programs or Windows.
fn app_area_name(tree: &Tree, parent: NodeId, id: NodeId) -> bool {
    let name = tree.name_lower(id);
    (parent == tree.root
        && matches!(
            name,
            b"windows" | b"program files" | b"program files (x86)" | b"programdata"
        ))
        || name == b"appdata"
}

fn in_app_area(tree: &Tree, id: NodeId) -> bool {
    std::iter::once(id)
        .chain(tree.ancestors(id))
        .any(|a| a != tree.root && app_area_name(tree, tree.get(a).parent, a))
}

/// `%TEMP%` can be in 8.3 form (`HANANS~1`); trees use long names.
fn long(path: &str) -> String {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let w = crate::wide::wide(Path::new(path));
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetLongPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n > buf.len() {
        return path.to_string();
    }
    String::from_utf16_lossy(&buf[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_build_output_and_leaves_programs_alone() {
        let now = 1_000 * DAY as u64;
        let old = (now as u32) - 100 * DAY;
        let mut t = Tree::new("Q:");
        let proj = t.add(t.root, "proj", flags::DIR, 0, 0, 0, 0);
        t.add(proj, "Cargo.toml", 0, 1, 4096, old, 0);
        let target = t.add(proj, "target", flags::DIR, 0, 0, 0, 0);
        t.add(target, "app.exe", 0, 10, 8192, old, 0);
        let nm = t.add(proj, "node_modules", flags::DIR, 0, 0, 0, 0);
        let inner = t.add(nm, "node_modules", flags::DIR, 0, 0, 0, 0);
        t.add(inner, "x.js", 0, 10, 4096, old, 0);
        // A program's bundled packages, and a "target" that isn't Rust's.
        let pf = t.add(t.root, "Program Files", flags::DIR, 0, 0, 0, 0);
        let app = t.add(pf, "App", flags::DIR, 0, 0, 0, 0);
        let bundled = t.add(app, "node_modules", flags::DIR, 0, 0, 0, 0);
        t.add(bundled, "y.js", 0, 10, 4096, old, 0);
        let other = t.add(t.root, "docs", flags::DIR, 0, 0, 0, 0);
        let not_rust = t.add(other, "target", flags::DIR, 0, 0, 0, 0);
        t.add(not_rust, "z.txt", 0, 10, 4096, old, 0);
        t.add(t.root, "crash.dmp", 0, 10, 4096, old, 0);
        t.aggregate();

        let f = scan(&t, t.root, now);
        let get = |k: &str| f.iter().find(|x| x.key == k);
        let rust = get("rust-target").expect("finds target next to Cargo.toml");
        assert_eq!(rust.items.len(), 1);
        assert_eq!(rust.safety, Safety::Safe);
        let nm = get("node_modules").unwrap();
        assert_eq!(
            nm.items.len(),
            1,
            "only the outer node_modules, not the program's"
        );
        assert_eq!(nm.items[0].path, "Q:\\proj\\node_modules");
        assert!(get("crash-dumps").is_some());

        // Scoped to a folder, only what's inside it.
        let f = scan(&t, other, now);
        assert!(f.is_empty(), "{f:?}");
    }
}
