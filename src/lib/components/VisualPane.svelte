<script lang="ts">
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import LayoutGrid from '@lucide/svelte/icons/layout-grid';
  import PieChart from '@lucide/svelte/icons/chart-pie';
  import { api, type Crumb } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import Treemap from './Treemap.svelte';
  import Sunburst from './Sunburst.svelte';

  let { letter }: { letter: string } = $props();

  let view: { zoomOut: (to?: number | null) => void } | undefined = $state();
  let crumbs = $state<Crumb[]>([]);
  const focus = $derived(app.focus[letter] ?? null);

  $effect(() => {
    const f = focus;
    void app.rowsVersion;
    api.ancestors(letter, f ?? 0).then((c) => (crumbs = c));
  });

  function go(c: Crumb, i: number) {
    if (i === crumbs.length - 1) return;
    view?.zoomOut(i === 0 ? null : c.id);
  }

  export function zoomOut() {
    view?.zoomOut();
  }

  const modes = [
    { key: 'treemap', label: 'Treemap', icon: LayoutGrid },
    { key: 'sunburst', label: 'Sunburst', icon: PieChart },
  ] as const;
</script>

<div class="flex h-full flex-col">
  <div class="flex h-9 shrink-0 items-center gap-1 border-b border-line px-2 text-[12px]">
    <button
      class="pressable grid size-6 shrink-0 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg disabled:opacity-30"
      disabled={focus === null}
      title="Up one level (right-click the chart)"
      aria-label="Up one level"
      onclick={() => view?.zoomOut()}
    >
      <ArrowUp class="size-3.5" />
    </button>
    <nav class="flex min-w-0 flex-1 items-center overflow-hidden">
      {#each crumbs as c, i (c.id)}
        <span
          class="flex min-w-0 shrink items-center"
          in:fly={{ x: -6, duration: 220, easing: cubicOut }}
        >
          {#if i > 0}<ChevronRight class="size-3 shrink-0 text-fg-subtle" />{/if}
          <button
            class="truncate rounded-md px-1.5 py-0.5 hover:bg-hover {i === crumbs.length - 1
              ? 'font-medium text-fg'
              : 'text-fg-muted hover:text-fg'}"
            onclick={() => go(c, i)}
          >
            {i === 0 ? `${c.name}\\` : c.name}
          </button>
        </span>
      {/each}
    </nav>

    <div class="seg relative flex shrink-0 rounded-lg bg-panel-2 p-0.5">
      <span class="thumb" style="transform: translateX({app.visual === 'treemap' ? 0 : 100}%)"
      ></span>
      {#each modes as m (m.key)}
        {@const Icon = m.icon}
        <button
          class="relative z-10 flex h-6 w-[30px] items-center justify-center rounded-md transition-colors {app.visual ===
          m.key
            ? 'text-fg'
            : 'text-fg-subtle hover:text-fg-muted'}"
          title={m.label}
          aria-label={m.label}
          aria-pressed={app.visual === m.key}
          onclick={() => app.setVisual(m.key)}
        >
          <Icon class="size-3.5" />
        </button>
      {/each}
    </div>
  </div>
  <div class="min-h-0 flex-1 p-1.5">
    {#key app.visual}
      <div class="h-full" in:fly={{ y: 6, duration: 240, easing: cubicOut }}>
        {#if app.visual === 'treemap'}
          <Treemap bind:this={view} {letter} />
        {:else}
          <Sunburst bind:this={view} {letter} />
        {/if}
      </div>
    {/key}
  </div>
</div>

<style>
  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 30px;
    height: 24px;
    border-radius: 6px;
    background: var(--panel);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.25),
      inset 0 0 0 1px var(--line);
    transition: transform 240ms var(--ease-spring);
  }
</style>
