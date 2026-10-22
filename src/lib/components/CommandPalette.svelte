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
  import CornerDownLeft from '@lucide/svelte/icons/corner-down-left';
  import { api, type SearchHit } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { ago, bytes, count } from '$lib/format';

  let { open = $bindable(false), onShortcuts }: { open: boolean; onShortcuts: () => void } =
    $props();

  type Action = { kind: 'action'; label: string; hint?: string; icon: Component; run: () => void };
  type Item = Action | { kind: 'hit'; hit: SearchHit };

  let input: HTMLInputElement | undefined = $state();
  let list: HTMLDivElement | undefined = $state();
  let text = $state('');
  let hits = $state<SearchHit[]>([]);
  let stats = $state<{ total: number; ms: number } | null>(null);
  let active = $state(0);
  let seq = 0;

  const ROW = 38;

  $effect(() => {
    if (open) {
      text = '';
      hits = [];
      stats = null;
      active = 0;
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
        label: 'Turn on Turbo scan',
        icon: Zap,
        run: () => app.enableFastScan(),
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

  const hasFilters = $derived(/\b(ext|size|older|newer|before|after|type|is|in):/i.test(text));
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
    if (!open) return;
    if (q.length < 2) {
      hits = [];
      stats = null;
      return;
    }
    const mine = ++seq;
    const t = setTimeout(async () => {
      const r = await api.search(q);
      if (mine !== seq) return;
      hits = r.hits;
      stats = { total: r.total, ms: r.elapsedMs };
    }, 90);
    return () => clearTimeout(t);
  });

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
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(items[active], e.shiftKey);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      open = false;
    }
  }

  const examples = ['ext:mp4,mkv', 'size:>1GB', 'older:1y', 'type:dir', 'in:downloads'];

  function addFilter(f: string) {
    text = (text.trim() + ' ' + f).trim() + ' ';
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
      {#if stats}
        <span class="tabular shrink-0 text-[11px] text-fg-subtle" in:fade={{ duration: 120 }}>
          {count(stats.total)} found · {stats.ms < 10 ? stats.ms.toFixed(1) : Math.round(stats.ms)} ms
        </span>
      {/if}
    </div>

    {#if !text.trim()}
      <div class="flex flex-wrap items-center gap-1.5 border-b border-line px-4 py-2 text-[11.5px]">
        <span class="text-fg-subtle">Filters:</span>
        {#each examples as f (f)}
          <button
            class="pressable rounded-md border border-line px-1.5 py-0.5 font-mono text-[11px] text-fg-muted hover:border-line-strong hover:text-fg"
            onclick={() => addFilter(f)}>{f}</button
          >
        {/each}
      </div>
    {/if}

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

    <div class="flex items-center gap-3 border-t border-line px-4 py-2 text-[11px] text-fg-subtle">
      <span><kbd class="kbd">↑</kbd> <kbd class="kbd">↓</kbd> move</span>
      <span class="flex items-center gap-1"
        ><kbd class="kbd"><CornerDownLeft class="size-2.5" /></kbd> open</span
      >
      <span
        ><kbd class="kbd">Shift</kbd> +
        <kbd class="kbd"><CornerDownLeft class="inline size-2.5" /></kbd> show in Explorer</span
      >
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
