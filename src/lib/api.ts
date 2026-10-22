import { Channel, invoke } from '@tauri-apps/api/core';

export type DriveKind =
  'fixed' | 'removable' | 'network' | 'virtual' | 'optical' | 'ram' | 'unknown';

export interface Drive {
  letter: string;
  root: string;
  label: string;
  fs: string;
  serial: number;
  kind: DriveKind;
  total: number;
  free: number;
  cluster: number;
  autoScan: boolean;
  scanning: boolean;
  scanned: boolean;
  scannedAt: number | null;
  method: 'mft' | 'walk' | null;
  files: number;
  alloc: number;
}

export type HelperStatus = 'checking' | 'ready' | 'notInstalled' | 'outdated';

export interface Progress {
  files: number;
  dirs: number;
  bytes: number;
  done: number;
  total: number;
}

export type ScanEvent =
  | { type: 'started'; method: 'mft' | 'walk' }
  | ({ type: 'progress' } & Progress)
  | { type: 'done'; elapsedMs: number; files: number; alloc: number }
  | { type: 'error'; message: string }
  | { type: 'cancelled' };

export interface Row {
  id: number;
  depth: number;
  name: string;
  dir: boolean;
  flags: number;
  size: number;
  alloc: number;
  files: number;
  mtime: number;
  hasKids: boolean;
  open: boolean;
  share: number;
}

export type SortKey = 'size' | 'alloc' | 'files' | 'name' | 'modified';

export const Flags = {
  DIR: 1 << 0,
  HIDDEN: 1 << 1,
  SYSTEM: 1 << 2,
  REPARSE: 1 << 3,
  COMPRESSED: 1 << 4,
  SPARSE: 1 << 5,
  EXTRA_LINK: 1 << 6,
  OFFLINE: 1 << 8,
  META: 1 << 9,
} as const;

export const api = {
  drives: () => invoke<Drive[]>('drives'),
  helperStatus: () => invoke<HelperStatus>('helper_status'),
  enableFastScan: () => invoke<HelperStatus>('enable_fast_scan'),
  scan(letter: string, onEvent: (e: ScanEvent) => void) {
    const ch = new Channel<ScanEvent>();
    ch.onmessage = onEvent;
    return invoke<void>('scan', { letter, onEvent: ch });
  },
  cancelScan: (letter: string) => invoke<void>('cancel_scan', { letter }),
  rows: (letter: string, start: number, count: number) =>
    invoke<{ total: number; rows: Row[] }>('rows', { letter, start, count }),
  toggle: (letter: string, id: number, open?: boolean) =>
    invoke<number>('toggle', { letter, id, open }),
  setSort: (letter: string, key: SortKey) => invoke<number>('set_sort', { letter, key }),
  reveal: (letter: string, id: number) => invoke<number | null>('reveal', { letter, id }),
  nodePath: (letter: string, id: number) => invoke<string>('node_path', { letter, id }),
  showInExplorer: (path: string) => invoke<void>('show_in_explorer', { path }),
  backdrop: () => invoke<'mica' | 'solid'>('backdrop'),
};
