import { save } from '@tauri-apps/plugin-dialog';
import FileText from '@lucide/svelte/icons/file-text';
import Sheet from '@lucide/svelte/icons/sheet';
import Braces from '@lucide/svelte/icons/braces';
import { api } from './api';
import { app, type MenuItem } from './app.svelte';

const formats = {
  html: { name: 'HTML report', ext: 'html', icon: FileText },
  csv: { name: 'CSV', ext: 'csv', icon: Sheet },
  json: { name: 'JSON', ext: 'json', icon: Braces },
} as const;

type Format = keyof typeof formats;

/** What each tab exports, and how its menu describes the formats. */
const kinds = {
  browse: {
    file: '',
    labels: {
      html: 'HTML report',
      csv: 'CSV list of every file',
      json: 'JSON folder tree',
    },
  },
  changes: {
    file: ' changes',
    labels: {
      html: 'HTML report of the changes',
      csv: 'CSV of every change',
      json: 'JSON of every change',
    },
  },
  duplicates: {
    file: ' duplicates',
    labels: {
      html: 'HTML report of the duplicates',
      csv: 'CSV of every duplicate file',
      json: 'JSON of the duplicate sets',
    },
  },
} as const;

type Kind = keyof typeof kinds;

/** Whether the current tab has anything to export. */
export function canExport(letter: string): boolean {
  if (app.tab === 'types') return false;
  if (app.tab === 'changes') return app.diffView?.letter === letter && app.diffView.items > 0;
  if (app.tab === 'duplicates') return (app.dupes[letter]?.groups.length ?? 0) > 0;
  return true;
}

async function run(letter: string, kind: Kind, format: Format) {
  const f = formats[format];
  const stamp = new Date().toISOString().slice(0, 10);
  const path = await save({
    title: `Export ${f.name}`,
    defaultPath: `Bulk ${letter}${kinds[kind].file} ${stamp}.${f.ext}`,
    filters: [{ name: f.name, extensions: [f.ext] }],
  });
  if (!path) return;
  app.toast(`Exporting ${f.name}…`, { ms: 1500 });
  try {
    if (kind === 'changes')
      await api.exportChanges(letter, app.diffView?.scope ?? null, format, path);
    else if (kind === 'duplicates') {
      const d = app.dupes[letter];
      await api.exportDupes(d.scope, d.groups, format, path);
    } else await api.export(letter, app.focus[letter] ?? null, format, path);
    app.toast(`Saved ${path.slice(path.lastIndexOf('\\') + 1)}`, {
      tone: 'ok',
      action:
        format === 'html'
          ? { label: 'Open', run: () => api.openPath(path) }
          : { label: 'Show', run: () => api.showInExplorer(path) },
    });
  } catch (e) {
    app.toast(`Export failed: ${e}`, { tone: 'error' });
  }
}

/** Menu for the Export button: the folder shown in the chart, or what the Changes or
 * Duplicates tab is showing. */
export function exportMenu(e: MouseEvent, letter: string) {
  if (app.tab === 'types') return;
  const kind: Kind = app.tab;
  const items: MenuItem[] = (Object.keys(formats) as Format[]).map((k) => ({
    label: kinds[kind].labels[k],
    icon: formats[k].icon,
    run: () => run(letter, kind, k),
  }));
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  app.openMenu(
    { clientX: r.left, clientY: r.bottom + 6, preventDefault() {} } as MouseEvent,
    items,
  );
}
