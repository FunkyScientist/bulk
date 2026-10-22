# Bulk

A fast, friendly disk space analyzer for Windows. Reads the NTFS master file table
directly, so a full scan of a 2-million-file drive takes about 5 seconds, then keeps
it up to date live from the change journal.

## Features

- **Fast mode** (on by default): MFT-level scanning through a small elevated helper
  (one UAC prompt, on first launch). Turn it off in Settings to walk folders instead;
  other drives (FAT/exFAT, USB) always use the parallel folder walk.
- **Instant reopen and live view**: the last snapshot loads at startup, the change
  journal catches it up, and changes appear as they happen.
- **What changed**: compare against any earlier snapshot, folder by folder.
- **Treemap and sunburst** with smooth zoom, synced with the file list.
- **Command palette** (Ctrl+K): search every drive, with filters such as
  `ext:mp4 size:>1GB older:1y in:downloads`, and run any command.
- **Cleanup cart**: collect items, review, then send to the Recycle Bin (with undo)
  or delete permanently.
- **Duplicate finder**: matches by content (size, then sampled hash, then BLAKE3).
- **Scheduled snapshots** (off by default): daily or weekly in the background, kept
  for a set count. The scheduled task only exists while they're turned on.
- **Explorer integration**: "Analyze with Bulk" on any folder or drive.
- **Export**: a self-contained HTML report, a CSV of every file, or a JSON folder tree.
- Keyboard-first: arrows, Enter, Backspace, Delete, `?` for the full list.

## Layout

```
crates/bulk-core     scanning, tree model, layouts, search, diff, snapshots (no UI)
crates/bulk-helper   elevated helper: MFT reads and journal watching over a named pipe
src-tauri            the app's backend (Tauri commands and state)
src                  Svelte 5 UI
```

## Developing

Needs Rust (MSVC), Node 20+ and pnpm.

```
pnpm install
pnpm tauri dev
```

On first launch Windows asks once to register the helper's scheduled task; after that
scans need no prompt. If that prompt is declined, the sidebar offers to ask again.

Useful tools:

```
cargo test -p bulk-core
cargo run --release -p bulk-core --example bench -- C: walk     # folder walk timing
cargo run --release -p bulk-core --example bench -- C: mft      # MFT timing (run elevated)
cargo run --release -p bulk-helper --bin probe -- C             # scan through the helper
```

Notes:

- `pnpm dev:app` asks a running helper to exit before rebuilding it, since Windows
  locks a running exe.
- When the dev app is launched from inside a packaged app (like the Claude desktop
  app), Windows redirects its AppData writes into that package's folder, while the
  helper's scheduled snapshots write to the real `%LOCALAPPDATA%\Bulk`. Installed
  builds aren't affected.

## Building the installer

```
pnpm bundle
```

(`tauri.release.conf.json` adds the helper to the installer; it's kept out of the main
config because Tauri would otherwise copy it over the dev helper in `target/debug`.)

Produces a per-machine NSIS installer in `target/release/bundle/nsis`. Installing into
Program Files matters: the helper runs elevated without a prompt, so its exe must not be
writable by normal processes. Uninstalling removes the scheduled tasks and the Explorer
entry.
