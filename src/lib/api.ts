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
  live: boolean;
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

export interface NodeInfo {
  id: number;
  name: string;
  path: string;
  dir: boolean;
  flags: number;
  size: number;
  alloc: number;
  files: number;
  mtime: number;
  parent: number | null;
}

export interface Crumb {
  id: number;
  name: string;
}

export const TileFlags = { DIR: 1, HEADER: 2, COLLAPSED: 4, MANY_SMALL: 8, IN_CART: 16, CART_ROOT: 32 } as const;

/** Treemap tiles in struct-of-arrays form, straight from the binary response. */
export interface Tiles {
  count: number;
  x: Float32Array;
  y: Float32Array;
  w: Float32Array;
  h: Float32Array;
  id: Uint32Array;
  depth: Uint8Array;
  flags: Uint8Array;
  cat: Uint8Array;
  value: Float64Array;
  labels: Map<number, string>;
}

function decodeTiles(buf: ArrayBuffer): Tiles {
  const dv = new DataView(buf);
  const count = dv.getUint32(0, true);
  const t: Tiles = {
    count,
    x: new Float32Array(count),
    y: new Float32Array(count),
    w: new Float32Array(count),
    h: new Float32Array(count),
    id: new Uint32Array(count),
    depth: new Uint8Array(count),
    flags: new Uint8Array(count),
    cat: new Uint8Array(count),
    value: new Float64Array(count),
    labels: new Map(),
  };
  let o = 4;
  for (let i = 0; i < count; i++, o += 32) {
    t.x[i] = dv.getFloat32(o, true);
    t.y[i] = dv.getFloat32(o + 4, true);
    t.w[i] = dv.getFloat32(o + 8, true);
    t.h[i] = dv.getFloat32(o + 12, true);
    t.id[i] = dv.getUint32(o + 16, true);
    t.depth[i] = dv.getUint8(o + 20);
    t.flags[i] = dv.getUint8(o + 21);
    t.cat[i] = dv.getUint8(o + 22);
    t.value[i] = Number(dv.getBigUint64(o + 24, true));
  }
  const n = dv.getUint32(o, true);
  o += 4;
  const td = new TextDecoder();
  const bytes = new Uint8Array(buf);
  for (let i = 0; i < n; i++) {
    const id = dv.getUint32(o, true);
    const len = dv.getUint16(o + 4, true);
    t.labels.set(id, td.decode(bytes.subarray(o + 6, o + 6 + len)));
    o += 6 + len;
  }
  return t;
}

export interface HistoryEntry {
  path: string;
  taken: number;
  alloc: number;
  files: number;
  method: string;
}

export interface DiffSummary {
  taken: number;
  oldAlloc: number;
  newAlloc: number;
  oldFiles: number;
  newFiles: number;
  oldRoot: number;
  newRoot: number;
}

export interface DiffItem {
  name: string;
  oldId: number | null;
  newId: number | null;
  dir: boolean;
  oldAlloc: number;
  newAlloc: number;
  delta: number;
  oldFiles: number;
  newFiles: number;
  status: 'added' | 'removed' | 'changed';
  hasKids: boolean;
}

export interface SearchHit {
  letter: string;
  id: number;
  name: string;
  path: string;
  dir: boolean;
  alloc: number;
  mtime: number;
}

export interface SearchResult {
  total: number;
  elapsedMs: number;
  hits: SearchHit[];
}

export interface CartItem {
  letter: string;
  id: number;
  path: string;
  name: string;
  dir: boolean;
  alloc: number;
  files: number;
}

export interface DeleteResult {
  freed: number;
  deleted: number;
  failed: { path: string; error: string }[];
  undo: number | null;
}

export interface UndoResult {
  restored: number;
  failed: { path: string; error: string }[];
  needsRescan: string[];
}

export interface DupeFile {
  id: number;
  path: string;
  size: number;
  mtime: number;
}

export interface DupeGroup {
  size: number;
  files: DupeFile[];
}

export type DupesEvent =
  | {
      type: 'progress';
      phase: number;
      filesDone: number;
      filesTotal: number;
      bytesDone: number;
      bytesTotal: number;
    }
  | { type: 'done'; groups: DupeGroup[]; elapsedMs: number; candidates: number }
  | { type: 'cancelled' };

export interface Settings {
  autoSnapshots: boolean;
  snapshotEvery: 'daily' | 'weekly';
  historyKeep: number;
  explorerMenu: boolean;
}

export const api = {
  settingsGet: () => invoke<Settings>('settings_get'),
  settingsSet: (settings: Settings) => invoke<Settings>('settings_set', { settings }),
  snapshotStorage: () => invoke<{ path: string; bytes: number; files: number }>('snapshot_storage'),
  removeFastScan: () => invoke<HelperStatus>('remove_fast_scan'),
  appVersion: () => invoke<string>('app_version'),
  takeOpenPath: () => invoke<string | null>('take_open_path'),
  findPath: (letter: string, path: string) => invoke<number | null>('find_path', { letter, path }),
  dupesStart(
    letter: string,
    root: number | null,
    minSize: number,
    skipSystem: boolean,
    onEvent: (e: DupesEvent) => void,
  ) {
    const ch = new Channel<DupesEvent>();
    ch.onmessage = onEvent;
    return invoke<void>('dupes_start', { letter, root, minSize, skipSystem, onEvent: ch });
  },
  dupesCancel: () => invoke<void>('dupes_cancel'),
  cartGet: () => invoke<CartItem[]>('cart_get'),
  cartAdd: (letter: string, ids: number[]) => invoke<CartItem[]>('cart_add', { letter, ids }),
  cartRemove: (paths: string[]) => invoke<CartItem[]>('cart_remove', { paths }),
  cartClear: () => invoke<void>('cart_clear'),
  cartDelete: (permanent: boolean) => invoke<DeleteResult>('cart_delete', { permanent }),
  cartUndo: (id: number) => invoke<UndoResult>('cart_undo', { id }),
  search: (query: string, letter?: string) =>
    invoke<SearchResult>('search', { query, letter: letter ?? null }),
  history: (letter: string) => invoke<HistoryEntry[]>('history', { letter }),
  diffOpen: (letter: string, path: string) => invoke<DiffSummary>('diff_open', { letter, path }),
  diffChildren: (letter: string, oldId: number | null, newId: number | null) =>
    invoke<DiffItem[]>('diff_children', { letter, oldId, newId }),
  diffClose: (letter: string) => invoke<void>('diff_close', { letter }),
  async treemap(letter: string, root: number | null, width: number, height: number) {
    const buf = await invoke<ArrayBuffer>('treemap', { letter, root, width, height });
    return decodeTiles(buf);
  },
  async sunburst(letter: string, root: number | null, depth: number) {
    const buf = await invoke<ArrayBuffer>('sunburst', { letter, root, depth });
    return decodeTiles(buf);
  },
  nodeInfo: (letter: string, id: number) => invoke<NodeInfo>('node_info', { letter, id }),
  ancestors: (letter: string, id: number) => invoke<Crumb[]>('ancestors', { letter, id }),
  drives: () => invoke<Drive[]>('drives'),
  restored: () => invoke<boolean>('restored'),
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
  openPath: (path: string) => invoke<void>('open_path', { path }),
  export: (letter: string, root: number | null, format: 'html' | 'csv' | 'json', path: string) =>
    invoke<void>('export', { letter, root, format, path }),
  backdrop: () => invoke<'mica' | 'solid'>('backdrop'),
};
