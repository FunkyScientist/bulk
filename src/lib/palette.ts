/** File-type colours, indexed by the category the backend assigns (see layout.rs). */
export const categories = [
  { name: 'Folder', dark: '#8b7cff', light: '#6a58f5' },
  { name: 'Other', dark: '#7d8496', light: '#9aa1b2' },
  { name: 'Video', dark: '#ff6b8b', light: '#f0506f' },
  { name: 'Images', dark: '#f5b544', light: '#e0a020' },
  { name: 'Audio', dark: '#c38bff', light: '#a865f5' },
  { name: 'Archives', dark: '#ff9a5a', light: '#f27c35' },
  { name: 'Code', dark: '#3fd0c9', light: '#16b2aa' },
  { name: 'Documents', dark: '#5fa8ff', light: '#3b88f0' },
  { name: 'Programs', dark: '#9d92ff', light: '#7a6cf2' },
  { name: 'Disk images', dark: '#f37bc4', light: '#e055a8' },
  { name: 'Data & logs', dark: '#8fb3a8', light: '#6f9a8d' },
] as const;

export function categoryColor(cat: number, theme: 'dark' | 'light'): string {
  return (categories[cat] ?? categories[1])[theme];
}

/** Readable label colour on top of a tile filled with `hex` (light theme). */
export function inkOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const lin = (v: number) => {
    const c = v / 255;
    return c <= 0.04 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const l = 0.2126 * lin(n >> 16) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255);
  return l > 0.3 ? 'rgba(16,16,24,0.9)' : '#fff';
}

const stripeCache = new WeakMap<CanvasRenderingContext2D, Map<string, CanvasPattern>>();

/** Diagonal stripes marking things that are in the cleanup cart. */
export function cartStripes(ctx: CanvasRenderingContext2D, color: string): CanvasPattern {
  let byColor = stripeCache.get(ctx);
  if (!byColor) stripeCache.set(ctx, (byColor = new Map()));
  const hit = byColor.get(color);
  if (hit) return hit;
  const c = new OffscreenCanvas(8, 8);
  const g = c.getContext('2d')!;
  g.strokeStyle = color;
  g.lineWidth = 2;
  g.beginPath();
  g.moveTo(-2, 10);
  g.lineTo(10, -2);
  g.moveTo(-2, 2);
  g.lineTo(2, -2);
  g.moveTo(6, 10);
  g.lineTo(10, 6);
  g.stroke();
  const p = ctx.createPattern(c, 'repeat')!;
  byColor.set(color, p);
  return p;
}
