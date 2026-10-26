<script lang="ts">
  import { untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import Search from '@lucide/svelte/icons/search';
  import { api, type TypesResult } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, count } from '$lib/format';

  let { letter }: { letter: string } = $props();

  /** Colours per kind, matching the chart where the groups overlap. */
  const colors: Record<string, { dark: string; light: string }> = {
    video: { dark: '#ff6b8b', light: '#f0506f' },
    image: { dark: '#f5b544', light: '#e0a020' },
    audio: { dark: '#c38bff', light: '#a865f5' },
    document: { dark: '#5fa8ff', light: '#3b88f0' },
    spreadsheet: { dark: '#4fd18b', light: '#22a861' },
    presentation: { dark: '#ff8f6b', light: '#ef6a40' },
    archive: { dark: '#ff9a5a', light: '#f27c35' },
    program: { dark: '#9d92ff', light: '#7a6cf2' },
    system: { dark: '#8a93b8', light: '#6c76a0' },
    code: { dark: '#3fd0c9', light: '#16b2aa' },
    diskimage: { dark: '#f37bc4', light: '#e055a8' },
    database: { dark: '#8fb3a8', light: '#6f9a8d' },
    design: { dark: '#e8d26a', light: '#c9ad2c' },
    font: { dark: '#b0a4ff', light: '#8e80f0' },
    email: { dark: '#6fc3ff', light: '#3aa5ee' },
    log: { dark: '#a3a9b8', light: '#8b92a3' },
    other: { dark: '#7d8496', light: '#9aa1b2' },
    none: { dark: '#5c6273', light: '#b7bcc8' },
  };
  const color = (kind: string) => (colors[kind] ?? colors.other)[app.resolvedTheme];

  let mode = $state<'kind' | 'ext'>('kind');
  // Start with the folder open in Browse, when there is one.
  let inFolder = $state(untrack(() => app.focus[letter] ?? null) !== null);
  let folderName = $state('');
  let data = $state<TypesResult | null>(null);
  let loading = $state(false);
  let showAll = $state(false);

  const focus = $derived(app.focus[letter] ?? null);
  $effect(() => {
    const id = focus;
    if (id === null) folderName = '';
    else api.nodeInfo(letter, id).then((i) => (folderName = i.name));
  });

  // Reload when the scope changes or the drive updates (live changes, rescans).
  let seq = 0;
  $effect(() => {
    const folder = inFolder ? focus : null;
    void app.rowsVersion;
    const mine = ++seq;
    loading = data === null;
    const t = setTimeout(async () => {
      const r = await api.types(letter, folder).catch(() => null);
      if (mine !== seq) return;
      data = r;
      loading = false;
    }, 60);
    return () => clearTimeout(t);
  });

  type Row = {
    id: string;
    label: string;
    kind: string;
    files: number;
    alloc: number;
    exts?: string[];
    query: string;
  };

  const rows = $derived.by((): Row[] => {
    if (!data) return [];
    if (mode === 'kind')
      return data.kinds.map((k) => ({
        id: k.key,
        label: k.label,
        kind: k.key,
        files: k.files,
        alloc: k.alloc,
        exts: k.exts,
        query: k.key === 'none' ? 'ext:none' : `kind:${k.key}`,
      }));
    return data.exts.map((e) => ({
      id: e.ext,
      label: e.ext ? `.${e.ext}` : 'No extension',
      kind: e.kind,
      files: e.files,
      alloc: e.alloc,
      query: `ext:${e.ext || 'none'}`,
    }));
  });
  const LIMIT = 200;
  const shown = $derived(showAll ? rows : rows.slice(0, LIMIT));
  const max = $derived(rows[0]?.alloc ?? 0);

  /** The strip at the top: the biggest groups, the rest merged. */
  const strip = $derived.by(() => {
    if (!data || !data.alloc) return [];
    const top = rows.slice(0, 12);
    const rest = data.alloc - top.reduce((s, r) => s + r.alloc, 0);
    const out = top.map((r) => ({ label: r.label, kind: r.kind, alloc: r.alloc }));
    if (rest > 0) out.push({ label: 'Everything else', kind: 'none', alloc: rest });
    return out.filter((s) => s.alloc / data!.alloc > 0.002);
  });

  function open(r: Row) {
    app.openSearch(`${r.query} sort:size`, inFolder && focus !== null ? 'folder' : 'drive');
  }
</script>

<div class="flex h-full flex-col" in:fade={{ duration: 160 }}>
  <div
    class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-4 py-2.5 text-[12px]"
  >
    <div class="flex rounded-lg bg-panel-2 p-0.5">
      {#each [{ k: 'kind', l: 'By type' }, { k: 'ext', l: 'By extension' }] as const as o (o.k)}
        <button
          class="rounded-md px-2.5 py-1 transition-colors {mode === o.k
            ? 'bg-panel text-fg shadow-sm'
            : 'text-fg-subtle hover:text-fg-muted'}"
          aria-pressed={mode === o.k}
          onclick={() => {
            mode = o.k;
            showAll = false;
          }}>{o.l}</button
        >
      {/each}
    </div>
    <div class="flex min-w-0 rounded-lg bg-panel-2 p-0.5">
      <button
        class="flex items-center gap-1.5 rounded-md px-2.5 py-1 transition-colors {!inFolder ||
        focus === null
          ? 'bg-panel text-fg shadow-sm'
          : 'text-fg-subtle hover:text-fg-muted'}"
        onclick={() => (inFolder = false)}><HardDrive class="size-3.5" /> Whole {letter}: drive</button
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
      <span class="tabular ml-auto min-w-0 truncate text-fg-subtle">
        {count(data.files)} files · {bytes(data.alloc)} in
        <span class="mono" title={data.scope}>{data.scope}</span>
      </span>
    {/if}
  </div>

  {#if loading}
    <div class="grid flex-1 place-items-center text-fg-subtle">Adding up file types…</div>
  {:else if data && !data.files}
    <div class="grid flex-1 place-items-center text-fg-subtle">No files here.</div>
  {:else if data}
    <div class="shrink-0 px-4 pt-3 pb-2">
      <div class="flex h-3 overflow-hidden rounded-full bg-panel-2">
        {#each strip as s, i (s.label + i)}
          <div
            class="strip h-full"
            style="width: {(s.alloc / data.alloc) * 100}%; background: {color(s.kind)}"
            title="{s.label} · {bytes(s.alloc)}"
          ></div>
        {/each}
      </div>
    </div>

    <div class="grid shrink-0 grid-cols-[1fr_90px_90px_160px] gap-3 px-4 py-1.5 text-[11px] text-fg-subtle">
      <span>{mode === 'kind' ? 'Type' : 'Extension'}</span>
      <span class="text-right">Files</span>
      <span class="text-right">On disk</span>
      <span>Share</span>
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
      {#each shown as r (r.id)}
        <button
          class="row grid w-full grid-cols-[1fr_90px_90px_160px] items-center gap-3 rounded-lg px-2 py-1.5 text-left hover:bg-hover"
          title="Find these files"
          onclick={() => open(r)}
        >
          <span class="flex min-w-0 items-center gap-2.5">
            <span class="size-2.5 shrink-0 rounded-sm" style="background: {color(r.kind)}"></span>
            <span class="min-w-0">
              <span class="block truncate {mode === 'ext' ? 'font-mono text-[12.5px]' : ''}"
                >{r.label}</span
              >
              {#if r.exts?.length}
                <span class="block truncate font-mono text-[10.5px] text-fg-subtle"
                  >{r.exts.slice(0, 10).join(' ')}{r.exts.length > 10
                    ? ` +${r.exts.length - 10}`
                    : ''}</span
                >
              {/if}
            </span>
            <Search class="go size-3.5 shrink-0 text-fg-subtle" />
          </span>
          <span class="tabular text-right text-fg-muted">{count(r.files)}</span>
          <span class="tabular text-right">{bytes(r.alloc)}</span>
          <span class="flex items-center gap-2">
            <span class="h-1.5 flex-1 overflow-hidden rounded-full bg-panel-2">
              <span
                class="block h-full rounded-full"
                style="width: {max ? (r.alloc / max) * 100 : 0}%; background: {color(r.kind)}"
              ></span>
            </span>
            <span class="tabular w-11 text-right text-[11px] text-fg-subtle"
              >{data.alloc ? ((r.alloc / data.alloc) * 100).toFixed(1) : '0.0'}%</span
            >
          </span>
        </button>
      {/each}
      {#if rows.length > shown.length}
        <button
          class="mx-2 mt-1 text-[12px] text-accent hover:underline"
          onclick={() => (showAll = true)}>Show all {count(rows.length)} extensions</button
        >
      {/if}
    </div>
  {/if}
</div>

<style>
  .strip + .strip {
    box-shadow: inset 1px 0 0 var(--panel);
  }
  .row :global(.go) {
    opacity: 0;
    transition: opacity 120ms;
  }
  .row:hover :global(.go) {
    opacity: 1;
  }
</style>
