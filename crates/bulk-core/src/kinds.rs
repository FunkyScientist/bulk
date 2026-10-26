//! File kinds (videos, spreadsheets, …) grouped by extension, and the storage each
//! kind and extension takes under a folder.

use std::collections::HashMap;
use std::sync::LazyLock;

use rayon::prelude::*;
use serde::Serialize;

use crate::tree::{NodeId, Tree, flags};

pub struct Kind {
    pub key: &'static str,
    pub label: &'static str,
    pub exts: &'static [&'static str],
}

pub const KINDS: &[Kind] = &[
    Kind {
        key: "video",
        label: "Videos",
        exts: &[
            "mp4", "mkv", "avi", "mov", "wmv", "webm", "m4v", "flv", "mpg", "mpeg", "m2ts", "mts",
            "3gp", "vob", "ogv",
        ],
    },
    Kind {
        key: "image",
        label: "Images",
        exts: &[
            "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "avif", "tif", "tiff",
            "raw", "cr2", "cr3", "nef", "dng", "arw", "orf", "rw2", "svg", "ico", "jfif",
        ],
    },
    Kind {
        key: "audio",
        label: "Audio",
        exts: &[
            "mp3", "flac", "wav", "aac", "ogg", "m4a", "wma", "opus", "aiff", "aif", "alac", "mid",
            "midi", "amr",
        ],
    },
    Kind {
        key: "document",
        label: "Documents",
        exts: &[
            "pdf", "doc", "docx", "docm", "dot", "dotx", "txt", "rtf", "odt", "md", "epub", "mobi",
            "pages", "tex", "xps", "oxps", "one",
        ],
    },
    Kind {
        key: "spreadsheet",
        label: "Spreadsheets",
        exts: &[
            "xls", "xlsx", "xlsm", "xlsb", "xlt", "xltx", "csv", "tsv", "ods", "numbers",
        ],
    },
    Kind {
        key: "presentation",
        label: "Presentations",
        exts: &["ppt", "pptx", "pptm", "pps", "ppsx", "pot", "potx", "odp", "key"],
    },
    Kind {
        key: "archive",
        label: "Archives",
        exts: &[
            "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "lz", "lzma", "cab", "z",
        ],
    },
    Kind {
        key: "program",
        label: "Programs",
        exts: &[
            "exe", "dll", "msi", "msix", "msixbundle", "appx", "appxbundle", "msp", "so", "lib",
            "ocx", "node", "apk", "jar", "pyd", "asar",
        ],
    },
    Kind {
        key: "system",
        label: "System files",
        exts: &[
            "sys", "drv", "efi", "mui", "cat", "mum", "manifest", "winmd", "nls",
        ],
    },
    Kind {
        key: "code",
        label: "Code",
        exts: &[
            "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "pyc", "c", "cc", "cpp", "h",
            "hpp", "cs", "java", "class", "kt", "go", "rb", "php", "swift", "json", "xml", "yaml",
            "yml", "toml", "ini", "html", "htm", "css", "scss", "svelte", "vue", "map", "pdb",
            "lock", "sh", "ps1", "psm1", "bat", "cmd", "sql", "ipynb", "lua", "dart", "o", "obj",
            "a", "rlib", "rmeta", "d", "pak", "wasm",
        ],
    },
    Kind {
        key: "diskimage",
        label: "Disk images",
        exts: &[
            "iso", "img", "vhd", "vhdx", "vmdk", "vdi", "qcow2", "wim", "esd", "dmg", "avhdx",
        ],
    },
    Kind {
        key: "database",
        label: "Databases",
        exts: &[
            "db", "sqlite", "sqlite3", "mdf", "ldf", "ndf", "edb", "accdb", "mdb", "dbf", "realm",
            "ldb", "sdf", "qbw", "tlg",
        ],
    },
    Kind {
        key: "design",
        label: "Design & 3D",
        exts: &[
            "psd", "psb", "ai", "eps", "indd", "sketch", "fig", "xd", "blend", "fbx", "stl", "3ds",
            "max", "c4d", "glb", "gltf", "dwg", "dxf", "afphoto", "afdesign", "kra", "xcf",
        ],
    },
    Kind {
        key: "font",
        label: "Fonts",
        exts: &["ttf", "otf", "woff", "woff2", "fon", "ttc", "eot"],
    },
    Kind {
        key: "email",
        label: "Email",
        exts: &["pst", "ost", "eml", "msg", "mbox"],
    },
    Kind {
        key: "log",
        label: "Logs & temp",
        exts: &[
            "log", "etl", "evtx", "tmp", "temp", "dmp", "mdmp", "cache", "bak", "old", "blf",
            "regtrans-ms", "chk",
        ],
    },
];

/// Index of the "Other" group, after every entry in `KINDS`.
pub const OTHER: usize = KINDS.len();
/// Index of the "No extension" group.
pub const NO_EXT: usize = KINDS.len() + 1;

static BY_EXT: LazyLock<HashMap<&'static [u8], usize>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for (i, k) in KINDS.iter().enumerate() {
        for e in k.exts {
            m.entry(e.as_bytes()).or_insert(i);
        }
    }
    m
});

/// Lowercase extension of a lowercase name, without the dot. Hidden-file style names
/// (".gitignore") and long or spaced suffixes don't count as extensions.
pub fn ext_of(name: &[u8]) -> Option<&[u8]> {
    let dot = name.iter().rposition(|&b| b == b'.')?;
    let ext = &name[dot + 1..];
    if dot == 0 || ext.is_empty() || ext.len() > 16 || ext.contains(&b' ') {
        return None;
    }
    Some(ext)
}

/// Kind index for a lowercase extension (`OTHER` when unknown).
pub fn kind_of_ext(ext: &[u8]) -> usize {
    BY_EXT.get(ext).copied().unwrap_or(OTHER)
}

/// Kind index for a lowercase file name.
pub fn kind_of(name: &[u8]) -> usize {
    ext_of(name).map_or(NO_EXT, kind_of_ext)
}

/// Kind index for a user-typed name like "video", "videos" or "sheet".
pub fn find_kind(text: &str) -> Option<usize> {
    let t = text.to_ascii_lowercase();
    let t = t.trim_end_matches('s');
    if t.is_empty() {
        return None;
    }
    if "other".starts_with(t) {
        return Some(OTHER);
    }
    KINDS
        .iter()
        .position(|k| k.key.starts_with(t) || k.label.to_ascii_lowercase().starts_with(t))
        .or_else(|| KINDS.iter().position(|k| k.key.contains(t)))
}

pub fn kind_label(i: usize) -> &'static str {
    match KINDS.get(i) {
        Some(k) => k.label,
        None if i == NO_EXT => "No extension",
        None => "Other",
    }
}

pub fn kind_key(i: usize) -> &'static str {
    match KINDS.get(i) {
        Some(k) => k.key,
        None if i == NO_EXT => "none",
        None => "other",
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtStat {
    /// Lowercase, without the dot; empty for files without an extension.
    pub ext: String,
    pub kind: &'static str,
    pub files: u64,
    pub size: u64,
    pub alloc: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KindStat {
    pub key: &'static str,
    pub label: &'static str,
    pub files: u64,
    pub size: u64,
    pub alloc: u64,
    /// This kind's extensions found here, biggest first.
    pub exts: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub files: u64,
    pub alloc: u64,
    pub kinds: Vec<KindStat>,
    pub exts: Vec<ExtStat>,
}

type Acc = HashMap<Vec<u8>, (u64, u64, u64)>;

fn add(acc: &mut Acc, tree: &Tree, id: NodeId) {
    let n = tree.get(id);
    if n.is_dir() || n.has(flags::GONE) || n.has(flags::DELETED) || n.has(flags::EXTRA_LINK) {
        return;
    }
    let ext = ext_of(tree.name_lower(id)).unwrap_or_default();
    let e = acc.entry(ext.to_vec()).or_default();
    e.0 += 1;
    e.1 += n.size;
    e.2 += n.alloc;
}

/// Storage per extension and per kind for every file under `root`.
pub fn breakdown(tree: &Tree, root: NodeId) -> Breakdown {
    let acc: Acc = if root == tree.root {
        (1..tree.len())
            .into_par_iter()
            .with_min_len(8192)
            .fold(Acc::new, |mut acc, i| {
                add(&mut acc, tree, i as NodeId);
                acc
            })
            .reduce(Acc::new, |mut a, b| {
                for (k, v) in b {
                    let e = a.entry(k).or_default();
                    e.0 += v.0;
                    e.1 += v.1;
                    e.2 += v.2;
                }
                a
            })
    } else {
        let mut acc = Acc::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            for c in tree.children(id) {
                if tree.get(c).is_dir() {
                    stack.push(c);
                } else {
                    add(&mut acc, tree, c);
                }
            }
        }
        acc
    };

    let mut exts: Vec<ExtStat> = acc
        .into_iter()
        .map(|(ext, (files, size, alloc))| {
            let kind = if ext.is_empty() {
                NO_EXT
            } else {
                kind_of_ext(&ext)
            };
            ExtStat {
                ext: String::from_utf8_lossy(&ext).into_owned(),
                kind: kind_key(kind),
                files,
                size,
                alloc,
            }
        })
        .collect();
    exts.sort_by(|a, b| b.alloc.cmp(&a.alloc).then(b.files.cmp(&a.files)));

    let mut kinds: Vec<KindStat> = (0..=NO_EXT)
        .map(|i| KindStat {
            key: kind_key(i),
            label: kind_label(i),
            files: 0,
            size: 0,
            alloc: 0,
            exts: Vec::new(),
        })
        .collect();
    for e in &exts {
        let k = kinds.iter_mut().find(|k| k.key == e.kind).unwrap();
        k.files += e.files;
        k.size += e.size;
        k.alloc += e.alloc;
        if !e.ext.is_empty() {
            k.exts.push(e.ext.clone());
        }
    }
    kinds.retain(|k| k.files > 0);
    kinds.sort_by(|a, b| b.alloc.cmp(&a.alloc));

    Breakdown {
        files: exts.iter().map(|e| e.files).sum(),
        alloc: exts.iter().map(|e| e.alloc).sum(),
        kinds,
        exts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_kind_and_extension() {
        let mut t = Tree::new("C:");
        let a = t.add(t.root, "A", flags::DIR, 0, 0, 0, 0);
        t.add(a, "clip.MP4", 0, 10, 100, 0, 0);
        t.add(a, "film.mkv", 0, 10, 300, 0, 0);
        t.add(a, "sheet.xlsx", 0, 10, 50, 0, 0);
        t.add(t.root, "data.csv", 0, 10, 20, 0, 0);
        t.add(t.root, "README", 0, 10, 5, 0, 0);
        t.aggregate();

        let b = breakdown(&t, t.root);
        assert_eq!(b.files, 5);
        assert_eq!(b.kinds[0].key, "video");
        assert_eq!(b.kinds[0].alloc, 400);
        assert_eq!(b.kinds[0].exts, vec!["mkv", "mp4"]);
        let sheets = b.kinds.iter().find(|k| k.key == "spreadsheet").unwrap();
        assert_eq!(sheets.alloc, 70);
        assert!(b.exts.iter().any(|e| e.ext.is_empty() && e.kind == "none"));

        let only_a = breakdown(&t, a);
        assert_eq!(only_a.files, 3);
        assert_eq!(find_kind("Videos"), Some(0));
        assert_eq!(find_kind("sheet"), KINDS.iter().position(|k| k.key == "spreadsheet"));
    }
}
