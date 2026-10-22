<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { api, TileFlags, type Tiles } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { nodeMenu } from '$lib/menus';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import { cartStripes, categoryColor, inkOn } from '$lib/palette';
  import { bytes, percent } from '$lib/format';

  let { letter }: { letter: string } = $props();

  let wrap: HTMLDivElement;
  let base: HTMLCanvasElement;
  let overlay: HTMLCanvasElement;
  let width = $state(0);
  let height = $state(0);
  let tiles: Tiles | null = null;
  let total = $state(0);
  let hover = $state(-1);
  let mouse = $state({ x: 0, y: 0 });
  let ready = $state(false);

  type Anim = {
    snap: ImageBitmap;
    rect: { x: number; y: number; w: number; h: number };
    dir: 'in' | 'out';
  };
  let pending: Anim | null = null;
  let pendingOutFrom: number | null = null;
  let animating = $state(false);
  let loadSeq = 0;

  const focus = $derived(app.focus[letter] ?? null);

  function css(name: string) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  // Reload when the focus, size, data or sort changes. Resizes are debounced.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void focus;
    void app.rowsVersion;
    void app.sort;
    void app.resolvedTheme;
    void app.cart;
    const w = width;
    const h = height;
    if (!w || !h) return;
    untrack(() => {
      clearTimeout(timer);
      timer = setTimeout(load, pending || pendingOutFrom !== null ? 0 : 70);
    });
  });

  async function load() {
    const seq = ++loadSeq;
    const t = await api.treemap(letter, focus, width, height);
    // Stale, or the component went away while the layout was loading.
    if (seq !== loadSeq || !base) return;
    tiles = t;
    total = 0;
    for (let i = 0; i < t.count; i++) if (t.depth[i] === 1) total += t.value[i];
    hover = -1;

    const anim = pending ?? outAnim(t);
    pending = null;
    pendingOutFrom = null;
    if (anim) await animate(anim);
    else render(base, t);
    ready = true;
    drawOverlay();
  }

  /** For zooming out: where the folder we came from sits in the new layout. */
  function outAnim(t: Tiles): Anim | null {
    if (pendingOutFrom === null || !outSnap) return null;
    for (let i = 0; i < t.count; i++) {
      if (t.id[i] === pendingOutFrom) {
        const a = {
          snap: outSnap,
          rect: { x: t.x[i], y: t.y[i], w: t.w[i], h: t.h[i] },
          dir: 'out' as const,
        };
        outSnap = null;
        return a;
      }
    }
    outSnap = null;
    return null;
  }
  let outSnap: ImageBitmap | null = null;

  function setupCanvas(c: HTMLCanvasElement) {
    const dpr = window.devicePixelRatio || 1;
    const W = Math.round(width * dpr);
    const H = Math.round(height * dpr);
    if (c.width !== W || c.height !== H) {
      c.width = W;
      c.height = H;
    }
    const ctx = c.getContext('2d')!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return ctx;
  }

  function render(target: HTMLCanvasElement | OffscreenCanvas, t: Tiles) {
    const dpr = window.devicePixelRatio || 1;
    const ctx = (
      target instanceof HTMLCanvasElement ? setupCanvas(target) : target.getContext('2d')!
    ) as CanvasRenderingContext2D;
    if (!(target instanceof HTMLCanvasElement)) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const theme = app.resolvedTheme;
    const dark = theme === 'dark';
    const fg = css('--fg');
    const muted = css('--fg-muted');
    const accent = css('--accent');
    const danger = css('--danger');
    const cartRoots: number[] = [];
    ctx.textBaseline = 'middle';

    for (let i = 0; i < t.count; i++) {
      const x = t.x[i];
      const y = t.y[i];
      const w = t.w[i];
      const h = t.h[i];
      const f = t.flags[i];
      const inset = w > 3 && h > 3 ? 0.5 : 0;
      const rx = x + inset;
      const ry = y + inset;
      const rw = w - inset * 2;
      const rh = h - inset * 2;
      const r = Math.min(4, rw / 4, rh / 4);

      if (f & TileFlags.GONE) {
        // Deleted while watched: a faint placeholder until the next scan.
        ctx.fillStyle = muted;
        ctx.globalAlpha = dark ? 0.1 : 0.12;
      } else if (f & TileFlags.DIR && !(f & TileFlags.COLLAPSED)) {
        // Nested folders stack, so keep each layer faint.
        ctx.fillStyle = dark ? 'rgba(255,255,255,0.028)' : 'rgba(20,20,60,0.03)';
      } else if (f & TileFlags.DIR) {
        // Folder too small (or too busy) to open up: a quiet block of its own colour.
        ctx.fillStyle = accent;
        ctx.globalAlpha = dark ? 0.16 : 0.22;
      } else {
        ctx.fillStyle = categoryColor(t.cat[i], theme);
        ctx.globalAlpha = dark ? 0.82 : 0.88;
      }
      // Things in the cart fade back, so what stays stands out.
      if (f & TileFlags.IN_CART) ctx.globalAlpha *= 0.45;
      if (r >= 1.5) {
        ctx.beginPath();
        ctx.roundRect(rx, ry, rw, rh, r);
        ctx.fill();
      } else {
        ctx.fillRect(rx, ry, rw, rh);
      }
      // Lots of files too small to draw: hatch the folder so it doesn't look empty.
      if (f & TileFlags.MANY_SMALL) {
        ctx.fillStyle = hatch(ctx, dark);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
      // Stripe leaves only: stripes on nested folders would stack up.
      if (f & TileFlags.IN_CART && (!(f & TileFlags.DIR) || f & TileFlags.COLLAPSED)) {
        ctx.fillStyle = cartStripes(ctx, danger);
        ctx.globalAlpha = dark ? 0.32 : 0.28;
        ctx.fillRect(rx, ry, rw, rh);
        ctx.globalAlpha = 1;
      }
      if (f & TileFlags.CART_ROOT) cartRoots.push(i);
      if (f & TileFlags.GONE && rw > 3 && rh > 3) {
        ctx.strokeStyle = muted;
        ctx.globalAlpha = 0.5;
        ctx.lineWidth = 1;
        ctx.setLineDash([3, 3]);
        ctx.strokeRect(rx + 0.5, ry + 0.5, rw - 1, rh - 1);
        ctx.setLineDash([]);
        ctx.globalAlpha = 1;
      }

      const label = t.labels.get(t.id[i]);
      if (!label) continue;
      if (f & TileFlags.HEADER) {
        ctx.font = '500 11px "Geist Variable", sans-serif';
        ctx.fillStyle = dark ? muted : fg;
        const size = bytes(t.value[i]);
        const room = w - 12;
        const sizeW = size.length * 6 + 8;
        ctx.fillText(ellipsize(label, room - sizeW, 6.1), x + 6, y + 9);
        if (room > sizeW + 40) {
          ctx.fillStyle = muted;
          ctx.globalAlpha = 0.7;
          ctx.textAlign = 'right';
          ctx.fillText(size, x + w - 6, y + 9);
          ctx.textAlign = 'left';
          ctx.globalAlpha = 1;
        }
      } else if (!(f & TileFlags.DIR) || f & TileFlags.COLLAPSED) {
        ctx.font = '500 11px "Geist Variable", sans-serif';
        ctx.fillStyle = dark ? 'rgba(10,10,16,0.82)' : inkOn(categoryColor(t.cat[i], theme));
        if (f & TileFlags.DIR || f & TileFlags.IN_CART) ctx.fillStyle = fg;
        if (f & TileFlags.GONE) ctx.fillStyle = muted;
        const cy = h >= 32 ? y + h / 2 - 6 : y + h / 2;
        ctx.fillText(ellipsize(label, w - 10, 6.1), x + 5, cy);
        if (h >= 32) {
          ctx.globalAlpha = 0.75;
          ctx.font = '400 10.5px "Geist Variable", sans-serif';
          ctx.fillText(ellipsize(bytes(t.value[i]), w - 10, 5.8), x + 5, cy + 13);
          ctx.globalAlpha = 1;
        }
      }
    }

    // Outline whatever was added to the cart, on top of everything inside it.
    ctx.strokeStyle = danger;
    ctx.lineWidth = 1.5;
    ctx.setLineDash([4, 3]);
    for (const i of cartRoots) {
      const w = t.w[i] - 1.5;
      const h = t.h[i] - 1.5;
      if (w < 3 || h < 3) continue;
      ctx.beginPath();
      ctx.roundRect(t.x[i] + 0.75, t.y[i] + 0.75, w, h, Math.min(4, w / 4, h / 4));
      ctx.stroke();
    }
    ctx.setLineDash([]);
  }

  let hatchCache: { ctx: CanvasRenderingContext2D; dark: boolean; pattern: CanvasPattern } | null =
    null;
  function hatch(ctx: CanvasRenderingContext2D, dark: boolean) {
    if (hatchCache?.ctx === ctx && hatchCache.dark === dark) return hatchCache.pattern;
    const c = new OffscreenCanvas(6, 6);
    const g = c.getContext('2d')!;
    g.strokeStyle = dark ? 'rgba(255,255,255,0.07)' : 'rgba(20,20,60,0.08)';
    g.lineWidth = 1;
    g.beginPath();
    g.moveTo(0, 6);
    g.lineTo(6, 0);
    g.stroke();
    const pattern = ctx.createPattern(c, 'repeat')!;
    hatchCache = { ctx, dark, pattern };
    return pattern;
  }

  function ellipsize(s: string, room: number, charW: number) {
    const max = Math.floor(room / charW);
    if (max <= 1) return '';
    return s.length <= max ? s : s.slice(0, max - 1) + '…';
  }

  const ease = (p: number) => 1 - Math.pow(1 - p, 3);

  /** Zoom the old picture into (or out of) the clicked rect while the new one fades in. */
  function animate(a: Anim) {
    const off = new OffscreenCanvas(base.width, base.height);
    render(off, tiles!);
    const ctx = setupCanvas(base);
    const dur = 340;
    const start = performance.now();
    animating = true;
    return new Promise<void>((resolve) => {
      const frame = (now: number) => {
        const p = Math.min(1, (now - start) / dur);
        const e = ease(p);
        // Interpolated rect the old image's `rect` maps onto.
        const k = a.dir === 'in' ? e : 1 - e;
        const from = a.rect;
        const tx = from.x + (0 - from.x) * k;
        const ty = from.y + (0 - from.y) * k;
        const tw = from.w + (width - from.w) * k;
        const th = from.h + (height - from.h) * k;
        const sx = tw / from.w;
        const sy = th / from.h;
        ctx.clearRect(0, 0, width, height);
        const newAlpha = a.dir === 'in' ? Math.max(0, (e - 0.35) / 0.65) : Math.min(1, e * 1.4);
        if (a.dir === 'out') {
          ctx.globalAlpha = newAlpha;
          ctx.drawImage(off, 0, 0, width, height);
        }
        ctx.save();
        if (a.dir === 'in') {
          ctx.globalAlpha = 1 - newAlpha * 0.6;
          ctx.translate(tx - from.x * sx, ty - from.y * sy);
          ctx.scale(sx, sy);
          ctx.drawImage(a.snap, 0, 0, width, height);
        } else {
          // Old view (the folder we were in) shrinks back into its tile.
          ctx.globalAlpha = 1 - newAlpha;
          ctx.drawImage(a.snap, tx, ty, tw, th);
        }
        ctx.restore();
        if (a.dir === 'in') {
          ctx.globalAlpha = newAlpha;
          ctx.drawImage(off, 0, 0, width, height);
        }
        ctx.globalAlpha = 1;
        if (p < 1) requestAnimationFrame(frame);
        else {
          ctx.clearRect(0, 0, width, height);
          ctx.drawImage(off, 0, 0, width, height);
          a.snap.close();
          animating = false;
          resolve();
        }
      };
      requestAnimationFrame(frame);
    });
  }

  function drawOverlay() {
    if (!overlay || !width) return;
    const ctx = setupCanvas(overlay);
    ctx.clearRect(0, 0, width, height);
    if (!tiles || animating) return;
    const sel = app.selected;
    for (const [i, kind] of [
      [indexOf(sel), 'sel'],
      [hover, 'hover'],
    ] as const) {
      if (i < 0) continue;
      const x = tiles.x[i] + 0.5;
      const y = tiles.y[i] + 0.5;
      const w = tiles.w[i] - 1;
      const h = tiles.h[i] - 1;
      ctx.beginPath();
      ctx.roundRect(x, y, Math.max(1, w), Math.max(1, h), Math.min(4, w / 4, h / 4));
      if (kind === 'hover') {
        const dark = app.resolvedTheme === 'dark';
        ctx.fillStyle = dark ? 'rgba(255,255,255,0.14)' : 'rgba(255,255,255,0.3)';
        ctx.fill();
        ctx.strokeStyle = dark ? 'rgba(255,255,255,0.85)' : 'rgba(16,16,24,0.55)';
        ctx.lineWidth = 1.5;
      } else {
        ctx.strokeStyle = app.resolvedTheme === 'dark' ? '#fff' : '#17171d';
        ctx.lineWidth = 2;
        ctx.shadowColor = css('--accent');
        ctx.shadowBlur = 10;
      }
      ctx.stroke();
      ctx.shadowBlur = 0;
    }
  }

  function indexOf(id: number | null) {
    if (id === null || !tiles) return -1;
    for (let i = 0; i < tiles.count; i++) if (tiles.id[i] === id) return i;
    return -1;
  }

  $effect(() => {
    void app.selected;
    void hover;
    untrack(drawOverlay);
  });

  function hit(x: number, y: number) {
    if (!tiles) return -1;
    // Children come after parents, so the last match is the deepest tile.
    for (let i = tiles.count - 1; i >= 0; i--) {
      if (
        x >= tiles.x[i] &&
        y >= tiles.y[i] &&
        x < tiles.x[i] + tiles.w[i] &&
        y < tiles.y[i] + tiles.h[i]
      )
        return i;
    }
    return -1;
  }

  function onMove(e: MouseEvent) {
    const r = overlay.getBoundingClientRect();
    mouse = { x: e.clientX - r.left, y: e.clientY - r.top };
    if (!animating) hover = hit(mouse.x, mouse.y);
  }

  function onClick(e: MouseEvent) {
    onMove(e);
    if (hover < 0 || !tiles) return;
    app.selectNode(tiles.id[hover], true);
  }

  function onDblClick(e: MouseEvent) {
    onMove(e);
    if (hover >= 0) zoomInto(hover);
  }

  async function zoomInto(i: number) {
    if (!tiles || animating) return;
    let target = i;
    // Double-clicking a file zooms into its folder.
    if (!(tiles.flags[i] & TileFlags.DIR)) {
      const info = await api.nodeInfo(letter, tiles.id[i]);
      if (info.parent === null) return;
      target = indexOf(info.parent);
      if (target < 0) return;
    }
    const id = tiles.id[target];
    if (id === focus) return;
    pending = {
      snap: await createImageBitmap(base),
      rect: { x: tiles.x[target], y: tiles.y[target], w: tiles.w[target], h: tiles.h[target] },
      dir: 'in',
    };
    app.setFocus(letter, id);
  }

  export async function zoomOut(to?: number | null) {
    if (focus === null || animating) return;
    let parent: number | null = to === undefined ? null : to;
    if (to === undefined) {
      const info = await api.nodeInfo(letter, focus);
      parent = info.parent === 0 ? null : info.parent;
    }
    outSnap = await createImageBitmap(base);
    // Animate from the child we leave if it's a direct descendant shown in the new view.
    pendingOutFrom = focus;
    app.setFocus(letter, parent);
  }

  const tip = $derived.by(() => {
    if (hover < 0 || !tiles) return null;
    const id = tiles.id[hover];
    return {
      id,
      name: tiles.labels.get(id),
      value: tiles.value[hover],
      dir: !!(tiles.flags[hover] & TileFlags.DIR),
      cart: !!(tiles.flags[hover] & TileFlags.IN_CART),
      gone: !!(tiles.flags[hover] & TileFlags.GONE),
    };
  });

  // Path relative to the folder the chart shows, e.g. `SubFolder\Item.ext`.
  let tipPath = $state('');
  const pathCache = new Map<number, string>();
  $effect(() => {
    const t = tip;
    if (!t) return;
    const base = focus;
    const key = t.id;
    const hit = pathCache.get(key);
    if (hit !== undefined) {
      tipPath = hit;
      return;
    }
    tipPath = t.name ?? '';
    const timer = setTimeout(async () => {
      const p = await api.relPath(letter, key, base);
      if (base !== focus) return;
      pathCache.set(key, p);
      if (tip?.id === key) tipPath = p;
    }, 30);
    return () => clearTimeout(timer);
  });
  // A new view or new data can change any path.
  $effect(() => {
    void focus;
    void app.rowsVersion;
    pathCache.clear();
  });
  const tipParts = $derived.by(() => {
    const i = tipPath.lastIndexOf('\\');
    return i < 0
      ? { dir: '', name: tipPath }
      : { dir: tipPath.slice(0, i + 1), name: tipPath.slice(i + 1) };
  });

  onMount(() => {
    const mq = matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    const redraw = () => tiles && render(base, tiles);
    mq.addEventListener('change', redraw);
    return () => mq.removeEventListener('change', redraw);
  });
</script>

<div
  class="relative h-full w-full overflow-hidden"
  bind:this={wrap}
  bind:clientWidth={width}
  bind:clientHeight={height}
>
  <canvas bind:this={base} class="absolute inset-0 h-full w-full" class:show={ready}></canvas>
  <canvas
    bind:this={overlay}
    class="absolute inset-0 h-full w-full"
    onpointermove={onMove}
    onpointerleave={() => (hover = -1)}
    onclick={onClick}
    ondblclick={onDblClick}
    oncontextmenu={(e) => {
      e.preventDefault();
      onMove(e);
      if (hover < 0 || !tiles) return zoomOut();
      if (tiles.flags[hover] & TileFlags.GONE) return;
      const id = tiles.id[hover];
      app.selectNode(id, true);
      nodeMenu(
        e,
        letter,
        id,
        focus === null
          ? []
          : [{ label: 'Zoom out', icon: ArrowUp, hint: 'Backspace', run: () => zoomOut() }],
      );
    }}
  ></canvas>

  {#if tip && !animating}
    {@const right = mouse.x > width - 240}
    {@const below = mouse.y < 70}
    <div
      class="tip pointer-events-none absolute z-10 max-w-[260px] rounded-lg border border-line px-2.5 py-1.5"
      style="left: {right ? mouse.x - 14 : mouse.x + 14}px; top: {below
        ? mouse.y + 18
        : mouse.y - 12}px; transform: translate({right ? '-100%' : '0'}, {below ? '0' : '-100%'})"
      in:fade={{ duration: 90 }}
    >
      <div class="font-medium break-all">
        {#if tipParts.dir}<span class="font-normal text-fg-muted">{tipParts.dir}</span
          >{/if}{tipParts.name || '…'}
      </div>
      <div class="tabular text-[11.5px] text-fg-muted">
        {bytes(tip.value)}
        {#if total > 0}<span class="text-fg-subtle"> · {percent(tip.value / total)}</span>{/if}
      </div>
      {#if tip.gone}
        <div class="mt-0.5 text-[11.5px] text-danger">Deleted</div>
      {:else if tip.cart}
        <div class="mt-0.5 flex items-center gap-1 text-[11.5px] text-danger">
          <Trash2 class="size-3" /> In cleanup cart
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  canvas:first-child {
    opacity: 0;
    transition: opacity 260ms ease;
  }
  canvas.show {
    opacity: 1;
  }
  .tip {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(12px);
  }
</style>
