import { listen } from '@tauri-apps/api/event';
import { api, type Drive, type HelperStatus, type Progress, type SortKey } from './api';

export type ThemePref = 'system' | 'dark' | 'light';

export interface ScanState {
  method: 'mft' | 'walk' | null;
  progress: Progress;
  startedAt: number;
  /** Set briefly after a scan finishes so the UI can celebrate it. */
  finished?: { elapsedMs: number; files: number };
  error?: string;
}

const emptyProgress: Progress = { files: 0, dirs: 0, bytes: 0, done: 0, total: 0 };

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
  split = $state(Number(readPref('bulk.split', '0.5')) || 0.5);

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
    const [drives, helper] = await Promise.all([api.drives(), api.helperStatus()]);
    this.drives = drives;
    if (this.helper === 'checking') this.helper = helper;
    this.current = drives.find((d) => d.autoScan)?.letter ?? drives[0]?.letter ?? null;

    void this.autoScan();
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
    this.rowsVersion++;
  }

  /** Select a node; when it came from the visualisation, show it in the list too. */
  selectNode(id: number, reveal = false) {
    this.selected = id;
    if (reveal) this.revealRequest = { id, seq: (this.revealRequest?.seq ?? 0) + 1 };
  }

  setFocus(letter: string, id: number | null) {
    this.focus[letter] = id;
  }

  setVisual(v: 'treemap' | 'sunburst') {
    this.visual = v;
    writePref('bulk.visual', v);
  }

  setSplit(v: number) {
    this.split = Math.min(0.75, Math.max(0.25, v));
    writePref('bulk.split', String(this.split));
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

  /** Start a scan and resolve when it ends. */
  scan(letter: string): Promise<void> {
    if (this.isScanning(letter)) return Promise.resolve();
    this.scans[letter] = { method: null, progress: { ...emptyProgress }, startedAt: Date.now() };
    return new Promise((resolve) => {
      api
        .scan(letter, async (e) => {
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
              await api.setSort(letter, this.sort);
              await this.refreshDrives();
              if (letter === this.current) this.rowsVersion++;
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

  async enableFastScan(): Promise<boolean> {
    try {
      this.helper = await api.enableFastScan();
      return this.helper === 'ready';
    } catch {
      return false;
    }
  }
}

export const app = new AppState();
