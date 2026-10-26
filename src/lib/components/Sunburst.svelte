<script lang="ts">
  import { untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { api, sameTiles, TileFlags, type NodeInfo, type Tiles } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { nodeMenu } from '$lib/menus';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import { branchColor, cartStripes, categoryColor, inkOn } from '$lib/palette';
  import { bytes, percent } from '$lib/format';

  let { letter }: { letter: string } = $props();

  const RINGS = 6;
  const TAU = Math.PI * 2;

  let canvas: HTMLCanvasElement;
  let width = $state(0);
  let height = $state(0);
  let arcs: Tiles | null = null;
  let total = $state(0);
  let hover = $state(-1);
  let hoverCenter = $state(false);
  let mouse = $state({ x: 0, y: 0 });
  let center = $state<NodeInfo | null>(null);
  // What the canvas last showed, so live updates elsewhere on the drive don't redraw it.
  let drawnTheme = '';
  let inside = false;
  let ready = $state(false);
  let animating = $state(false);
  let loadSeq = 0;

  type Zoom = { a0: number; a1: number; depth: number; dir: 'in' | 'out' };
  let pendingIn: Zoom | null = null;
  let pendingOutFrom: number | null = null;

  const focus = $derived(app.focus[letter] ?? null);
  const geo = $derived.by(() => {
    const R = Math.max(40, Math.min(width, height) / 2 - 12);
    const hole = R * 0.26;
    return { cx: width / 2, cy: height / 2, R, hole, ring: (R - hole) / RINGS };
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void focus;
    void app.rowsVersion;
    void app.sort;
    void app.themeKey;
    void app.cart;
    if (!width || !height) return;
    untrack(() => {
      clearTimeout(timer);
      timer = setTimeout(load, pendingIn || pendingOutFrom !== null ? 0 : 60);
    });
  });

  async function load() {
    const seq = ++loadSeq;
    const [t, info] = await Promise.all([
      api.sunburst(letter, focus, RINGS),
      api.nodeInfo(letter, focus ?? 0),
    ]);
    // Stale, or the component went away while the layout was loading.
    if (seq !== loadSeq || !canvas) return;
    // Live updates elsewhere on the drive: nothing here changed, so leave it be.
    const zooming = pendingIn !== null || pendingOutFrom !== null;
    if (
      !zooming &&
      arcs &&
      center &&
      drawnTheme === app.themeKey &&
      sameTiles(arcs, t) &&
      center.alloc === info.alloc &&
      center.size === info.size
    )
      return;
    drawnTheme = app.themeKey;
    arcs = t;
    branch = branches(t);
    center = info;
    total = info.dir ? (app.sort === 'size' ? info.size : info.alloc) : 0;
    // Keep the tooltip on whatever is under the pointer now.
    if (zooming || !inside) hover = -1;
    else {
      const h = hit(mouse.x, mouse.y);
      hover = h.i;
      hoverCenter = h.center;
    }

    let zoom: Zoom | null = pendingIn;
    if (!zoom && pendingOutFrom !== null) {
      const i = indexOf(pendingOutFrom);
      if (i >= 0) zoom = { a0: t.x[i], a1: t.y[i], depth: t.depth[i], dir: 'out' };
    }
    pendingIn = null;
    pendingOutFrom = null;
    if (zoom) await animate(zoom);
    else draw();
    pendingOutFromAnim = null;
    ready = true;
  }

  let branch = new Uint8Array(0);

  function branches(t: Tiles) {
    const top: number[] = [];
    for (let i = 0; i < t.count; i++) if (t.depth[i] === 1) top.push(i);
    top.sort((a, b) => t.x[a] - t.x[b]);
    const out = new Uint8Array(t.count);
    for (let i = 0; i < t.count; i++) {
      const mid = (t.x[i] + t.y[i]) / 2;
      // Binary search for the top-level arc whose span contains this one.
      let lo = 0;
      let hi = top.length - 1;
      while (lo < hi) {
        const m = (lo + hi + 1) >> 1;
        if (t.x[top[m]] <= mid) lo = m;
        else hi = m - 1;
      }
      out[i] = lo;
    }
    return out;
  }

  function indexOf(id: number | null) {
    if (id === null || !arcs) return -1;
    for (let i = 0; i < arcs.count; i++) if (arcs.id[i] === id) return i;
    return -1;
  }

  const ease = (p: number) => 1 - Math.pow(1 - p, 3);

  function animate(z: Zoom) {
    animating = true;
    const start = performance.now();
    return new Promise<void>((resolve) => {
      const frame = (now: number) => {
        const p = Math.min(1, (now - start) / 420);
        draw(z, ease(p));
        if (p < 1) requestAnimationFrame(frame);
        else {
          animating = false;
          draw();
          resolve();
        }
      };
      requestAnimationFrame(frame);
    });
  }

  function css(name: string) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  function draw(z?: Zoom, p = 1) {
    if (!canvas || !arcs) return;
    const dpr = window.devicePixelRatio || 1;
    const W = Math.round(width * dpr);
    const H = Math.round(height * dpr);
    if (canvas.width !== W || canvas.height !== H) {
      canvas.width = W;
      canvas.height = H;
    }
    const ctx = canvas.getContext('2d')!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    const { cx, cy, R, hole, ring } = geo;
    const theme = app.resolvedTheme;
    const dark = theme === 'dark';
    const accent = css('--accent');
    const cart = css('--cart');
    const labelInk = css('--chart-label');
    const labelInkFolder = css('--chart-label-folder');
    const t = arcs;
    const sel = app.selected;

    const span = z ? (z.a1 - z.a0) / TAU : 1;
    for (let i = 0; i < t.count; i++) {
      let a0 = t.x[i];
      let a1 = t.y[i];
      let d = t.depth[i];
      let alpha = 1;
      if (z) {
        if (z.dir === 'in') {
          // New arcs start squeezed into the arc that was clicked, then open out.
          a0 = lerp(z.a0 + a0 * span, a0, p);
          a1 = lerp(z.a0 + a1 * span, a1, p);
          d = d + z.depth * (1 - p);
          alpha = 0.35 + 0.65 * p;
        } else {
          // Zooming out: the folder we left starts as the whole circle and shrinks back.
          const inside = a0 >= z.a0 - 1e-6 && a1 <= z.a1 + 1e-6 && t.depth[i] > z.depth;
          a0 = lerp((a0 - z.a0) / span, a0, p);
          a1 = lerp((a1 - z.a0) / span, a1, p);
          d = d - z.depth * (1 - p);
          alpha = inside || t.id[i] === pendingOutFromAnim ? 1 : p;
        }
      }
      const r0 = hole + (d - 1) * ring + 1;
      const r1 = hole + d * ring;
      if (r1 <= hole || r0 >= R + ring) continue;
      const pad = Math.min(0.006, (a1 - a0) * 0.12);
      const s = a0 + pad - Math.PI / 2;
      const e = a1 - pad - Math.PI / 2;
      if (e <= s) continue;

      const f = t.flags[i];
      if (f & TileFlags.GONE) {
        ctx.fillStyle = css('--fg-subtle');
        ctx.globalAlpha = alpha * 0.18;
      } else if (f & TileFlags.DIR) {
        ctx.fillStyle = branchColor(branch[i]) ?? accent;
        ctx.globalAlpha =
          alpha * (f & TileFlags.COLLAPSED ? 0.3 : Math.max(0.35, 0.9 - (t.depth[i] - 1) * 0.11));
      } else {
        ctx.fillStyle = categoryColor(t.cat[i]);
        ctx.globalAlpha = alpha * (dark ? 0.82 : 0.88);
      }
      if (f & TileFlags.IN_CART) ctx.globalAlpha *= 0.45;
      ctx.beginPath();
      ctx.arc(cx, cy, Math.max(hole, r1), s, e);
      ctx.arc(cx, cy, Math.max(hole, r0), e, s, true);
      ctx.closePath();
      ctx.fill();
      if (f & TileFlags.IN_CART) {
        ctx.fillStyle = cartStripes(ctx, cart);
        ctx.globalAlpha = alpha * (dark ? 0.32 : 0.28);
        ctx.fill();
      }
      if (f & TileFlags.NEW) {
        ctx.globalAlpha = alpha;
        ctx.strokeStyle = css('--added');
        ctx.lineWidth = 1.5;
        ctx.stroke();
      }
      if (f & TileFlags.CART_ROOT) {
        ctx.globalAlpha = alpha;
        ctx.strokeStyle = cart;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([4, 3]);
        ctx.stroke();
        ctx.setLineDash([]);
      }

      if (!z && (i === hover || t.id[i] === sel)) {
        ctx.globalAlpha = 1;
        ctx.lineWidth = i === hover ? 1.5 : 2;
        ctx.strokeStyle = css('--chart-select');
        if (t.id[i] === sel) {
          ctx.shadowColor = accent;
          ctx.shadowBlur = 10;
        }
        ctx.stroke();
        ctx.shadowBlur = 0;
        if (i === hover) {
          ctx.fillStyle = css('--chart-hover');
          ctx.fill();
        }
      }
    }
    ctx.globalAlpha = 1;

    // Labels on arcs with room for them.
    if (!z) {
      ctx.font = '500 11px "Geist Variable", sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      for (let i = 0; i < t.count; i++) {
        const label = t.labels.get(t.id[i]);
        if (!label) continue;
        const d = t.depth[i];
        const rm = hole + (d - 0.5) * ring;
        const len = (t.y[i] - t.x[i]) * rm;
        if (len < 56 || ring < 16) continue;
        const mid = (t.x[i] + t.y[i]) / 2;
        const text = ellipsize(label, len * 0.82, 6.1);
        if (!text) continue;
        const f = t.flags[i];
        ctx.fillStyle =
          f & TileFlags.DIR || (!dark && f & TileFlags.IN_CART)
            ? labelInkFolder
            : labelInk === 'auto'
              ? inkOn(categoryColor(t.cat[i]))
              : labelInk;
        // Run the text along the arc, flipped on the lower half so it's never upside down.
        const am = mid - Math.PI / 2;
        const flip = mid > Math.PI / 2 && mid < (Math.PI * 3) / 2;
        ctx.save();
        ctx.translate(cx + Math.cos(am) * rm, cy + Math.sin(am) * rm);
        ctx.rotate(flip ? am - Math.PI / 2 : am + Math.PI / 2);
        ctx.fillText(text, 0, 0);
        ctx.restore();
      }
      ctx.textAlign = 'left';
    }
  }
  let pendingOutFromAnim: number | null = null;

  function ellipsize(s: string, room: number, charW: number) {
    const max = Math.floor(room / charW);
    if (max <= 2) return '';
    return s.length <= max ? s : s.slice(0, max - 1) + '…';
  }

  function lerp(a: number, b: number, p: number) {
    return a + (b - a) * p;
  }

  $effect(() => {
    void app.selected;
    void hover;
    untrack(() => !animating && draw());
  });

  function hit(x: number, y: number) {
    if (!arcs) return { i: -1, center: false };
    const { cx, cy, hole, ring } = geo;
    const dx = x - cx;
    const dy = y - cy;
    const dist = Math.hypot(dx, dy);
    if (dist < hole) return { i: -1, center: true };
    const d = Math.floor((dist - hole) / ring) + 1;
    let a = Math.atan2(dx, -dy);
    if (a < 0) a += TAU;
    for (let i = 0; i < arcs.count; i++) {
      if (arcs.depth[i] === d && a >= arcs.x[i] && a < arcs.y[i]) return { i, center: false };
    }
    return { i: -1, center: false };
  }

  function onMove(e: PointerEvent | MouseEvent) {
    const r = canvas.getBoundingClientRect();
    mouse = { x: e.clientX - r.left, y: e.clientY - r.top };
    if (animating) return;
    const h = hit(mouse.x, mouse.y);
    hover = h.i;
    hoverCenter = h.center;
  }

  function onClick(e: MouseEvent) {
    onMove(e);
    if (hoverCenter) return zoomOut();
    if (hover >= 0 && arcs) app.selectNode(arcs.id[hover], true);
  }

  function onDblClick(e: MouseEvent) {
    onMove(e);
    if (hover < 0 || !arcs || animating) return;
    const i = hover;
    if (!(arcs.flags[i] & TileFlags.DIR)) return;
    pendingIn = { a0: arcs.x[i], a1: arcs.y[i], depth: arcs.depth[i], dir: 'in' };
    app.setFocus(letter, arcs.id[i]);
  }

  export async function zoomOut(to?: number | null) {
    if (focus === null || animating) return;
    let parent: number | null;
    if (to === undefined) {
      const info = await api.nodeInfo(letter, focus);
      parent = info.parent === 0 ? null : info.parent;
    } else parent = to;
    pendingOutFrom = focus;
    pendingOutFromAnim = focus;
    app.setFocus(letter, parent);
  }

  const tip = $derived.by(() => {
    if (hover < 0 || !arcs || animating) return null;
    const id = arcs.id[hover];
    return {
      id,
      name: arcs.labels.get(id),
      value: arcs.value[hover],
      cart: !!(arcs.flags[hover] & TileFlags.IN_CART),
      gone: !!(arcs.flags[hover] & TileFlags.GONE),
      added: !!(arcs.flags[hover] & TileFlags.NEW),
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
</script>

<div
  class="relative h-full w-full overflow-hidden"
  bind:clientWidth={width}
  bind:clientHeight={height}
>
  <canvas
    bind:this={canvas}
    class="absolute inset-0 h-full w-full"
    class:show={ready}
    class:pointer={hover >= 0 || (hoverCenter && focus !== null)}
    onpointermove={(e) => {
      inside = true;
      onMove(e);
    }}
    onpointerleave={() => {
      inside = false;
      hover = -1;
      hoverCenter = false;
    }}
    onclick={onClick}
    ondblclick={onDblClick}
    oncontextmenu={(e) => {
      e.preventDefault();
      onMove(e);
      if (hover < 0 || !arcs) return zoomOut();
      if (arcs.flags[hover] & TileFlags.GONE) return;
      const id = arcs.id[hover];
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

  {#if center && ready}
    {@const size = Math.min(geo.hole * 1.7, 180)}
    <div
      class="pointer-events-none absolute flex flex-col items-center justify-center text-center"
      style="left: {geo.cx - size / 2}px; top: {geo.cy -
        size / 2}px; width: {size}px; height: {size}px"
    >
      {#key center.id}
        <div class="flex flex-col items-center" in:fade={{ duration: 220 }}>
          <div class="max-w-full truncate px-2 text-[12px] text-fg-muted">
            {focus === null ? `${center.name}\\` : center.name}
          </div>
          <div class="tabular text-[17px] font-semibold tracking-tight">
            {bytes(app.sort === 'size' ? center.size : center.alloc)}
          </div>
          {#if focus !== null}
            <div
              class="mt-0.5 text-[10.5px] text-fg-subtle transition-opacity"
              class:opacity-0={!hoverCenter}
            >
              Click to go up
            </div>
          {/if}
        </div>
      {/key}
    </div>
  {/if}

  {#if tip}
    {@const right = mouse.x > width - 240}
    <div
      class="tip pointer-events-none absolute z-10 max-w-[260px] rounded-lg border border-line px-2.5 py-1.5"
      style="left: {right ? mouse.x - 14 : mouse.x + 14}px; top: {mouse.y -
        12}px; transform: translate({right ? '-100%' : '0'}, -100%)"
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
        <div class="mt-0.5 text-[11.5px] text-removed">Deleted</div>
      {:else if tip.cart}
        <div class="mt-0.5 flex items-center gap-1 text-[11.5px] text-cart">
          <Trash2 class="size-3" /> In cleanup cart
        </div>
      {:else if tip.added}
        <div class="mt-0.5 text-[11.5px] text-added">New since the last scan</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  canvas {
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
