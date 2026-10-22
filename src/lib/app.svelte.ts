import { listen } from '@tauri-apps/api/event';
import type { Component } from 'svelte';
import {
  api,
  type CartItem,
  type DeleteResult,
  type Drive,
  type HelperStatus,
  type Progress,
  type SortKey,
} from './api';
import { bytes } from './format';

export type ThemePref = 'system' | 'dark' | 'light';

export interface MenuItem {
  label: string;
  icon?: Component;
  hint?: string;
  danger?: boolean;
  run: () => void;
}

export interface Toast {
  id: number;
  message: string;
  tone?: 'ok' | 'error';
  action?: { label: string; run: () => void };
}

export interface ScanState {
  method: 'mft' | 'walk' | null;
  progress: Progress;
  startedAt: number;
  /** Name of the folder being rescanned, when it isn't the whole drive. */
  folder?: string;
  /** Set briefly after a scan finishes so the UI can celebrate it. */
  finished?: { elapsedMs: number; files: number };
  error?: string;
}

const emptyProgress: Progress = { files: 0, dirs: 0, bytes: 0, done: 0, total: 0 };

export type Layout = 'columns' | 'rows';

function readPref<T extends string>(key: string, fallback: T): T {
  try {
    return (localStorage.getItem(key) as T) ?? fallback;
  } catch {
    return fallback;
  }
}

function writePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage can be unavailable; the preference just won't persist.
  }
}

class AppState {
  drives = $state<Drive[]>([]);
  current = $state<string | null>(null);
  helper = $state<HelperStatus>('checking');
  scans = $state<Record<string, ScanState>>({});
  theme = $state<ThemePref>(readPref('bulk.theme', 'system'));
  systemDark = $state(true);
  sort = $state<SortKey>(readPref('bulk.sort', 'alloc'));
  /** Bumped whenever the current drive's rows change and must be refetched. */
  rowsVersion = $state(0);
  selected = $state<number | null>(null);
  /** Folder the visualisation is zoomed into, per drive (null = drive root). */
  focus = $state<Record<string, number | null>>({});
  /** Asks the tree list to expand to and scroll to a node. */
  revealRequest = $state<{ id: number; seq: number } | null>(null);
  visual = $state<'treemap' | 'sunburst'>(readPref('bulk.visual', 'treemap'));
  /** Files beside the chart (columns) or above it (rows). */
  layout = $state<Layout>(readPref<Layout>('bulk.layout', 'columns'));
  split = $state(Number(readPref('bulk.split', '0.5')) || 0.5);
  tab = $state<'browse' | 'changes' | 'duplicates'>('browse');
  cart = $state<CartItem[]>([]);
  cartOpen = $state(false);
  /** Bumped when something is added, so the cart button can bounce. */
  cartBump = $state(0);
  menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  toasts = $state<Toast[]>([]);
  private toastSeq = 0;
  sidebarCollapsed = $state(readPref<'open' | 'collapsed'>('bulk.sidebar', 'open') === 'collapsed');

  drive = $derived(this.drives.find((d) => d.letter === this.current) ?? null);
  resolvedTheme = $derived<'dark' | 'light'>(
    this.theme === 'system' ? (this.systemDark ? 'dark' : 'light') : this.theme,
  );

  async init() {
    const mq = matchMedia('(prefers-color-scheme: dark)');
    this.systemDark = mq.matches;
    mq.addEventListener('change', (e) => (this.systemDark = e.matches));

    await listen<HelperStatus>('helper-status', (e) => (this.helper = e.payload));
    await listen<string>('drive-updated', (e) => this.driveChanged(e.payload));
    await listen<string>('drive-live', (e) => this.driveChanged(e.payload));
    // The change journal couldn't cover the gap since the last snapshot.
    await listen<string>('needs-rescan', (e) => void this.scan(e.payload));
    await listen('cart-changed', async () => (this.cart = await api.cartGet()));
    // "Analyze with Bulk" from Explorer while we're already open.
    await listen<string>('open-path', (e) => void this.openPath(e.payload));
    this.cart = await api.cartGet();
    const [drives, helper] = await Promise.all([api.drives(), api.helperStatus()]);
    this.drives = drives;
    if (this.helper === 'checking') this.helper = helper;
    this.current = drives.find((d) => d.autoScan)?.letter ?? drives[0]?.letter ?? null;

    void this.autoScan();
    // Keep free space and "as of …" times current.
    setInterval(() => void this.refreshDrives(), 60_000);
  }

  private pendingChanged = new Set<string>();
  private changeTimer: ReturnType<typeof setTimeout> | undefined;

  /** Coalesce bursts of backend updates (live file changes) into one UI refresh. */
  private driveChanged(letter: string) {
    this.pendingChanged.add(letter);
    if (this.changeTimer) return;
    this.changeTimer = setTimeout(async () => {
      this.changeTimer = undefined;
      const letters = [...this.pendingChanged];
      this.pendingChanged.clear();
      await this.refreshDrives();
      if (this.current && letters.includes(this.current) && !this.isScanning(this.current))
        this.rowsVersion++;
    }, 600);
  }

  /** Scan local drives that have no saved snapshot, once startup settles. */
  private async autoScan() {
    const deadline = Date.now() + 5000;
    while ((this.helper === 'checking' || !(await api.restored())) && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 50));
    }
    await this.refreshDrives();
    if (this.current) this.rowsVersion++;
    // Without the fast path a full walk is slow, so start with the selected drive only.
    const targets =
      this.helper === 'ready'
        ? this.drives.filter((d) => d.autoScan && !d.scanned)
        : this.drives.filter((d) => d.letter === this.current && !d.scanned);
    for (const d of targets) await this.scan(d.letter);
  }

  async refreshDrives() {
    this.drives = await api.drives();
  }

  select(letter: string) {
    if (this.current === letter) return;
    this.current = letter;
    this.selected = null;
    // Node ids belong to one drive; don't let the new drive's list act on them.
    this.revealRequest = null;
    this.rowsVersion++;
  }

  /** Select a node; when it came from the visualisation, show it in the list too. */
  selectNode(id: number, reveal = false) {
    this.selected = id;
    if (reveal) this.revealRequest = { id, seq: (this.revealRequest?.seq ?? 0) + 1 };
  }

  /** Show a folder from Explorer: switch to its drive, scanning first if needed. */
  async openPath(path: string) {
    const letter = path[0].toUpperCase();
    if (!this.drives.some((d) => d.letter === letter)) await this.refreshDrives();
    if (!this.drives.some((d) => d.letter === letter)) {
      this.toast(`Drive ${letter}: isn't available`, { tone: 'error' });
      return;
    }
    this.select(letter);
    // Wait out startup restore, then scan if there's still nothing to show.
    for (let i = 0; i < 100 && !(await api.restored()); i++)
      await new Promise((r) => setTimeout(r, 50));
    await this.refreshDrives();
    if (!this.drive?.scanned) await this.scan(letter);
    const id = await api.findPath(letter, path);
    if (id === null) {
      this.toast(`Couldn't find ${path}`, { tone: 'error' });
      return;
    }
    this.tab = 'browse';
    if (id === 0) this.setFocus(letter, null);
    else {
      this.setFocus(letter, id);
      this.selectNode(id, true);
    }
  }

  /** Jump to a node: switch drive and tab, reveal it in the list and show its folder. */
  async goTo(letter: string, id: number) {
    if (this.current !== letter) this.select(letter);
    this.tab = 'browse';
    const info = await api.nodeInfo(letter, id);
    this.setFocus(letter, info.parent === null || info.parent === 0 ? null : info.parent);
    this.selectNode(id, true);
  }

  toast(message: string, opts: Omit<Toast, 'id' | 'message'> & { ms?: number } = {}) {
    const id = ++this.toastSeq;
    this.toasts.push({ id, message, tone: opts.tone, action: opts.action });
    setTimeout(() => this.dismissToast(id), opts.ms ?? (opts.action ? 7000 : 3200));
  }

  dismissToast(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  openMenu(e: MouseEvent, items: MenuItem[]) {
    e.preventDefault();
    this.menu = { x: e.clientX, y: e.clientY, items };
  }

  async addToCart(letter: string, ids: number[]) {
    const before = this.cart.length;
    this.cart = await api.cartAdd(letter, ids);
    if (this.cart.length !== before || ids.length) {
      this.cartBump++;
      this.toast(
        ids.length === 1 ? 'Added to cleanup cart' : `Added ${ids.length} items to cleanup cart`,
        {
          action: { label: 'Review', run: () => (this.cartOpen = true) },
        },
      );
    }
  }

  async removeFromCart(paths: string[]) {
    this.cart = await api.cartRemove(paths);
  }

  async copyPath(letter: string, id: number) {
    const path = await api.nodePath(letter, id);
    await navigator.clipboard.writeText(path);
    this.toast('Path copied');
  }

  /** Asks for confirmation before deleting a node (Shift+Del, right-click → Delete). */
  deleteRequest = $state<{ letter: string; id: number } | null>(null);

  /** Report a delete (from the cart or the dialog) with an Undo for recycled items. */
  reportDelete(r: DeleteResult, permanent: boolean) {
    if (r.deleted) {
      const what = r.deleted === 1 ? '1 item' : `${r.deleted.toLocaleString()} items`;
      this.toast(
        permanent
          ? `Deleted ${what} · freed ${bytes(r.freed)}`
          : `Moved ${what} (${bytes(r.freed)}) to the Recycle Bin`,
        {
          tone: 'ok',
          action:
            r.undo !== null
              ? { label: 'Undo', run: () => this.undoDelete(r.undo as number) }
              : undefined,
        },
      );
    }
    if (r.failed.length)
      this.toast(`${r.failed.length} couldn't be deleted: ${r.failed[0].error}`, {
        tone: 'error',
        ms: 8000,
      });
  }

  async undoDelete(id: number) {
    const r = await api.cartUndo(id);
    if (r.restored)
      this.toast(`Restored ${r.restored === 1 ? '1 item' : `${r.restored} items`}`, { tone: 'ok' });
    if (r.failed.length)
      this.toast(`Couldn't restore ${r.failed.length}: ${r.failed[0].error}`, { tone: 'error' });
    for (const l of r.needsRescan)
      this.toast(`Rescan ${l}: to see restored items`, {
        action: { label: 'Rescan', run: () => this.scan(l) },
      });
  }

  /** Delete key: add the node to the cart, or take it out if it's already there. */
  async toggleCart(letter: string, id: number) {
    const path = (await api.nodePath(letter, id)).toLowerCase();
    const entry = this.cart.find((c) => c.path.toLowerCase() === path);
    if (entry) {
      await this.removeFromCart([entry.path]);
      this.toast('Removed from cleanup cart');
    } else if (this.inCart(path)) {
      this.toast('Its folder is already in the cleanup cart', {
        action: { label: 'Review', run: () => (this.cartOpen = true) },
      });
    } else {
      await this.addToCart(letter, [id]);
    }
  }

  inCart(path: string) {
    const p = path.toLowerCase();
    return this.cart.some((c) => {
      const cp = c.path.toLowerCase();
      return p === cp || p.startsWith(cp.replace(/\\$/, '') + '\\');
    });
  }

  /** Bumped to ask the visualisation to go up one level. */
  zoomOutSeq = $state(0);
  zoomOut() {
    this.zoomOutSeq++;
  }

  setFocus(letter: string, id: number | null) {
    this.focus[letter] = id;
  }

  setVisual(v: 'treemap' | 'sunburst') {
    this.visual = v;
    writePref('bulk.visual', v);
  }

  toggleSidebar() {
    this.sidebarCollapsed = !this.sidebarCollapsed;
    writePref('bulk.sidebar', this.sidebarCollapsed ? 'collapsed' : 'open');
  }

  setSplit(v: number) {
    this.split = Math.min(0.75, Math.max(0.25, v));
    writePref('bulk.split', String(this.split));
  }

  setLayout(l: Layout) {
    this.layout = l;
    writePref('bulk.layout', l);
  }

  setTheme(t: ThemePref) {
    this.theme = t;
    writePref('bulk.theme', t);
  }

  cycleTheme() {
    const order: ThemePref[] = ['system', 'dark', 'light'];
    this.setTheme(order[(order.indexOf(this.theme) + 1) % order.length]);
  }

  async setSort(key: SortKey) {
    this.sort = key;
    writePref('bulk.sort', key);
    await Promise.all(this.drives.filter((d) => d.scanned).map((d) => api.setSort(d.letter, key)));
    this.rowsVersion++;
  }

  isScanning(letter: string) {
    const s = this.scans[letter];
    return !!s && !s.finished && !s.error;
  }

  /** Rescan the folder open in the chart, or the whole drive when none is. */
  async scanFolder(letter: string) {
    const id = this.focus[letter] ?? null;
    if (id === null) return this.scan(letter);
    const info = await api.nodeInfo(letter, id);
    return this.scan(letter, { id, name: info.name });
  }

  /** Start a scan (of the drive, or one folder in it) and resolve when it ends. */
  scan(letter: string, folder?: { id: number; name: string }): Promise<void> {
    if (this.isScanning(letter)) return Promise.resolve();
    this.scans[letter] = {
      method: null,
      progress: { ...emptyProgress },
      startedAt: Date.now(),
      folder: folder?.name,
    };
    // A whole-drive scan builds a new tree with new ids, so remember the open
    // folder and the selection by path and find them again afterwards.
    const pathOf = (id: number | null | undefined) =>
      id == null ? Promise.resolve(null) : api.nodePath(letter, id).catch(() => null);
    const kept = folder
      ? null
      : Promise.all([
          pathOf(this.focus[letter]),
          pathOf(this.current === letter ? this.selected : null),
        ]);
    const restore = async () => {
      if (!kept) return;
      const [focusPath, selPath] = await kept;
      const find = (p: string | null) => (p ? api.findPath(letter, p) : Promise.resolve(null));
      const [focus, sel] = await Promise.all([find(focusPath), find(selPath)]);
      this.focus[letter] = focus === 0 ? null : focus;
      return this.current === letter ? sel : null;
    };
    return new Promise((resolve) => {
      api
        .scan(letter, folder?.id ?? null, async (e) => {
          const s = this.scans[letter];
          if (!s) return;
          switch (e.type) {
            case 'started':
              s.method = e.method;
              break;
            case 'progress':
              s.progress = {
                files: e.files,
                dirs: e.dirs,
                bytes: e.bytes,
                done: e.done,
                total: e.total,
              };
              break;
            case 'done':
              s.finished = { elapsedMs: e.elapsedMs, files: e.files };
              const sel = await restore();
              await api.setSort(letter, this.sort);
              await this.refreshDrives();
              if (letter === this.current) this.rowsVersion++;
              if (kept && letter === this.current) {
                if (sel != null) this.selectNode(sel, true);
                else this.selected = null;
              }
              setTimeout(() => {
                if (this.scans[letter]?.finished) delete this.scans[letter];
              }, 2600);
              resolve();
              break;
            case 'cancelled':
              delete this.scans[letter];
              resolve();
              break;
            case 'error':
              s.error = e.message;
              resolve();
              break;
          }
        })
        .catch((err) => {
          this.scans[letter] = { ...this.scans[letter], error: String(err) };
          resolve();
        });
      void this.refreshDrives();
    });
  }

  async cancel(letter: string) {
    await api.cancelScan(letter);
  }

  /** Ask Windows again for the helper's permission, after it was declined. */
  async allowFastMode(): Promise<boolean> {
    try {
      this.helper = await api.allowFastMode();
      return this.helper === 'ready';
    } catch {
      return false;
    }
  }
}

export const app = new AppState();
