/** Chart categories the backend assigns (see layout.rs), as file kinds. */
const categoryKinds = [
  'folder',
  'other',
  'video',
  'image',
  'audio',
  'archive',
  'code',
  'document',
  'program',
  'diskimage',
  'database',
] as const;

/** Built-in colours, used for anything the colour template leaves out. */
const builtIn: Record<string, string> = {
  folder: '#8b7cff',
  other: '#7d8496',
  none: '#5c6273',
  video: '#ff6b8b',
  image: '#f5b544',
  audio: '#c38bff',
  document: '#5fa8ff',
  spreadsheet: '#4fd18b',
  presentation: '#ff8f6b',
  archive: '#ff9a5a',
  program: '#9d92ff',
  system: '#8a93b8',
  code: '#3fd0c9',
  diskimage: '#f37bc4',
  database: '#8fb3a8',
  design: '#e8d26a',
  font: '#b0a4ff',
  email: '#6fc3ff',
  log: '#a3a9b8',
};
const builtInBranches = [
  '#8b7cff',
  '#3fd0c9',
  '#f5b544',
  '#ff6b8b',
  '#5fa8ff',
  '#ff9a5a',
  '#7ad67a',
  '#c38bff',
];

let files: Record<string, string> = builtIn;
let branchList: string[] = builtInBranches;

/** Called by the colour template for the current light/dark mode. */
export function setFileColors(f: Record<string, string> | undefined, b: string[] | undefined) {
  files = { ...builtIn, ...f };
  branchList = b?.length ? b : builtInBranches;
}

/** Colour of a file kind (see kinds.rs), for the Types tab. */
export function kindColor(kind: string): string {
  return files[kind] ?? files.other;
}

/** Colour of a chart tile's category. */
export function categoryColor(cat: number): string {
  return kindColor(categoryKinds[cat] ?? 'other');
}

/** Sunburst: one colour per top-level folder, repeating. */
export function branchColor(i: number): string {
  return branchList[i % branchList.length];
}

/** Readable label colour on top of a tile filled with `hex` (light theme). */
export function inkOn(hex: string): string {
  let h = hex.trim();
  if (/^#[0-9a-f]{3}$/i.test(h)) h = '#' + [...h.slice(1)].map((c) => c + c).join('');
  if (!/^#[0-9a-f]{6}$/i.test(h)) return 'rgba(16,16,24,0.9)';
  const n = parseInt(h.slice(1), 16);
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
