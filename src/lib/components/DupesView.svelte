<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { nodeMenu } from '$lib/menus';
  import { fade, fly, slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import Copy from '@lucide/svelte/icons/copy';
  import File from '@lucide/svelte/icons/file';
  import ShoppingCart from '@lucide/svelte/icons/shopping-cart';
  import { api, type DupeGroup, type DupesEvent } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, count, date, duration } from '$lib/format';

  let { letter }: { letter: string } = $props();

  const sizes = [
    { label: '1 MB', v: 1 << 20 },
    { label: '10 MB', v: 10 << 20 },
    { label: '100 MB', v: 100 << 20 },
  ];
  let minSize = $state(1 << 20);
  let skipSystem = $state(true);
  // Start with the folder open in Browse, when there is one.
  let inFolder = $state(untrack(() => app.focus[letter] ?? null) !== null);
  let folderName = $state('');
  let running = $state(false);
  let progress = $state<Extract<DupesEvent, { type: 'progress' }> | null>(null);
  // Results survive switching tabs.
  const kept = untrack(() => app.dupes[letter]);
  let groups = $state<DupeGroup[] | null>(kept?.groups ?? null);
  let meta = $state<{ ms: number; candidates: number } | null>(kept?.meta ?? null);
  let scopePath = $state(kept?.scope ?? '');
  let folderPath = $state('');
  let error = $state('');
  /** Selected file paths (to add to the cart). */
  let picked = $state<Record<string, boolean>>({});

  const focus = $derived(app.focus[letter] ?? null);
  $effect(() => {
    const id = focus;
    if (id === null) folderName = folderPath = '';
    else
      api.nodeInfo(letter, id).then((i) => {
        folderName = i.name;
        folderPath = i.path;
      });
  });
  const wasted = $derived((groups ?? []).reduce((s, g) => s + g.size * (g.files.length - 1), 0));
  const pickedList = $derived(
    (groups ?? []).flatMap((g) =>
      g.files.filter((f) => picked[f.path]).map((f) => ({ ...f, size: g.size })),
    ),
  );
  const pickedSize = $derived(pickedList.reduce((s, f) => s + f.size, 0));

  async function start() {
    running = true;
    groups = null;
    error = '';
    picked = {};
    progress = null;
    const scope = inFolder && focus !== null ? folderPath : `${letter}:\\`;
    await api.dupesStart(letter, inFolder ? focus : null, minSize, skipSystem, (e) => {
      if (e.type === 'cancelled') running = false;
      if (e.type === 'progress') progress = e;
      else {
        running = false;
        if (e.type === 'done') {
          groups = e.groups;
          meta = { ms: e.elapsedMs, candidates: e.candidates };
          scopePath = scope;
          app.dupes[letter] = { groups: e.groups, scope, meta };
        }
      }
    });
  }

  onDestroy(() => {
    if (running) void api.dupesCancel();
  });

  /** In each group, pick every copy except the newest (or oldest). */
  function keepOne(which: 'newest' | 'oldest') {
    const next: Record<string, boolean> = {};
    for (const g of groups ?? []) {
      const sorted = [...g.files].sort((a, b) =>
        which === 'newest' ? b.mtime - a.mtime : a.mtime - b.mtime,
      );
      for (const f of sorted.slice(1)) next[f.path] = true;
    }
    picked = next;
  }

  async function addPicked() {
    const ids = pickedList.map((f) => f.id);
    await app.addToCart(letter, ids);
    picked = {};
  }

  const phaseLabel = ['Listing files', 'Quick check', 'Comparing contents'];
  const frac = $derived.by(() => {
    if (!progress) return 0;
    if (progress.phase === 2 && progress.bytesTotal)
      return progress.bytesDone / progress.bytesTotal;
    return progress.filesTotal ? progress.filesDone / progress.filesTotal : 0;
  });

  function name(p: string) {
    return p.slice(p.lastIndexOf('\\') + 1);
  }
  function folder(p: string) {
    return p.slice(0, p.lastIndexOf('\\'));
  }
</script>

<div class="flex h-full flex-col" in:fade={{ duration: 160 }}>
  <!-- Options -->
  <div
    class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-4 py-2.5 text-[12px]"
  >
    <div class="flex items-center gap-1.5">
      <span class="text-fg-subtle">Min size</span>
      {#each sizes as s (s.v)}
        <button
          class="pressable rounded-md border px-2 py-0.5 {minSize === s.v
            ? 'border-accent bg-accent-soft text-fg'
            : 'border-line text-fg-muted hover:text-fg'}"
          disabled={running}
          onclick={() => (minSize = s.v)}>{s.label}</button
        >
      {/each}
    </div>
    <label class="flex items-center gap-1.5 text-fg-muted">
      <input
        type="checkbox"
        bind:checked={skipSystem}
        disabled={running}
        class="accent-[var(--accent)]"
      />
      Skip Windows and program folders
    </label>
    <label class="flex items-center gap-1.5 text-fg-muted" class:opacity-40={focus === null}>
      <input
        type="checkbox"
        bind:checked={inFolder}
        disabled={running || focus === null}
        class="accent-[var(--accent)]"
      />
      <span class="max-w-[360px] truncate" title={folderPath}>
        {focus === null
          ? 'Only the open folder'
          : `Only ${folderPath || folderName || 'the open folder'}`}
      </span>
    </label>
    <div class="ml-auto">
      {#if running}
        <button
          class="pressable h-7 rounded-lg border border-line px-3 text-fg-muted hover:text-fg"
          onclick={() => api.dupesCancel()}>Cancel</button
        >
      {:else}
        <button
          class="pressable h-7 rounded-lg bg-accent px-3 font-medium text-accent-fg hover:brightness-110"
          onclick={start}>{groups ? 'Search again' : 'Find duplicates'}</button
        >
      {/if}
    </div>
  </div>

  {#if running}
    <div class="grid flex-1 place-items-center" in:fade>
      <div class="w-[340px] text-center">
        <div
          class="orb mx-auto mb-5 grid size-14 place-items-center rounded-2xl bg-accent-soft text-accent"
        >
          <Copy class="size-6" />
        </div>
        <div class="font-medium">{phaseLabel[progress?.phase ?? 0]}…</div>
        <div class="tabular mt-1 text-[12px] text-fg-subtle">
          {#if progress?.phase === 2}
            {bytes(progress.bytesDone)} of {bytes(progress.bytesTotal)}
          {:else if progress}
            {count(progress.filesDone)} of {count(progress.filesTotal)} files
          {/if}
        </div>
        <div class="mt-4 h-1.5 overflow-hidden rounded-full bg-panel-2">
          <div
            class="h-full rounded-full bg-accent transition-[width] duration-150"
            style="width: {Math.max(2, frac * 100)}%"
          ></div>
        </div>
      </div>
    </div>
  {:else if error}
    <div class="p-6 text-danger">{error}</div>
  {:else if groups === null}
    <div class="grid flex-1 place-items-center">
      <div
        class="flex max-w-[360px] flex-col items-center text-center"
        in:fly={{ y: 8, duration: 300 }}
      >
        <div class="mb-4 grid size-12 place-items-center rounded-2xl bg-panel-2 text-fg-muted">
          <Copy class="size-5" />
        </div>
        <div class="font-semibold">Find duplicate files</div>
        <p class="mt-1 leading-relaxed text-fg-muted">
          Files are compared by content, not just name, so only true copies show up. Nothing is
          deleted unless you add it to the cleanup cart and confirm.
        </p>
      </div>
    </div>
  {:else}
    <div class="flex shrink-0 items-center gap-3 px-4 py-3">
      <div>
        <div class="text-[15px] font-semibold tracking-tight">
          {groups.length ? `${bytes(wasted)} in duplicate copies` : 'No duplicates found'}
        </div>
        <div class="tabular text-[11.5px] text-fg-subtle">
          {count(groups.length)} sets · checked {count(meta?.candidates ?? 0)} files in {duration(
            meta?.ms ?? 0,
          )}{#if scopePath}
            · in <span class="mono" title={scopePath}>{scopePath}</span>{/if}
        </div>
      </div>
      {#if groups.length}
        <div class="ml-auto flex items-center gap-1.5 text-[12px]">
          <span class="text-fg-subtle">Select all but the</span>
          <button
            class="pressable rounded-md border border-line px-2 py-0.5 text-fg-muted hover:text-fg"
            onclick={() => keepOne('newest')}>newest</button
          >
          <button
            class="pressable rounded-md border border-line px-2 py-0.5 text-fg-muted hover:text-fg"
            onclick={() => keepOne('oldest')}>oldest</button
          >
          {#if pickedList.length}
            <button
              class="rounded-md px-2 py-0.5 text-fg-subtle hover:text-fg"
              onclick={() => (picked = {})}>Clear</button
            >
          {/if}
        </div>
      {/if}
    </div>

    <div class="min-h-0 flex-1 space-y-2 overflow-y-auto px-3 pb-3">
      {#each groups as g, gi (g.files[0].path)}
        {@const newest = Math.max(...g.files.map((f) => f.mtime))}
        <div
          class="group-card rounded-xl border border-line"
          in:fly={{ y: 6, duration: 240, delay: Math.min(gi, 10) * 25, easing: cubicOut }}
        >
          <div class="flex items-center gap-2 px-3 pt-2 pb-1 text-[12px]">
            <span class="font-medium">{name(g.files[0].path)}</span>
            <span class="text-fg-subtle">· {g.files.length} copies of {bytes(g.size)}</span>
            <span class="tabular ml-auto text-danger"
              >{bytes(g.size * (g.files.length - 1))} extra</span
            >
          </div>
          {#each g.files as f (f.path)}
            <!-- Clicking selects the file (for Ctrl+C etc.); the checkbox stays the main control. -->
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <label
              class="file flex cursor-default items-center gap-2.5 px-3 py-1.5 hover:bg-hover"
              class:selected={app.selected === f.id}
              onclick={() => app.selectNode(f.id)}
              oncontextmenu={(e) => {
                e.preventDefault();
                app.selectNode(f.id);
                nodeMenu(e, letter, f.id);
              }}
            >
              <input type="checkbox" bind:checked={picked[f.path]} class="accent-[var(--accent)]" />
              <File class="size-3.5 shrink-0 text-fg-subtle" />
              <span class="min-w-0 flex-1 truncate text-[12px] text-fg-muted" title={f.path}
                >{folder(f.path)}</span
              >
              {#if f.mtime === newest}
                <span class="rounded bg-accent-soft px-1.5 text-[10.5px] text-accent">newest</span>
              {/if}
              <span class="tabular shrink-0 text-[11.5px] text-fg-subtle">{date(f.mtime)}</span>
            </label>
          {/each}
        </div>
      {/each}
    </div>

    {#if pickedList.length}
      <div
        class="flex shrink-0 items-center gap-3 border-t border-line px-4 py-2.5"
        transition:slide={{ duration: 200 }}
      >
        <span class="tabular text-[12px] text-fg-muted">
          {count(pickedList.length)} selected · {bytes(pickedSize)}
        </span>
        <button
          class="pressable ml-auto flex h-8 items-center gap-2 rounded-lg bg-accent px-3 text-[12.5px] font-medium text-accent-fg hover:brightness-110"
          onclick={addPicked}><ShoppingCart class="size-3.5" /> Add to cleanup cart</button
        >
      </div>
    {/if}
  {/if}
</div>

<style>
  .file.selected {
    background: var(--accent-soft);
  }
  .group-card {
    content-visibility: auto;
    contain-intrinsic-size: auto 120px;
  }
  .orb {
    animation: breathe 2.4s ease-in-out infinite;
  }
  @keyframes breathe {
    50% {
      transform: scale(1.06);
    }
  }
</style>
