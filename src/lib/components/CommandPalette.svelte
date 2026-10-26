<script lang="ts">
  import { tick } from 'svelte';
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import type { Component } from 'svelte';
  import Search from '@lucide/svelte/icons/search';
  import Folder from '@lucide/svelte/icons/folder';
  import File from '@lucide/svelte/icons/file';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import LayoutGrid from '@lucide/svelte/icons/layout-grid';
  import PieChart from '@lucide/svelte/icons/chart-pie';
  import History from '@lucide/svelte/icons/history';
  import FolderTree from '@lucide/svelte/icons/folder-tree';
  import PanelLeft from '@lucide/svelte/icons/panel-left';
  import Sun from '@lucide/svelte/icons/sun';
  import Moon from '@lucide/svelte/icons/moon';
  import Monitor from '@lucide/svelte/icons/monitor';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Zap from '@lucide/svelte/icons/zap';
  import Keyboard from '@lucide/svelte/icons/keyboard';
  import SettingsIcon from '@lucide/svelte/icons/settings';
  import Copy from '@lucide/svelte/icons/copy';
  import ShoppingCart from '@lucide/svelte/icons/shopping-cart';
  import CornerDownLeft from '@lucide/svelte/icons/corner-down-left';
  import CircleHelp from '@lucide/svelte/icons/circle-help';
  import Check from '@lucide/svelte/icons/check';
  import Minus from '@lucide/svelte/icons/minus';
  import Globe from '@lucide/svelte/icons/globe';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import { api, type SearchHit, type SearchScope } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { ago, bytes, count } from '$lib/format';

  let {
    open = $bindable(false),
    onShortcuts,
    onSettings,
    onRescanDrives,
  }: {
    open: boolean;
    onShortcuts: () => void;
    onSettings: () => void;
    onRescanDrives: () => void;
  } = $props();

  type Action = { kind: 'action'; label: string; hint?: string; icon: Component; run: () => void };
  type Item = Action | { kind: 'hit'; hit: SearchHit };

  let input: HTMLInputElement | undefined = $state();
  let list: HTMLDivElement | undefined = $state();
  let text = $state('');
  let hits = $state<SearchHit[]>([]);
  let stats = $state<{ total: number; ms: number } | null>(null);
  let active = $state(0);
  let seq = 0;
  let help = $state(false);
  /** Selected hits by `letter:id`. */
  let picked = $state<Record<string, SearchHit>>({});
  /** Every match, when "select all" covers more than the hits shown. */
  let pickedAll = $state<{ letter: string; ids: number[] }[] | null>(null);
  let folderName = $state('');

  const ROW = 38;

  $effect(() => {
    if (open) {
      text = '';
      hits = [];
      stats = null;
      active = 0;
      help = false;
      picked = {};
      pickedAll = null;
      tick().then(() => input?.focus());
    }
  });

  const actions = $derived.by((): Action[] => {
    const d = app.drive;
    const out: Action[] = [];
    if (d)
      out.push({
        kind: 'action',
        label: `Rescan ${d.letter}:`,
        hint: 'Ctrl R',
        icon: RefreshCw,
        run: () => app.scan(d.letter),
      });
    out.push(
      {
        kind: 'action',
        label: 'Show treemap',
        hint: 'Ctrl 1',
        icon: LayoutGrid,
        run: () => app.setVisual('treemap'),
      },
      {
        kind: 'action',
        label: 'Show sunburst',
        hint: 'Ctrl 2',
        icon: PieChart,
        run: () => app.setVisual('sunburst'),
      },
      { kind: 'action', label: 'Browse files', icon: FolderTree, run: () => (app.tab = 'browse') },
      { kind: 'action', label: 'What changed', icon: History, run: () => (app.tab = 'changes') },
      { kind: 'action', label: 'Find duplicates', icon: Copy, run: () => (app.tab = 'duplicates') },
      {
        kind: 'action',
        label: 'Open cleanup cart',
        icon: ShoppingCart,
        run: () => (app.cartOpen = true),
      },
      { kind: 'action', label: 'Rescan drives…', icon: RefreshCw, run: onRescanDrives },
      { kind: 'action', label: 'Settings', icon: SettingsIcon, run: onSettings },
      {
        kind: 'action',
        label: app.sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar',
        hint: 'Ctrl B',
        icon: PanelLeft,
        run: () => app.toggleSidebar(),
      },
      { kind: 'action', label: 'Theme: system', icon: Monitor, run: () => app.setTheme('system') },
      { kind: 'action', label: 'Theme: dark', icon: Moon, run: () => app.setTheme('dark') },
      { kind: 'action', label: 'Theme: light', icon: Sun, run: () => app.setTheme('light') },
      { kind: 'action', label: 'Keyboard shortcuts', hint: '?', icon: Keyboard, run: onShortcuts },
    );
    if (app.helper === 'notInstalled' || app.helper === 'outdated')
      out.push({
        kind: 'action',
        label: 'Allow fast mode',
        icon: Zap,
        run: () => app.allowFastMode(),
      });
    for (const drv of app.drives)
      out.push({
        kind: 'action',
        label: `Go to ${drv.letter}: ${drv.label || 'Local disk'}`,
        icon: HardDrive,
        run: () => app.select(drv.letter),
      });
    return out;
  });

  /** Loose match: every character of the query appears in order. */
  function fuzzy(label: string, q: string) {
    const l = label.toLowerCase();
    let i = 0;
    for (const c of q.toLowerCase()) {
      i = l.indexOf(c, i);
      if (i < 0) return false;
      i++;
    }
    return true;
  }

  // The folder open in the chart, offered as a search scope.
  const folder = $derived(app.current ? (app.focus[app.current] ?? null) : null);
  $effect(() => {
    const l = app.current;
    const id = folder;
    if (!l || id === null) folderName = '';
    else api.nodeInfo(l, id).then((i) => (folderName = i.name));
  });
  const scope = $derived.by((): SearchScope => {
    const l = app.current;
    if (app.searchScope === 'folder' && l && folder !== null) return { letter: l, folder };
    if (app.searchScope !== 'all' && l) return { letter: l, folder: null };
    return { letter: null, folder: null };
  });
  const scopes = $derived([
    { k: 'all' as const, label: 'All drives', icon: Globe },
    ...(app.current ? [{ k: 'drive' as const, label: `${app.current}: drive`, icon: HardDrive }] : []),
    ...(folder !== null && folderName
      ? [{ k: 'folder' as const, label: folderName, icon: FolderOpen }]
      : []),
  ]);
  const scopeKey = $derived(
    app.searchScope === 'folder' && folder === null ? 'drive' : app.searchScope,
  );

  // Filters, wildcards and paths only make sense as a file search.
  const hasFilters = $derived(/(^|\s)[-!]?[a-z]+:\S|[*?/\\|]/i.test(text));
  const items = $derived.by((): Item[] => {
    const q = text.trim();
    const acts = hasFilters
      ? []
      : actions.filter((a) => !q || fuzzy(a.label, q)).slice(0, q ? 4 : 8);
    return [...acts, ...hits.map((hit) => ({ kind: 'hit' as const, hit }))];
  });

  // Debounced search as you type.
  $effect(() => {
    const q = text.trim();
    const sc = scope;
    if (!open) return;
    picked = {};
    pickedAll = null;
    if (q.length < 2) {
      hits = [];
      stats = null;
      return;
    }
    const mine = ++seq;
    const t = setTimeout(async () => {
      const r = await api.search(q, sc);
      if (mine !== seq) return;
      hits = r.hits;
      stats = { total: r.total, ms: r.elapsedMs };
    }, 90);
    return () => clearTimeout(t);
  });

  const key = (h: SearchHit) => `${h.letter}:${h.id}`;
  const pickedCount = $derived(
    pickedAll ? pickedAll.reduce((n, d) => n + d.ids.length, 0) : Object.keys(picked).length,
  );
  const pickedSize = $derived(
    pickedAll ? null : Object.values(picked).reduce((n, h) => n + h.alloc, 0),
  );
  const allPicked = $derived(
    pickedAll !== null || (hits.length > 0 && hits.every((h) => picked[key(h)])),
  );

  function toggle(h: SearchHit) {
    if (pickedAll) {
      // Leaving "every match" mode: keep the visible ones except this.
      picked = Object.fromEntries(hits.filter((x) => x !== h).map((x) => [key(x), x]));
      pickedAll = null;
      return;
    }
    const k = key(h);
    if (picked[k]) delete picked[k];
    else picked[k] = h;
  }

  async function toggleAll() {
    if (allPicked) {
      picked = {};
      pickedAll = null;
      return;
    }
    picked = Object.fromEntries(hits.map((h) => [key(h), h]));
    if (stats && stats.total > hits.length) {
      const mine = seq;
      const all = await api.searchIds(text.trim(), scope);
      if (mine === seq) pickedAll = all;
    }
  }

  /** Add the selection (or the highlighted hit) to the cleanup cart. */
  async function addPicked() {
    const groups =
      pickedAll ??
      Object.values(
        Object.values(picked).reduce<Record<string, { letter: string; ids: number[] }>>(
          (acc, h) => {
            (acc[h.letter] ??= { letter: h.letter, ids: [] }).ids.push(h.id);
            return acc;
          },
          {},
        ),
      );
    if (!groups.length) {
      const cur = items[active];
      if (cur?.kind === 'hit') groups.push({ letter: cur.hit.letter, ids: [cur.hit.id] });
    }
    if (!groups.length) return;
    open = false;
    for (const g of groups) await app.addToCart(g.letter, g.ids);
  }

  $effect(() => {
    void items.length;
    if (active >= items.length) active = Math.max(0, items.length - 1);
  });

  function move(d: number) {
    if (!items.length) return;
    active = (active + d + items.length) % items.length;
    const top = active * ROW;
    if (list && (top < list.scrollTop || top + ROW > list.scrollTop + list.clientHeight))
      list.scrollTo({ top: top - list.clientHeight / 2 + ROW / 2, behavior: 'smooth' });
  }

  async function run(item: Item | undefined, reveal = false) {
    if (!item) return;
    open = false;
    if (item.kind === 'action') return item.run();
    if (reveal) return api.showInExplorer(item.hit.path);
    await app.goTo(item.hit.letter, item.hit.id);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      move(1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      move(-1);
    } else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void addPicked();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(items[active], e.shiftKey);
    } else if (e.key === 'Tab') {
      e.preventDefault();
      const cur = items[active];
      if (cur?.kind === 'hit') {
        toggle(cur.hit);
        move(1);
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (help) help = false;
      else open = false;
    }
  }

  const examples = [
    'kind:video',
    'ext:pdf',
    'size:>1GB',
    'modified:<30d',
    'type:dir',
    'is:empty',
    'sort:size',
  ];

  const syntax: [string, string][] = [
    ['tax 2024', 'Every word somewhere in the name'],
    ['"tax return"', 'Exact phrase, spaces included'],
    ['labels_2026*.pdf', 'Wildcards: * any text, ? one character'],
    ['Downloads/invoice', 'Folder, then name. C:/Users/… starts at a drive'],
    ['kind:video', 'Also image, audio, document, spreadsheet, archive, code…'],
    ['ext:pdf,docx', 'Extensions. Wildcards work too: ext:xls*'],
    ['size:>1GB', 'Size on disk. Also <10MB, 1GB..5GB'],
    ['modified:<30d', 'Also modified:2024, >2024-06, older:1y, newer:7d'],
    ['type:dir', 'Only folders (type:file for files)'],
    ['is:empty', 'Also is:hidden, is:system, is:new'],
    ['files:>1000', 'Folders holding at least that many files'],
    ['in:projects', 'Inside a folder whose name contains this'],
    ['-ext:tmp', 'Leave out anything matching (! works too)'],
    ['mp4 | mkv', 'Either one matches (OR works too)'],
    ['drive:d', 'Only these drives'],
    ['sort:size', 'Also small, name, newest, oldest, files'],
  ];

  function addFilter(f: string) {
    text = (text.trim() + ' ' + f).trim() + ' ';
    help = false;
    input?.focus();
  }
</script>

{#if open}
  <div
    class="fixed inset-0 z-40 bg-black/30"
    transition:fade={{ duration: 140 }}
    onclick={() => (open = false)}
    role="presentation"
  ></div>
  <div
    class="panel fixed top-[12%] left-1/2 z-50 flex max-h-[min(560px,72vh)] w-[min(640px,90vw)] -translate-x-1/2 flex-col overflow-hidden rounded-2xl border border-line-strong"
    transition:scale={{ start: 0.97, duration: 180, easing: cubicOut }}
    role="dialog"
    aria-label="Command palette"
  >
    <div class="flex items-center gap-2.5 border-b border-line px-4">
      <Search class="size-4 shrink-0 text-fg-subtle" />
      <input
        bind:this={input}
        bind:value={text}
        onkeydown={onKey}
        class="h-12 flex-1 bg-transparent text-[14px] outline-none placeholder:text-fg-subtle"
        placeholder="Search files on all drives, or type a command…"
        spellcheck="false"
        autocomplete="off"
      />
      <button
        class="pressable grid size-7 shrink-0 place-items-center rounded-md {help
          ? 'bg-accent-soft text-accent'
          : 'text-fg-subtle hover:bg-hover hover:text-fg'}"
        title="Search syntax"
        aria-label="Search syntax"
        aria-pressed={help}
        onclick={() => {
          help = !help;
          input?.focus();
        }}><CircleHelp class="size-4" /></button
      >
    </div>

    <div class="flex items-center gap-2 border-b border-line px-3 py-1.5 text-[11.5px]">
      <span class="pl-1 text-fg-subtle">In</span>
      <div class="flex min-w-0 rounded-lg bg-panel-2 p-0.5">
        {#each scopes as sc (sc.k)}
          <button
            class="flex min-w-0 items-center gap-1.5 rounded-md px-2 py-0.5 transition-colors {scopeKey ===
            sc.k
              ? 'bg-panel text-fg shadow-sm'
              : 'text-fg-subtle hover:text-fg-muted'}"
            aria-pressed={scopeKey === sc.k}
            onclick={() => {
              app.searchScope = sc.k;
              input?.focus();
            }}
            ><sc.icon class="size-3.5 shrink-0" /><span class="max-w-[160px] truncate"
              >{sc.label}</span
            ></button
          >
        {/each}
      </div>
      {#if stats}
        <span class="tabular ml-auto shrink-0 text-fg-subtle" in:fade={{ duration: 120 }}>
          {count(stats.total)} found · {stats.ms < 10 ? stats.ms.toFixed(1) : Math.round(stats.ms)} ms
        </span>
      {/if}
    </div>

    {#if !text.trim() && !help}
      <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-4 py-2 text-[11.5px]">
        <span class="text-fg-subtle">Filters:</span>
        {#each examples as f (f)}
          <button
            class="pressable rounded-md border border-line px-1.5 py-0.5 font-mono text-[11px] text-fg-muted hover:border-line-strong hover:text-fg"
            onclick={() => addFilter(f)}>{f}</button
          >
        {/each}
        <button class="ml-auto text-fg-subtle hover:text-fg" onclick={() => (help = true)}
          >More…</button
        >
      </div>
    {/if}

    {#if hits.length && !help}
      <div
        class="flex h-9 shrink-0 items-center gap-2.5 border-b border-line px-4 text-[12px]"
        transition:fade={{ duration: 120 }}
      >
        <button
          class="check {allPicked ? 'on' : pickedCount ? 'some' : ''}"
          role="checkbox"
          aria-checked={allPicked ? 'true' : pickedCount ? 'mixed' : 'false'}
          aria-label="Select all"
          onclick={toggleAll}
        >
          {#if allPicked}<Check class="size-3" />{:else if pickedCount}<Minus
              class="size-3"
            />{/if}
        </button>
        {#if pickedCount}
          <span class="tabular text-fg">
            {count(pickedCount)} selected{pickedSize !== null ? ` · ${bytes(pickedSize)}` : ''}
          </span>
          {#if !pickedAll && stats && stats.total > hits.length && allPicked}
            <button class="text-accent hover:underline" onclick={toggleAll}
              >Select all {count(stats.total)}</button
            >
          {/if}
          <button
            class="pressable ml-auto flex h-6 items-center gap-1.5 rounded-md bg-accent px-2.5 text-[11.5px] font-medium text-white"
            onclick={addPicked}
            ><ShoppingCart class="size-3.5" /> Add to cart
            <kbd class="opacity-70">Ctrl ↵</kbd></button
          >
        {:else}
          <span class="text-fg-subtle"
            >Select {stats && stats.total > hits.length
              ? `all ${count(stats.total)}`
              : 'all'} · <kbd class="kbd">Tab</kbd> picks one</span
          >
        {/if}
      </div>
    {/if}

    {#if help}
      <div class="min-h-0 flex-1 overflow-y-auto p-2" in:fade={{ duration: 120 }}>
        <div class="px-2 pt-1 pb-2 text-[11.5px] text-fg-subtle">
          Combine any of these; every part has to match. Click one to add it.
        </div>
        <div class="grid grid-cols-1 gap-x-3 sm:grid-cols-2">
          {#each syntax as [ex, what] (ex)}
            <button
              class="flex flex-col items-start rounded-lg px-2.5 py-1.5 text-left hover:bg-hover"
              onclick={() => addFilter(ex)}
            >
              <span class="font-mono text-[12px] text-fg">{ex}</span>
              <span class="text-[11px] text-fg-subtle">{what}</span>
            </button>
          {/each}
        </div>
      </div>
    {:else}

    <div class="relative min-h-0 flex-1 overflow-y-auto p-1.5" bind:this={list}>
      {#if items.length}
        <div
          class="highlight absolute inset-x-1.5 rounded-lg"
          style="height: {ROW}px; transform: translateY({active * ROW}px)"
        ></div>
      {:else if text.trim().length >= 2 && stats}
        <div class="p-6 text-center text-fg-muted">Nothing matches “{text.trim()}”.</div>
      {/if}
      {#each items as item, i (item.kind === 'hit' ? `h${item.hit.letter}${item.hit.id}` : `a${item.label}`)}
        <button
          class="relative flex w-full items-center gap-3 rounded-lg px-3 text-left"
          style="height: {ROW}px"
          onpointermove={() => (active = i)}
          onclick={(e) => run(item, e.shiftKey)}
        >
          {#if item.kind === 'action'}
            <item.icon class="size-4 shrink-0 text-fg-muted" />
            <span class="flex-1 truncate">{item.label}</span>
            {#if item.hint}<kbd class="kbd">{item.hint}</kbd>{/if}
          {:else}
            {@const h = item.hit}
            {@const on = !!pickedAll || !!picked[key(h)]}
            <span
              class="check {on ? 'on' : ''}"
              role="checkbox"
              tabindex="-1"
              aria-checked={on}
              aria-label="Select {h.name}"
              onclick={(e) => {
                e.stopPropagation();
                toggle(h);
              }}
              onkeydown={() => {}}
            >
              {#if on}<Check class="size-3" />{/if}
            </span>
            <span class="shrink-0 {h.dir ? 'text-accent' : 'text-fg-subtle'}">
              {#if h.dir}<Folder class="size-4" />{:else}<File class="size-4" />{/if}
            </span>
            <span class="min-w-0 flex-1">
              <span class="block truncate">{h.name}</span>
              <span class="block truncate text-[11px] text-fg-subtle">{h.path}</span>
            </span>
            <span class="tabular shrink-0 text-right text-[12px]">
              <span class="block">{bytes(h.alloc)}</span>
              <span class="block text-[10.5px] text-fg-subtle">{ago(h.mtime)}</span>
            </span>
          {/if}
        </button>
      {/each}
    </div>
    {/if}

    <div class="flex items-center gap-3 border-t border-line px-4 py-2 text-[11px] text-fg-subtle">
      <span><kbd class="kbd">↑</kbd> <kbd class="kbd">↓</kbd> move</span>
      <span class="flex items-center gap-1"
        ><kbd class="kbd"><CornerDownLeft class="size-2.5" /></kbd> open</span
      >
      <span
        ><kbd class="kbd">Shift</kbd> +
        <kbd class="kbd"><CornerDownLeft class="inline size-2.5" /></kbd> show in Explorer</span
      >
      <span><kbd class="kbd">Tab</kbd> select</span>
      <span class="ml-auto"><kbd class="kbd">Esc</kbd> close</span>
    </div>
  </div>
{/if}

<style>
  .panel {
    background: var(--popover);
    box-shadow:
      0 30px 80px -20px rgba(0, 0, 0, 0.6),
      var(--shadow);
    backdrop-filter: blur(24px) saturate(1.2);
  }
  .check {
    display: grid;
    flex-shrink: 0;
    place-items: center;
    width: 15px;
    height: 15px;
    border: 1px solid var(--line-strong);
    border-radius: 4px;
    color: white;
    transition:
      background 120ms,
      border-color 120ms;
  }
  .check:hover {
    border-color: var(--accent);
  }
  .check.on,
  .check.some {
    border-color: var(--accent);
    background: var(--accent);
  }
  .highlight {
    top: 6px;
    background: var(--accent-soft);
    transition: transform 140ms var(--ease-out-soft);
  }
  :global(.kbd) {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--line-strong);
    border-radius: 4px;
    padding: 0 4px;
    font-size: 10.5px;
    line-height: 16px;
    color: var(--fg-muted);
  }
</style>
