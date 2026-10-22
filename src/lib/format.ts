const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

/** 1536 -> "1.5 KB". Binary units, as Explorer shows them. */
export function bytes(n: number, digits = 1): string {
  if (!Number.isFinite(n) || n <= 0) return '0 B';
  let i = Math.min(UNITS.length - 1, Math.floor(Math.log2(n) / 10));
  let v = n / 2 ** (10 * i);
  // Avoid "1024.0 MB": roll over to the next unit.
  if (v >= 999.95 && i < UNITS.length - 1) {
    v /= 1024;
    i += 1;
  }
  const d = i === 0 ? 0 : v >= 100 ? 0 : digits;
  return `${v.toFixed(d)} ${UNITS[i]}`;
}

/** Split for styling the number and unit separately. */
export function bytesParts(n: number, digits = 1): [string, string] {
  const s = bytes(n, digits);
  const i = s.lastIndexOf(' ');
  return [s.slice(0, i), s.slice(i + 1)];
}

const nf = new Intl.NumberFormat();
export function count(n: number): string {
  return nf.format(Math.round(n));
}

export function percent(x: number): string {
  if (x <= 0) return '0%';
  if (x < 0.001) return '<0.1%';
  return `${(x * 100).toFixed(x < 0.1 ? 1 : 0)}%`;
}

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
/** Unix seconds -> "3 days ago". */
export function ago(unix: number): string {
  if (!unix) return '';
  const s = unix - Date.now() / 1000;
  const abs = Math.abs(s);
  if (abs < 45) return 'just now';
  if (abs < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (abs < 86400) return rtf.format(Math.round(s / 3600), 'hour');
  if (abs < 86400 * 30) return rtf.format(Math.round(s / 86400), 'day');
  if (abs < 86400 * 365) return rtf.format(Math.round(s / (86400 * 30)), 'month');
  return rtf.format(Math.round(s / (86400 * 365)), 'year');
}

const df = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
export function date(unix: number): string {
  return unix ? df.format(new Date(unix * 1000)) : '';
}

export function duration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}
