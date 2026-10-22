<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { Tween } from 'svelte/motion';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Folder from '@lucide/svelte/icons/folder';
  import File from '@lucide/svelte/icons/file';
  import History from '@lucide/svelte/icons/history';
  import TrendingUp from '@lucide/svelte/icons/trending-up';
  import TrendingDown from '@lucide/svelte/icons/trending-down';
  import { api, type DiffItem, type DiffSummary, type HistoryEntry } from '$lib/api';
  import { ago, bytes, count, date } from '$lib/format';

  let { letter, method }: { letter: string; method: string | null } = $props();

  let entries = $state<HistoryEntry[] | null>(null);
  let chosen = $state<string | null>(null);
  let summary = $state<DiffSummary | null>(null);
  let loading = $state(false);
  let error = $state('');
  /** Children per expanded item, keyed by `old:new` ids. */
  let kids = $state<Record<string, DiffItem[]>>({});
  let open = $state<Record<string, boolean>>({});
  const ROOT = 'root';

  const key = (i: DiffItem) => `${i.oldId ?? 'x'}:${i.newId ?? 'x'}`;

  $effect(() => {
    void letter;
    entries = null;
    api.history(letter).then((h) => {
      entries = h;
      // Default: a snapshot taken the same way as the current data (a standard scan
      // can't see protected folders, so mixing them shows false changes), at least
      // an hour old if possible.
      const hourAgo = Date.now() / 1000 - 3600;
      const same = h.filter((e) => e.method === (method ?? e.method));
      const pool = same.length ? same : h;
      const pick = pool.find((e) => e.taken < hourAgo) ?? pool[pool.length - 1];
      if (pick) choose(pick.path);
    });
  });

  onDestroy(() => void api.diffClose(letter));

  async function choose(path: string) {
    chosen = path;
    loading = true;
    error = '';
    try {
      summary = await api.diffOpen(letter, path);
      open = {};
      kids = { [ROOT]: await api.diffChildren(letter, summary.oldRoot, summary.newRoot) };
    } catch (e) {
      error = String(e);
      summary = null;
    } finally {
      loading = false;
    }
  }

  async function toggle(item: DiffItem) {
    if (!item.hasKids) return;
    const k = key(item);
    if (open[k]) {
      open[k] = false;
      return;
    }
    if (!kids[k]) kids[k] = await api.diffChildren(letter, item.oldId, item.newId);
    open[k] = true;
  }

  type Row = { item: DiffItem; depth: number; max: number };
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const walk = (list: DiffItem[] | undefined, depth: number) => {
      if (!list) return;
      const max = Math.max(1, ...list.map((i) => Math.abs(i.delta)));
      for (const item of list) {
        out.push({ item, depth, max });
        const k = key(item);
        if (open[k]) walk(kids[k], depth + 1);
      }
    };
    walk(kids[ROOT], 0);
    return out;
  });

  const delta = $derived(summary ? summary.newAlloc - summary.oldAlloc : 0);
  const chosenMethod = $derived(entries?.find((e) => e.path === chosen)?.method ?? null);
  const shown = new Tween(0, { duration: 700, easing: cubicOut });
  $effect(() => {
    shown.target = delta;
  });

  function signed(n: number) {
    if (n === 0) return '±0 B';
    return `${n > 0 ? '+' : '−'}${bytes(Math.abs(n))}`;
  }

  function signedCount(n: number) {
    if (n === 0) return 'no change';
    return `${n > 0 ? '+' : '−'}${count(Math.abs(n))} files`;
  }
</script>

<div class="flex h-full flex-col" in:fade={{ duration: 160 }}>
  {#if entries === null}
    <div class="grid flex-1 place-items-center text-fg-subtle">Loading snapshots…</div>
  {:else if entries.length === 0}
    <div class="grid flex-1 place-items-center">
      <div
        class="flex max-w-[340px] flex-col items-center text-center"
        in:fly={{ y: 8, duration: 300 }}
      >
        <div class="mb-4 grid size-12 place-items-center rounded-2xl bg-panel-2 text-fg-muted">
          <History class="size-5" />
        </div>
        <div class="font-semibold">No earlier snapshots yet</div>
        <p class="mt-1 leading-relaxed text-fg-muted">
          Bulk saves a snapshot after every scan and once a day. Come back later to see what grew.
        </p>
      </div>
    </div>
  {:else}
    <!-- Snapshot picker -->
    <div class="flex shrink-0 items-center gap-2 border-b border-line px-4 py-2.5">
      <span class="shrink-0 text-[12px] text-fg-subtle">Compare with</span>
      <div class="chips flex min-w-0 gap-1.5 overflow-x-auto pb-0.5">
        {#each entries as e (e.path)}
          <button
            class="pressable shrink-0 rounded-lg border px-2.5 py-1 text-left text-[12px] {chosen ===
            e.path
              ? 'border-accent bg-accent-soft text-fg'
              : 'border-line text-fg-muted hover:border-line-strong hover:text-fg'}"
            title={date(e.taken)}
            onclick={() => choose(e.path)}
          >
            <div class="whitespace-nowrap">{ago(e.taken)}</div>
            <div class="tabular text-[10.5px] text-fg-subtle">
              {bytes(e.alloc)}{e.method !== 'mft' ? ' · standard' : ''}
            </div>
          </button>
        {/each}
      </div>
    </div>

    {#if summary}
      <div class="flex shrink-0 items-center gap-5 px-5 pt-4 pb-3">
        <div
          class="grid size-11 place-items-center rounded-xl {delta > 0
            ? 'bg-[color-mix(in_oklab,var(--danger)_15%,transparent)] text-danger'
            : 'bg-[color-mix(in_oklab,var(--ok)_15%,transparent)] text-ok'}"
        >
          {#if delta > 0}<TrendingUp class="size-5" />{:else}<TrendingDown class="size-5" />{/if}
        </div>
        <div>
          <div class="tabular text-[26px] leading-tight font-semibold tracking-tight">
            {signed(Math.round(shown.current))}
          </div>
          <div class="text-[12px] text-fg-muted">
            since {date(summary.taken)} · {bytes(summary.oldAlloc)} → {bytes(summary.newAlloc)} · {signedCount(
              summary.newFiles - summary.oldFiles,
            )}
          </div>
          {#if chosenMethod && method && chosenMethod !== method}
            <div class="mt-1 text-[11.5px] text-warn" in:fade>
              That snapshot came from a standard scan, which can't see protected folders, so some
              differences are just what it couldn't see.
            </div>
          {/if}
        </div>
      </div>
    {/if}

    <div class="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2" class:opacity-50={loading}>
      {#if error}
        <div class="p-4 text-danger">{error}</div>
      {:else if summary && rows.length === 0 && !loading}
        <div class="p-6 text-center text-fg-muted">Nothing changed since then.</div>
      {/if}
      {#each rows as { item, depth, max }, i (key(item) + depth)}
        <button
          class="row flex h-8 w-full items-center gap-2 rounded-lg pr-3 text-left"
          style="padding-left: {8 + depth * 16}px"
          onclick={() => toggle(item)}
          in:fly={{ y: -4, duration: 220, delay: Math.min(i, 12) * 12, easing: cubicOut }}
        >
          <span
            class="chev grid size-5 shrink-0 place-items-center text-fg-subtle"
            class:invisible={!item.hasKids || !item.dir}
            class:open={open[key(item)]}
          >
            <ChevronRight class="size-3.5" />
          </span>
          <span class="shrink-0 {item.dir ? 'text-accent' : 'text-fg-subtle'}">
            {#if item.dir}<Folder class="size-3.5" />{:else}<File class="size-3.5" />{/if}
          </span>
          <span
            class="min-w-0 truncate"
            class:line-through={item.status === 'removed'}
            class:text-fg-muted={item.status === 'removed'}
          >
            {item.name}
          </span>
          {#if item.status !== 'changed'}
            <span
              class="shrink-0 rounded px-1.5 py-px text-[10.5px] font-medium {item.status ===
              'added'
                ? 'bg-[color-mix(in_oklab,var(--danger)_14%,transparent)] text-danger'
                : 'bg-[color-mix(in_oklab,var(--ok)_14%,transparent)] text-ok'}"
              >{item.status === 'added' ? 'New' : 'Deleted'}</span
            >
          {/if}
          <span class="ml-auto flex w-[120px] shrink-0 items-center">
            <span class="relative h-1.5 w-full overflow-hidden rounded-full bg-panel-2">
              <span
                class="absolute inset-y-0 left-0 rounded-full {item.delta > 0
                  ? 'bg-danger'
                  : 'bg-ok'}"
                style="width: {(Math.abs(item.delta) / max) * 100}%"
              ></span>
            </span>
          </span>
          <span
            class="tabular w-[92px] shrink-0 text-right font-medium {item.delta > 0
              ? 'text-danger'
              : item.delta < 0
                ? 'text-ok'
                : 'text-fg-subtle'}">{signed(item.delta)}</span
          >
          <span
            class="tabular hidden w-[170px] shrink-0 truncate text-right text-[11.5px] text-fg-subtle lg:block"
          >
            {bytes(item.oldAlloc)} → {bytes(item.newAlloc)}
          </span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .row {
    transition: background-color 120ms ease;
  }
  .row:hover {
    background: var(--hover);
  }
  .chev :global(svg) {
    transition: transform 180ms var(--ease-out-soft);
  }
  .chev.open :global(svg) {
    transform: rotate(90deg);
  }
  .chips {
    scrollbar-width: none;
  }
</style>
