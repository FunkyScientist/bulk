import { save } from '@tauri-apps/plugin-dialog';
import FileText from '@lucide/svelte/icons/file-text';
import Sheet from '@lucide/svelte/icons/sheet';
import Braces from '@lucide/svelte/icons/braces';
import { api } from './api';
import { app, type MenuItem } from './app.svelte';

const formats = {
  html: { name: 'HTML report', ext: 'html', label: 'HTML report (shareable page)', icon: FileText },
  csv: { name: 'CSV', ext: 'csv', label: 'CSV list of every file', icon: Sheet },
  json: { name: 'JSON', ext: 'json', label: 'JSON folder tree', icon: Braces },
} as const;

type Format = keyof typeof formats;

async function run(letter: string, format: Format) {
  const root = app.focus[letter] ?? null;
  const f = formats[format];
  const stamp = new Date().toISOString().slice(0, 10);
  const path = await save({
    title: `Export ${f.name}`,
    defaultPath: `Bulk ${letter} ${stamp}.${f.ext}`,
    filters: [{ name: f.name, extensions: [f.ext] }],
  });
  if (!path) return;
  app.toast(`Exporting ${f.name}…`, { ms: 1500 });
  try {
    await api.export(letter, root, format, path);
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

/** Menu for the Export button. Exports the folder shown in the chart, or the drive. */
export function exportMenu(e: MouseEvent, letter: string) {
  const items: MenuItem[] = (Object.keys(formats) as Format[]).map((k) => ({
    label: formats[k].label,
    icon: formats[k].icon,
    run: () => run(letter, k),
  }));
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  app.openMenu(
    { clientX: r.left, clientY: r.bottom + 6, preventDefault() {} } as MouseEvent,
    items,
  );
}
