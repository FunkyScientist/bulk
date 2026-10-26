<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import X from '@lucide/svelte/icons/x';
  import Zap from '@lucide/svelte/icons/zap';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import { api, type Settings } from '$lib/api';
  import { app, type Layout, type ThemePref } from '$lib/app.svelte';
  import Columns2 from '@lucide/svelte/icons/columns-2';
  import Rows2 from '@lucide/svelte/icons/rows-2';
  import { bytes, count } from '$lib/format';
  import { resolve } from '$lib/theme';
  import Check from '@lucide/svelte/icons/check';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import type { TemplateInfo } from '$lib/api';

  let { open = $bindable(false) }: { open: boolean } = $props();

  let s = $state<Settings | null>(null);
  let storage = $state<{ path: string; bytes: number; files: number } | null>(null);
  let version = $state('');
  let busy = $state(false);

  $effect(() => {
    if (!open) return;
    api.settingsGet().then((v) => (s = v));
    api.snapshotStorage().then((v) => (storage = v));
    api.appVersion().then((v) => (version = v));
    void app.loadTemplates();
  });

  /** What a template looks like in the current light/dark mode, for its card. */
  function preview(t: TemplateInfo) {
    const base = app.templates.find((x) => x.id === 'default')?.data;
    const m = resolve(t.data, base, app.resolvedTheme);
    const f = m.files;
    return {
      bg: m.ui['app-bg'],
      panel: m.ui['panel-2'],
      fg: m.ui.fg,
      muted: m.ui['fg-subtle'],
      accent: m.ui.accent,
      accentFg: m.ui['accent-fg'],
      swatches: [f.folder, f.video, f.image, f.document, f.code, f.archive],
    };
  }

  async function reload() {
    await app.loadTemplates();
    app.toast(`${app.templates.length} colour templates`, { ms: 1500 });
  }

  async function save(patch: Partial<Settings>) {
    if (!s) return;
    try {
      // The template is picked through `app`, so take it from there.
      s = await api.settingsSet({ ...s, ...patch, template: app.templateId });
    } catch (e) {
      app.toast(`Couldn't save settings: ${e}`, { tone: 'error' });
    }
  }

  async function allow() {
    busy = true;
    await app.allowFastMode();
    busy = false;
  }

  const needsPermission = $derived(app.helper === 'notInstalled' || app.helper === 'outdated');

  const themes: { k: ThemePref; l: string }[] = [
    { k: 'system', l: 'System' },
    { k: 'dark', l: 'Dark' },
    { k: 'light', l: 'Light' },
  ];
  const layouts: { k: Layout; l: string; I: typeof Columns2 }[] = [
    { k: 'columns', l: 'Side by side', I: Columns2 },
    { k: 'rows', l: 'Stacked', I: Rows2 },
  ];
  const keeps = [7, 14, 30, 60, 90];
</script>

{#snippet toggle(on: boolean, onchange: (v: boolean) => void, label: string, disabled = false)}
  <button
    class="switch relative h-5 w-9 shrink-0 rounded-full transition-colors {on
      ? 'bg-accent'
      : 'bg-[var(--switch-off)]'}"
    role="switch"
    aria-checked={on}
    aria-label={label}
    {disabled}
    onclick={() => onchange(!on)}
  >
    <span class="knob absolute top-0.5 left-0.5 size-4 rounded-full bg-[var(--switch-knob)]" class:on></span>
  </button>
{/snippet}

{#snippet segmented<T extends string | number>(
  opts: { k: T; l: string }[],
  value: T,
  onpick: (v: T) => void,
)}
  <div class="flex rounded-lg bg-panel-2 p-0.5 text-[12px]">
    {#each opts as o (o.k)}
      <button
        class="rounded-md px-2.5 py-1 transition-colors {value === o.k
          ? 'bg-panel text-fg shadow-sm'
          : 'text-fg-subtle hover:text-fg-muted'}"
        onclick={() => onpick(o.k)}>{o.l}</button
      >
    {/each}
  </div>
{/snippet}

{#if open}
  <div
    class="fixed inset-0 z-40 bg-[var(--backdrop)]"
    transition:fade={{ duration: 140 }}
    onclick={() => (open = false)}
    role="presentation"
  ></div>
  <div
    class="panel fixed top-1/2 left-1/2 z-50 flex max-h-[86vh] w-[min(560px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border border-line-strong"
    transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
    role="dialog"
    aria-label="Settings"
  >
    <div class="flex items-center border-b border-line px-5 py-3.5">
      <div class="text-[15px] font-semibold tracking-tight">Settings</div>
      <button
        class="pressable ml-auto grid size-7 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
        aria-label="Close"
        onclick={() => (open = false)}><X class="size-4" /></button
      >
    </div>

    <div class="min-h-0 flex-1 space-y-6 overflow-y-auto px-5 py-4">
      <section>
        <h3 class="section">Appearance</h3>
        <div class="row">
          <div>Theme</div>
          {@render segmented(themes, app.theme, (v) => app.setTheme(v))}
        </div>
        <div class="row">
          <div>
            <div>Layout</div>
            <div class="sub">Put the chart beside the file list, or below it.</div>
          </div>
          <div class="flex rounded-lg bg-panel-2 p-0.5 text-[12px]">
            {#each layouts as o (o.k)}
              <button
                class="flex items-center gap-1.5 rounded-md px-2.5 py-1 transition-colors {app.layout ===
                o.k
                  ? 'bg-panel text-fg shadow-sm'
                  : 'text-fg-subtle hover:text-fg-muted'}"
                aria-pressed={app.layout === o.k}
                onclick={() => app.setLayout(o.k)}><o.I class="size-3.5" />{o.l}</button
              >
            {/each}
          </div>
        </div>
        <div class="mt-3">
          <div class="flex items-center">
            <div>
              <div>Colours</div>
              <div class="sub">Add your own by dropping a .json file in the templates folder.</div>
            </div>
            <div class="ml-auto flex items-center gap-1">
              <button
                class="pressable flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[12px] text-fg-muted hover:bg-hover hover:text-fg"
                title="Read the templates folder again"
                onclick={reload}><RefreshCw class="size-3.5" /> Reload</button
              >
              <button
                class="pressable flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[12px] text-fg-muted hover:bg-hover hover:text-fg"
                disabled={!app.templatesDir}
                onclick={() => api.openPath(app.templatesDir)}
                ><FolderOpen class="size-3.5" /> Open folder</button
              >
            </div>
          </div>
          <div class="mt-2.5 grid grid-cols-2 gap-2 sm:grid-cols-3">
            {#each app.templates as t (t.id)}
              {@const on = app.template?.id === t.id}
              {#if t.error}
                <div
                  class="flex flex-col gap-1 rounded-xl border border-dashed border-line p-2.5 text-[11.5px]"
                  title={t.error}
                >
                  <span class="flex items-center gap-1.5 font-medium text-fg-muted">
                    <TriangleAlert class="size-3.5 text-warn" />{t.id}.json
                  </span>
                  <span class="line-clamp-3 text-fg-subtle">{t.error}</span>
                </div>
              {:else}
                {@const p = preview(t)}
                <button
                  class="card pressable flex flex-col overflow-hidden rounded-xl border text-left {on
                    ? 'border-accent'
                    : 'border-line hover:border-line-strong'}"
                  aria-pressed={on}
                  title={t.description || t.name}
                  onclick={() => app.setTemplate(t.id)}
                >
                  <span class="relative block h-14 p-2" style="background: {p.bg}">
                    <span class="flex gap-1">
                      <span class="h-9 w-3 rounded" style="background: {p.panel}"></span>
                      <span class="flex flex-1 flex-col gap-1">
                        <span class="h-1.5 w-3/5 rounded-full" style="background: {p.fg}"></span>
                        <span class="h-1.5 w-2/5 rounded-full" style="background: {p.muted}"></span>
                        <span class="mt-auto flex gap-0.5">
                          {#each p.swatches as c, i (i)}
                            <span class="h-3 flex-1 rounded-sm" style="background: {c}"></span>
                          {/each}
                        </span>
                      </span>
                    </span>
                    <span
                      class="absolute top-2 right-2 grid size-4 place-items-center rounded-full"
                      style="background: {p.accent}"
                    >
                      {#if on}<Check class="size-3" style="color: {p.accentFg}" />{/if}
                    </span>
                  </span>
                  <span class="px-2.5 py-1.5">
                    <span class="block truncate text-[12.5px] font-medium">{t.name}</span>
                    <span class="block truncate text-[11px] text-fg-subtle"
                      >{t.description || `${t.id}.json`}</span
                    >
                  </span>
                </button>
              {/if}
            {/each}
          </div>
        </div>
      </section>

      {#if s}
        <section>
          <h3 class="section">Scanning</h3>
          <div class="row">
            <div>
              <div class="flex items-center gap-1.5">
                <Zap class="size-3.5 text-accent" /> Fast mode
              </div>
              <div class="sub">
                {#if !s.fastMode}
                  Off. Scans walk folders one by one, and drives don't update live.
                {:else if needsPermission}
                  Needs Windows' permission to read drives directly.
                {:else}
                  Reads drives directly: scans take seconds and stay up to date live.
                {/if}
              </div>
            </div>
            <div class="flex items-center gap-3">
              {#if s.fastMode && needsPermission}
                <button
                  class="pressable h-7 shrink-0 rounded-lg border border-line px-3 text-[12px] hover:border-line-strong disabled:opacity-50"
                  disabled={busy}
                  onclick={allow}>{busy ? 'Waiting for Windows…' : 'Allow'}</button
                >
              {/if}
              {@render toggle(s.fastMode, (v) => save({ fastMode: v }), 'Fast mode')}
            </div>
          </div>
        </section>

        <section>
          <h3 class="section">Snapshots</h3>
          <div class="row" class:opacity-50={!s.autoSnapshots && app.helper !== 'ready'}>
            <div>
              <div>Automatic snapshots</div>
              <div class="sub">
                {app.helper === 'ready' || s.autoSnapshots
                  ? 'Saved in the background, even when Bulk is closed.'
                  : 'Needs fast mode.'}
              </div>
            </div>
            {@render toggle(
              s.autoSnapshots,
              (v) => save({ autoSnapshots: v }),
              'Automatic snapshots',
              !s.autoSnapshots && app.helper !== 'ready',
            )}
          </div>
          <div class="row" class:opacity-50={!s.autoSnapshots}>
            <div>How often</div>
            {@render segmented(
              [
                { k: 'daily' as const, l: 'Daily' },
                { k: 'weekly' as const, l: 'Weekly' },
              ],
              s.snapshotEvery,
              (v) => save({ snapshotEvery: v }),
            )}
          </div>
          <div class="row">
            <div>
              <div>Keep</div>
              <div class="sub">Older snapshots are removed automatically.</div>
            </div>
            {@render segmented(
              keeps.map((k) => ({ k, l: String(k) })),
              s.historyKeep,
              (v) => save({ historyKeep: v }),
            )}
          </div>
          {#if storage}
            <div class="row">
              <div class="sub tabular">
                {count(storage.files)} files · {bytes(storage.bytes)} used
              </div>
              <button
                class="pressable flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[12px] text-fg-muted hover:bg-hover hover:text-fg"
                onclick={() => api.openPath(storage!.path)}
                ><FolderOpen class="size-3.5" /> Open folder</button
              >
            </div>
          {/if}
        </section>

        <section>
          <h3 class="section">Windows</h3>
          <div class="row">
            <div>
              <div>“Analyze with Bulk” in Explorer</div>
              <div class="sub">Right-click any folder or drive to open it here.</div>
            </div>
            {@render toggle(
              s.explorerMenu,
              (v) => save({ explorerMenu: v }),
              'Explorer menu entry',
            )}
          </div>
        </section>
      {/if}
    </div>

    <div class="flex items-center border-t border-line px-5 py-2 text-[11.5px] text-fg-subtle">
      Bulk {version}
      <button
        class="pressable ml-auto grid size-7 place-items-center rounded-md hover:bg-hover hover:text-fg"
        title="Bulk on GitHub"
        aria-label="Open Bulk on GitHub"
        onclick={() => api.openPath('https://github.com/FunkyScientist/bulk')}
      >
        <svg viewBox="0 0 16 16" class="size-4" fill="currentColor" aria-hidden="true">
          <path
            d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
          />
        </svg>
      </button>
    </div>
  </div>
{/if}

<style>
  .panel {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(24px);
  }
  .section {
    margin-bottom: 6px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--fg-subtle);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 0;
    transition: opacity 160ms ease;
  }
  .sub {
    margin-top: 2px;
    font-size: 11.5px;
    color: var(--fg-subtle);
  }
  .knob {
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
    transition: transform 220ms var(--ease-spring);
  }
  .knob.on {
    transform: translateX(16px);
  }
</style>
