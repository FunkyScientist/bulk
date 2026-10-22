<script lang="ts">
  import { untrack } from 'svelte';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Folder from '@lucide/svelte/icons/folder';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import File from '@lucide/svelte/icons/file';
  import Link from '@lucide/svelte/icons/link-2';
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import { api, Flags, type Row, type SortKey } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { nodeMenu } from '$lib/menus';
  import { ago, bytes, count, percent } from '$lib/format';

  let { letter }: { letter: string } = $props();

  const ROW = 28;
  const PAGE = 100;

  let viewport: HTMLDivElement;
  let scrollTop = $state(0);
  let height = $state(600);
  let total = $state(0);
  let pages = $state<Record<number, Row[]>>({});
  const loading = new Set<number>();
  let generation = 0;

  /** Rows that just appeared under an expanded folder animate in. */
  let entering = $state<{ from: number; to: number; at: number } | null>(null);

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 8));
  const last = $derived(Math.min(total, Math.ceil((scrollTop + height) / ROW) + 8));

  /** Reload the visible pages and swap them in together, so nothing flashes. */
  async function refetch() {
    const gen = ++generation;
    loading.clear();
    const a = Math.floor(first / PAGE);
    const b = Math.max(a, Math.floor(Math.max(first, last - 1) / PAGE));
    const res = await Promise.all(
      Array.from({ length: b - a + 1 }, (_, k) => api.rows(letter, (a + k) * PAGE, PAGE)),
    );
    if (gen !== generation) return;
    const next: Record<number, Row[]> = {};
    res.forEach((r, k) => (next[a + k] = r.rows));
    total = res[0].total;
    pages = next;
  }

  async function load(page: number) {
    if (loading.has(page)) return;
    loading.add(page);
    const gen = generation;
    try {
      const res = await api.rows(letter, page * PAGE, PAGE);
      if (gen !== generation) return;
      total = res.total;
      pages[page] = res.rows;
    } finally {
      loading.delete(page);
    }
  }

  // Refetch whenever the backend view changes.
  $effect(() => {
    void app.rowsVersion;
    void letter;
    untrack(refetch);
  });

  // Fetch any pages the visible window needs.
  $effect(() => {
    const a = Math.floor(first / PAGE);
    const b = Math.floor(Math.max(first, last - 1) / PAGE);
    untrack(() => {
      for (let p = a; p <= b; p++) if (!pages[p]) void load(p);
    });
  });

  function rowAt(i: number): Row | undefined {
    return pages[Math.floor(i / PAGE)]?.[i % PAGE];
  }

  const visible = $derived.by(() => {
    const out: { i: number; row: Row | undefined }[] = [];
    for (let i = first; i < last; i++) out.push({ i, row: rowAt(i) });
    return out;
  });

  async function toggle(row: Row, index: number) {
    if (!row.hasKids) return;
    const before = total;
    const n = await api.toggle(letter, row.id);
    if (!row.open && n > before)
      entering = { from: index + 1, to: index + 1 + (n - before), at: Date.now() };
    await refetch();
  }

  function select(row: Row) {
    app.selectNode(row.id);
    viewport.focus({ preventScroll: true });
  }

  /** Row at `i`, loading its page if needed. */
  async function fetchRow(i: number): Promise<Row | undefined> {
    if (i < 0 || i >= total) return undefined;
    const p = Math.floor(i / PAGE);
    if (!pages[p]) await load(p);
    return rowAt(i);
  }

  function ensureVisible(i: number) {
    const top = i * ROW;
    if (top < viewport.scrollTop) viewport.scrollTo({ top });
    else if (top + ROW > viewport.scrollTop + height)
      viewport.scrollTo({ top: top + ROW - height });
  }

  async function selectIndex(i: number) {
    const idx = Math.max(0, Math.min(total - 1, i));
    const row = await fetchRow(idx);
    if (!row) return;
    app.selectNode(row.id);
    ensureVisible(idx);
  }

  function selectedIndex() {
    for (const [p, rows] of Object.entries(pages)) {
      const k = rows.findIndex((r) => r.id === app.selected);
      if (k >= 0) return Number(p) * PAGE + k;
    }
    return -1;
  }

  async function onKey(e: KeyboardEvent) {
    const page = Math.max(1, Math.floor(height / ROW) - 1);
    const cur = selectedIndex();
    const row = cur >= 0 ? rowAt(cur) : undefined;
    const handled = [
      'ArrowDown',
      'ArrowUp',
      'ArrowLeft',
      'ArrowRight',
      'Home',
      'End',
      'PageDown',
      'PageUp',
      'Enter',
    ];
    if (!handled.includes(e.key)) return;
    e.preventDefault();
    if (cur < 0 || !row) return selectIndex(0);
    switch (e.key) {
      case 'ArrowDown':
        return selectIndex(cur + 1);
      case 'ArrowUp':
        return selectIndex(cur - 1);
      case 'PageDown':
        return selectIndex(cur + page);
      case 'PageUp':
        return selectIndex(cur - page);
      case 'Home':
        return selectIndex(0);
      case 'End':
        return selectIndex(total - 1);
      case 'ArrowRight':
        if (row.hasKids && !row.open) return toggle(row, cur);
        if (row.open) return selectIndex(cur + 1);
        return;
      case 'ArrowLeft':
        if (row.open && row.depth > 0) return toggle(row, cur);
        // Walk back to the parent row (the nearest row above with smaller depth).
        for (let i = cur - 1; i >= 0; i--) {
          const r = await fetchRow(i);
          if (r && r.depth < row.depth) return selectIndex(i);
        }
        return;
      case 'Enter':
        if (e.shiftKey) return api.showInExplorer(await api.nodePath(letter, row.id));
        if (row.dir) app.setFocus(letter, row.depth === 0 ? null : row.id);
        return;
    }
  }

  async function open(row: Row, index: number) {
    if (row.dir) return toggle(row, index);
    await api.showInExplorer(await api.nodePath(letter, row.id));
  }

  function enterDelay(i: number) {
    if (!entering || i < entering.from || i >= entering.to) return null;
    if (Date.now() - entering.at > 600) return null;
    return Math.min(i - entering.from, 14) * 14;
  }

  /** Optional columns drop out as the list gets narrower. */
  const allColumns: { key: SortKey; label: string; width: number; minWidth: number }[] = [
    { key: 'alloc', label: 'On disk', width: 84, minWidth: 0 },
    { key: 'size', label: 'Size', width: 80, minWidth: 640 },
    { key: 'files', label: 'Files', width: 80, minWidth: 740 },
    { key: 'modified', label: 'Modified', width: 104, minWidth: 860 },
  ];
  let listWidth = $state(800);
  const columns = $derived(allColumns.filter((c) => listWidth >= c.minWidth));
  const showShare = $derived(listWidth >= 460);

  function cell(row: Row, key: SortKey) {
    switch (key) {
      case 'alloc':
        return bytes(row.alloc);
      case 'size':
        return bytes(row.size);
      case 'files':
        return row.dir ? count(row.files) : '';
      case 'modified':
        return ago(row.mtime);
      default:
        return '';
    }
  }

  // Scroll to a node picked in the visualisation.
  $effect(() => {
    const req = app.revealRequest;
    if (!req) return;
    untrack(async () => {
      const index = await api.reveal(letter, req.id);
      if (index === null) return;
      await refetch();
      const top = index * ROW - height / 2 + ROW / 2;
      const visibleNow = index * ROW >= scrollTop && (index + 1) * ROW <= scrollTop + height;
      if (!visibleNow) viewport.scrollTo({ top: Math.max(0, top), behavior: 'smooth' });
      viewport.focus({ preventScroll: true });
    });
  });

  function iconColor(row: Row) {
    if (row.flags & (Flags.EXTRA_LINK | Flags.REPARSE)) return 'text-fg-subtle';
    return row.dir ? 'text-accent' : 'text-fg-subtle';
  }
</script>

<div class="flex h-full min-w-0 flex-col" bind:clientWidth={listWidth}>
  <div
    class="header flex h-8 shrink-0 items-center border-b border-line pr-[18px] pl-1.5 text-[11.5px] text-fg-subtle"
  >
    <button
      class="hcell flex min-w-0 flex-1 items-center gap-1 pl-4 text-left"
      class:active={app.sort === 'name'}
      onclick={() => app.setSort('name')}
    >
      Name {#if app.sort === 'name'}<ArrowDown class="size-3 rotate-180" />{/if}
    </button>
    {#if showShare}<div class="w-[96px] shrink-0 pl-2">Share</div>{/if}
    {#each columns as c (c.key)}
      <button
        class="hcell flex shrink-0 items-center justify-end gap-1"
        style="width: {c.width}px"
        class:active={app.sort === c.key}
        onclick={() => app.setSort(c.key)}
      >
        {#if app.sort === c.key}<ArrowDown class="size-3" />{/if}
        {c.label}
      </button>
    {/each}
  </div>

  <div
    class="min-h-0 flex-1 overflow-y-auto"
    bind:this={viewport}
    bind:clientHeight={height}
    onscroll={() => (scrollTop = viewport.scrollTop)}
    onkeydown={onKey}
    role="tree"
    tabindex="0"
  >
    <div class="relative" style="height: {total * ROW}px">
      {#each visible as { i, row } (row ? `${row.id}` : `p${i}`)}
        {@const delay = enterDelay(i)}
        <div
          class="row absolute inset-x-0 flex items-center pr-3"
          class:selected={row && app.selected === row.id}
          class:enter={delay !== null}
          style="transform: translateY({i * ROW}px); height: {ROW}px; {delay !== null
            ? `animation-delay: ${delay}ms`
            : ''}"
          role="treeitem"
          aria-selected={row ? app.selected === row.id : false}
          aria-expanded={row?.dir ? row.open : undefined}
          tabindex="-1"
          onclick={() => row && select(row)}
          oncontextmenu={(e) => {
            if (!row) return;
            select(row);
            nodeMenu(e, letter, row.id);
          }}
          ondblclick={() => row && open(row, i)}
          onkeydown={() => {}}
          onfocus={() => viewport.focus({ preventScroll: true })}
        >
          {#if row}
            <div
              class="flex min-w-0 flex-1 items-center gap-1.5"
              style="padding-left: {8 + row.depth * 16}px"
            >
              <button
                class="chev grid size-5 shrink-0 place-items-center rounded text-fg-subtle hover:bg-hover hover:text-fg"
                class:invisible={!row.hasKids}
                class:open={row.open}
                tabindex="-1"
                aria-label={row.open ? 'Collapse' : 'Expand'}
                onclick={(e) => {
                  e.stopPropagation();
                  toggle(row, i);
                }}
              >
                <ChevronRight class="size-3.5" />
              </button>
              <span class="shrink-0 {iconColor(row)}">
                {#if row.flags & (Flags.EXTRA_LINK | Flags.REPARSE)}<Link
                    class="size-3.5"
                  />{:else if row.dir}{#if row.open}<FolderOpen class="size-3.5" />{:else}<Folder
                      class="size-3.5"
                    />{/if}{:else}<File class="size-3.5" />{/if}
              </span>
              <span class="truncate" class:text-fg-muted={row.flags & (Flags.HIDDEN | Flags.META)}>
                {row.depth === 0 ? `${row.name}\\` : row.name}
              </span>
            </div>
            {#if showShare}
              <div class="flex w-[96px] shrink-0 items-center gap-2 pl-2">
                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-panel-2">
                  <div class="bar h-full rounded-full" style="width: {row.share * 100}%"></div>
                </div>
                <span class="tabular w-9 text-right text-[11px] text-fg-subtle"
                  >{percent(row.share)}</span
                >
              </div>
            {/if}
            {#each columns as c (c.key)}
              <div
                class="tabular shrink-0 truncate text-right {c.key === 'alloc'
                  ? ''
                  : c.key === 'modified'
                    ? 'text-fg-subtle'
                    : 'text-fg-muted'}"
                style="width: {c.width}px"
              >
                {cell(row, c.key)}
              </div>
            {/each}
          {:else}
            <div
              class="skeleton ml-4 h-2.5 rounded"
              style="width: {140 + ((i * 37) % 160)}px"
            ></div>
          {/if}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  /* The selected row already shows where the keyboard is. */
  [role='tree']:focus-visible {
    outline: none;
  }
  .hcell {
    height: 100%;
    transition: color 120ms ease;
  }
  .hcell:hover,
  .hcell.active {
    color: var(--fg);
  }
  .row {
    top: 0;
    border-radius: 8px;
    margin: 0 6px;
    /* Rows glide to their new place when folders open or the sort changes. */
    transition:
      transform 220ms var(--ease-out-soft),
      background-color 120ms ease;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--accent-soft);
  }
  .row.enter {
    animation: row-in 260ms var(--ease-out-soft) both;
  }
  @keyframes row-in {
    from {
      opacity: 0;
    }
  }
  .chev :global(svg) {
    transition: transform 180ms var(--ease-out-soft);
  }
  .chev.open :global(svg) {
    transform: rotate(90deg);
  }
  .bar {
    background: linear-gradient(
      90deg,
      var(--accent),
      color-mix(in oklab, var(--accent) 70%, white)
    );
    transition: width 400ms var(--ease-out-soft);
  }
  .skeleton {
    background: linear-gradient(90deg, var(--panel-2), var(--hover), var(--panel-2));
    background-size: 200% 100%;
    animation: shimmer 1.2s linear infinite;
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }
</style>
