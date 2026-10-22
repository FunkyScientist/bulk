<script lang="ts">
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import { api, type Crumb } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import Treemap from './Treemap.svelte';

  let { letter }: { letter: string } = $props();

  let map: Treemap | undefined = $state();
  let crumbs = $state<Crumb[]>([]);
  const focus = $derived(app.focus[letter] ?? null);

  $effect(() => {
    const f = focus;
    void app.rowsVersion;
    api.ancestors(letter, f ?? 0).then((c) => (crumbs = c));
  });

  function go(c: Crumb, i: number) {
    if (i === crumbs.length - 1) return;
    map?.zoomOut(i === 0 ? null : c.id);
  }
</script>

<div class="flex h-full flex-col">
  <div class="flex h-9 shrink-0 items-center gap-1 border-b border-line px-2 text-[12px]">
    <button
      class="pressable grid size-6 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg disabled:opacity-30"
      disabled={focus === null}
      title="Up one level (right-click the map)"
      aria-label="Up one level"
      onclick={() => map?.zoomOut()}
    >
      <ArrowUp class="size-3.5" />
    </button>
    <nav class="flex min-w-0 items-center overflow-hidden">
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
  </div>
  <div class="min-h-0 flex-1 p-1.5">
    <Treemap bind:this={map} {letter} />
  </div>
</div>
