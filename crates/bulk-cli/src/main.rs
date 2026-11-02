//! `bulk`: Bulk from the command line. Reads the same snapshots as the app, keeps
//! them current through the elevated helper, and can open folders in the app.

mod delete;
mod find;
mod load;
mod out;
mod read;
mod theme;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

const QUERY_HELP: &str = "\
Search syntax (bulk search, --filter, bulk find)
  tax 2024            names containing every word
  \"tax return\"        the phrase, spaces included
  IMG_2024*.jpg       whole names: * is any text, ? one character
  Downloads/invoice   inside those folders, in that order
  ext:mp4,mkv         extension (ext:none for files without one)
  kind:video          a group of extensions: video, image, audio, document,
                      spreadsheet, archive, program, code, diskimage, ...
  size:>1GB           size on disk; also size:<10mb, size:100mb..2gb
  older:1y newer:2w   modified more / less than h, d, w, m, y ago
  modified:2024       a year, month (2024-06) or day; also >, <
  type:dir type:file  folders or files only
  is:empty is:hidden  also is:system, is:new
  files:>1000         folders holding at least that many files
  in:downloads        inside a folder whose name contains this
  under:\"C:\\Users\\me\" inside exactly that folder
  drive:c,d           only those drives
  -ext:tmp !in:cache  leave out anything matching
  mp4|mkv  a OR b     either one
  sort:size           also small, name, newest, oldest, files";

#[derive(Parser)]
#[command(
    name = "bulk",
    bin_name = "bulk",
    version,
    about = "See what's using your disk space, from the command line.",
    long_about = "See what's using your disk space, from the command line.\n\n\
        `bulk <folder>` opens a folder in the Bulk app; `bulk` on its own searches \
        interactively. The other commands print tables, or JSON with --json.",
    args_conflicts_with_subcommands = true,
    after_long_help = QUERY_HELP
)]
struct Cli {
    /// Folder or drive to open in the Bulk app, e.g. `bulk .`
    path: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open a folder or file in the Bulk app (what `bulk <path>` does)
    Open {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List drives with their free space
    Drives(read::DrivesArgs),
    /// What's biggest in a folder
    Top(read::TopArgs),
    /// Search files and folders
    #[command(after_long_help = QUERY_HELP)]
    Search(read::SearchArgs),
    /// Space used by each kind of file, or each extension
    Types(read::TypesArgs),
    /// Search and browse interactively (what `bulk` on its own does)
    #[command(after_long_help = QUERY_HELP)]
    Find(find::FindArgs),
    /// Move files or folders to the Recycle Bin (lists them first; add --yes)
    Delete(delete::DeleteArgs),
    /// Put back what `bulk delete` recycled, or list recent deletes
    Restore(delete::RestoreArgs),
}

fn main() {
    let cli = Cli::parse();
    let r = match (cli.cmd, cli.path) {
        (Some(cmd), _) => run(cmd),
        (None, Some(path)) => open(&path),
        (None, None) => find::find(find::FindArgs::default()),
    };
    if let Err(e) = r {
        eprintln!("bulk: {e:#}");
        std::process::exit(1);
    }
}

fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Open { path } => open(&path),
        Cmd::Drives(a) => read::drives(a),
        Cmd::Top(a) => read::top(a),
        Cmd::Search(a) => read::search(a),
        Cmd::Types(a) => read::types(a),
        Cmd::Find(a) => find::find(a),
        Cmd::Delete(a) => delete::delete(a),
        Cmd::Restore(a) => delete::restore(a),
    }
}

fn open(path: &std::path::Path) -> anyhow::Result<()> {
    let p = load::absolute(path)?;
    anyhow::ensure!(std::path::Path::new(&p).exists(), "{p} doesn't exist");
    bulk_core::os::open_in_app(&p)
}
