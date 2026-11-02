<script lang="ts">
  import { untrack } from 'svelte';
  import { fade, slide } from 'svelte/transition';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import ShoppingCart from '@lucide/svelte/icons/shopping-cart';
  import EyeOff from '@lucide/svelte/icons/eye-off';
  import Eye from '@lucide/svelte/icons/eye';
  import Crosshair from '@lucide/svelte/icons/crosshair';
  import FolderSearch from '@lucide/svelte/icons/folder-search';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Wrench from '@lucide/svelte/icons/wrench';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import { api, type CleanupResult, type Finding } from '$lib/api';
  import { app, type MenuItem } from '$lib/app.svelte';
  import { ago, bytes, count } from '$lib/format';

  let { letter }: { letter: string } = $props();

  // Start with the folder open in Browse, when there is one.
  let inFolder = $state(untrack(() => app.focus[letter] ?? null) !== null);
  let folderName = $state('');
  let data = $state<CleanupResult | null>(null);
  let loading = $state(false);
  /** Ticked items, by node id. */
  let picked = $state<Set<number>>(new Set());
  /** Findings showing every item rather than the biggest few. */
  let open = $state<Record<string, boolean>>({});

  const focus = $derived(app.focus[letter] ?? null);
  $effect(() => {
    const id = focus;
    if (id === null) folderName = '';
    else api.nodeInfo(letter, id).then((i) => (folderName = i.name));
  });

  // Reload when the scope changes or the drive updates (live changes, rescans).
  let seq = 0;
  let reloads = $state(0);
  $effect(() => {
    const folder = inFolder ? focus : null;
    void app.rowsVersion;
    void reloads;
    const mine = ++seq;
    loading = data === null;
    const t = setTimeout(async () => {
      const r = await api.cleanup(letter, folder).catch(() => null);
      if (mine !== seq) return;
      data = r;
      loading = false;
      if (r && folder === null) app.cleanup[letter] = r;
      // Forget ticks on items that are no longer suggested.
      const ids = new Set(r?.findings.flatMap((f) => f.items.map((i) => i.id)) ?? []);
      picked = new Set([...picked].filter((id) => ids.has(id)));
    }, 60);
    return () => clearTimeout(t);
  });

  const SHOWN = 5;
  const badge = {
    safe: { label: 'Safe', cls: 'bg-ok/15 text-ok' },
    review: { label: 'Check first', cls: 'bg-warn/15 text-warn' },
    info: { label: 'Good to know', cls: 'bg-panel-2 text-fg-muted' },
  } as const;

  const pickedSize = $derived(
    (data?.findings ?? [])
      .flatMap((f) => f.items)
      .filter((i) => picked.has(i.id))
      .reduce((s, i) => s + i.alloc, 0),
  );

  function toggle(id: number) {
    const next = new Set(picked);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    picked = next;
  }

  function allPicked(f: Finding) {
    return f.items.every((i) => picked.has(i.id));
  }

  function toggleAll(f: Finding) {
    const next = new Set(picked);
    const on = !allPicked(f);
    for (const i of f.items) on ? next.add(i.id) : next.delete(i.id);
    picked = next;
  }

  async function addToCart(ids: number[]) {
    if (!ids.length) return;
    await app.addToCart(letter, ids);
    const next = new Set(picked);
    for (const id of ids) next.delete(id);
    picked = next;
  }

  /** Everything marked safe, in one go (still reviewed in the cart). */
  function addSafe() {
    void addToCart(
      (data?.findings ?? [])
        .filter((f) => f.safety === 'safe')
        .flatMap((f) => f.items.map((i) => i.id)),
    );
  }

  async function hide(entry: string, what: string) {
    await api.cleanupIgnore(entry, true);
    reloads++;
    app.toast(`Won't suggest ${what} again`, {
      action: { label: 'Undo', run: () => void unhide(entry) },
    });
  }

  async function unhide(entry: string) {
    await api.cleanupIgnore(entry, false);
    reloads++;
  }

  function hiddenMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuItem[] = (data?.hidden ?? []).map((h) => ({
      label: `Suggest ${h.includes('\\') ? h : (labelOf(h) ?? h)} again`,
      icon: Eye,
      run: () => void unhide(h),
    }));
    app.openMenu(
      { clientX: r.right - 280, clientY: r.bottom + 6, preventDefault() {} } as MouseEvent,
      items,
    );
  }

  const LABELS: Record<string, string> = {
    node_modules: 'node_modules folders',
    'rust-target': 'Rust build output',
    'python-cache': 'Python caches',
    'python-venv': 'Python virtual environments',
    'package-cache': 'package manager caches',
    temp: 'old temporary files',
    'browser-cache': 'browser caches',
    'crash-dumps': 'crash dumps',
    'old-downloads': 'old downloads',
    'recycle-bin': 'the Recycle Bin',
    'windows-update': 'Windows Update downloads',
    'virtual-disks': 'virtual disks',
    'system-files': 'hibernation and page files',
  };
  function labelOf(key: string) {
    return LABELS[key];
  }

  async function emptyBin() {
    try {
      await api.emptyRecycleBin();
      reloads++;
    } catch (e) {
      app.toast(`${e}`, { tone: 'error' });
    }
  }

  /** Path shown under the scope: drop the scope's own prefix. */
  function short(path: string) {
    const base = data?.scope.replace(/\\$/, '') ?? '';
    return path.toLowerCase().startsWith(base.toLowerCase() + '\\')
      ? path.slice(base.length + 1)
      : path;
  }
</script>

<div class="flex h-full flex-col" in:fade={{ duration: 160 }}>
  <div
    class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-4 py-2.5 text-[12px]"
  >
    <div class="flex min-w-0 rounded-lg bg-panel-2 p-0.5">
      <button
        class="flex items-center gap-1.5 rounded-md px-2.5 py-1 transition-colors {!inFolder ||
        focus === null
          ? 'bg-panel text-fg shadow-sm'
          : 'text-fg-subtle hover:text-fg-muted'}"
        onclick={() => (inFolder = false)}
        ><HardDrive class="size-3.5" /> Whole {letter}: drive</button
      >
      <button
        class="flex min-w-0 items-center gap-1.5 rounded-md px-2.5 py-1 transition-colors disabled:opacity-40 {inFolder &&
        focus !== null
          ? 'bg-panel text-fg shadow-sm'
          : 'text-fg-subtle hover:text-fg-muted'}"
        disabled={focus === null}
        title={focus === null ? 'Open a folder in Browse first' : data?.scope}
        onclick={() => (inFolder = true)}
        ><FolderOpen class="size-3.5 shrink-0" /><span class="max-w-[220px] truncate"
          >{focus === null ? 'Open folder' : `Only ${folderName}`}</span
        ></button
      >
    </div>
    {#if data}
      <span class="tabular text-fg-subtle">
        <span class="text-ok">{bytes(data.safe)}</span> safe ·
        <span class="text-warn">{bytes(data.review)}</span> to check
      </span>
    {/if}
    <div class="ml-auto flex items-center gap-2">
      {#if data?.hidden.length}
        <button
          class="pressable flex h-7 items-center gap-1.5 rounded-lg px-2 text-fg-subtle hover:bg-hover hover:text-fg"
          onclick={hiddenMenu}><EyeOff class="size-3.5" /> {data.hidden.length} hidden</button
        >
      {/if}
      {#if picked.size}
        <button
          class="pressable flex h-7 items-center gap-1.5 rounded-lg bg-accent px-2.5 font-medium text-accent-fg hover:brightness-110"
          in:fade={{ duration: 120 }}
          onclick={() => addToCart([...picked])}
          ><ShoppingCart class="size-3.5" /> Add {count(picked.size)} to cart · {bytes(
            pickedSize,
          )}</button
        >
      {:else if data?.safe}
        <button
          class="pressable flex h-7 items-center gap-1.5 rounded-lg border border-line px-2.5 text-fg-muted hover:border-line-strong hover:text-fg"
          title="Put everything marked safe in the cleanup cart, to review and delete there"
          onclick={addSafe}><ShoppingCart class="size-3.5" /> Add all safe to cart</button
        >
      {/if}
    </div>
  </div>

  {#if loading}
    <div class="grid flex-1 place-items-center text-fg-subtle">Looking for things to clean up…</div>
  {:else if data && !data.findings.length}
    <div class="grid flex-1 place-items-center text-center text-fg-subtle">
      <div>
        <Sparkles class="mx-auto mb-2 size-6 opacity-60" />
        Nothing obvious to clean up here.
      </div>
    </div>
  {:else if data}
    <div class="min-h-0 flex-1 overflow-y-auto px-4 py-3">
      <div class="mx-auto flex max-w-[980px] flex-col gap-3">
        {#each data.findings as f (f.key)}
          {@const b = badge[f.safety]}
          {@const items = open[f.key] ? f.items : f.items.slice(0, SHOWN)}
          <section
            class="card rounded-xl border border-line bg-panel"
            transition:slide={{ duration: 180 }}
          >
            <header class="flex items-start gap-3 px-4 pt-3 pb-2">
              {#if f.safety !== 'info'}
                <input
                  type="checkbox"
                  class="mt-1 size-3.5 shrink-0 accent-[var(--accent)]"
                  checked={allPicked(f)}
                  indeterminate={!allPicked(f) && f.items.some((i) => picked.has(i.id))}
                  aria-label="Select all {f.label}"
                  onchange={() => toggleAll(f)}
                />
              {/if}
              <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="font-medium">{f.label}</span>
                  <span class="rounded-full px-2 py-0.5 text-[10.5px] font-medium {b.cls}"
                    >{b.label}</span
                  >
                </div>
                <div class="mt-0.5 text-[12px] text-fg-subtle">{f.why}</div>
              </div>
              <div class="flex shrink-0 items-center gap-1">
                <span class="tabular mr-1 text-[14px] font-semibold">{bytes(f.alloc)}</span>
                {#if f.key === 'recycle-bin'}
                  <button
                    class="pressable flex h-7 items-center gap-1.5 rounded-lg border border-line px-2.5 text-[12px] text-fg-muted hover:border-line-strong hover:text-fg"
                    onclick={emptyBin}><Trash2 class="size-3.5" /> Empty…</button
                  >
                {:else if f.key === 'windows-update'}
                  <button
                    class="pressable flex h-7 items-center gap-1.5 rounded-lg border border-line px-2.5 text-[12px] text-fg-muted hover:border-line-strong hover:text-fg"
                    onclick={() =>
                      api.diskCleanup().catch((e) => app.toast(`${e}`, { tone: 'error' }))}
                    ><Wrench class="size-3.5" /> Disk Cleanup</button
                  >
                {/if}
                <button
                  class="pressable grid size-7 place-items-center rounded-lg text-fg-subtle hover:bg-hover hover:text-fg"
                  title="Don't suggest {labelOf(f.key) ?? f.label} again"
                  aria-label="Don't suggest these again"
                  onclick={() => hide(f.key, labelOf(f.key) ?? f.label)}
                  ><EyeOff class="size-3.5" /></button
                >
              </div>
            </header>
            <div class="px-2 pb-2">
              {#each items as it (it.id)}
                <div
                  class="item group flex items-center gap-3 rounded-lg px-2 py-1 hover:bg-hover"
                  class:selected={picked.has(it.id)}
                >
                  {#if f.safety !== 'info'}
                    <input
                      type="checkbox"
                      class="size-3.5 shrink-0 accent-[var(--accent)]"
                      checked={picked.has(it.id)}
                      aria-label="Select {it.path}"
                      onchange={() => toggle(it.id)}
                    />
                  {/if}
                  <button
                    class="mono min-w-0 flex-1 truncate text-left text-[12px] text-fg-muted group-hover:text-fg"
                    title="{it.path} — show in Browse"
                    onclick={() => app.goTo(letter, it.id)}>{short(it.path)}</button
                  >
                  <span class="tabular shrink-0 text-[11px] text-fg-subtle">{ago(it.mtime)}</span>
                  <span class="tabular w-20 shrink-0 text-right">{bytes(it.alloc)}</span>
                  <span class="go flex shrink-0 items-center">
                    <button
                      class="grid size-6 place-items-center rounded-md text-fg-subtle hover:bg-panel-2 hover:text-fg"
                      title="Show in Browse"
                      aria-label="Show in Browse"
                      onclick={() => app.goTo(letter, it.id)}><Crosshair class="size-3.5" /></button
                    >
                    <button
                      class="grid size-6 place-items-center rounded-md text-fg-subtle hover:bg-panel-2 hover:text-fg"
                      title="Show in Explorer"
                      aria-label="Show in Explorer"
                      onclick={() => api.showInExplorer(it.path)}
                      ><FolderSearch class="size-3.5" /></button
                    >
                    <button
                      class="grid size-6 place-items-center rounded-md text-fg-subtle hover:bg-panel-2 hover:text-fg"
                      title="Don't suggest this again"
                      aria-label="Don't suggest this again"
                      onclick={() => hide(it.path, short(it.path))}
                      ><EyeOff class="size-3.5" /></button
                    >
                  </span>
                </div>
              {/each}
              {#if f.items.length > SHOWN}
                <button
                  class="mx-2 mt-1 text-[12px] text-accent hover:underline"
                  onclick={() => (open[f.key] = !open[f.key])}
                  >{open[f.key] ? 'Show fewer' : `Show all ${count(f.items.length)}`}</button
                >
              {/if}
            </div>
          </section>
        {/each}
        <p class="px-1 pb-2 text-[11.5px] text-fg-subtle">
          Ticked items go to the cleanup cart, where you review them and send them to the Recycle
          Bin. Nothing is deleted from here.
        </p>
      </div>
    </div>
  {/if}
</div>

<style>
  .item :global(.go) {
    opacity: 0;
    transition: opacity 120ms;
  }
  .item:hover :global(.go) {
    opacity: 1;
  }
  .item.selected {
    background: var(--accent-soft);
  }
</style>
