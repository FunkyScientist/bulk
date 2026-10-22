<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';

  let { open = $bindable(false) }: { open: boolean } = $props();

  const groups = [
    {
      title: 'Anywhere',
      keys: [
        ['Ctrl K', 'Search and commands'],
        ['Ctrl R', 'Rescan open folder'],
        ['Ctrl Shift R', 'Rescan whole drive'],
        ['Ctrl B', 'Collapse sidebar'],
        ['Ctrl E', 'Show selection in Explorer'],
        ['Ctrl 1 / 2', 'Treemap / sunburst'],
        ['?', 'This list'],
      ],
    },
    {
      title: 'File list',
      keys: [
        ['↑ ↓', 'Move selection'],
        ['→', 'Open folder / first item'],
        ['←', 'Close folder / go to parent'],
        ['Enter', 'Show folder in the chart'],
        ['Shift Enter', 'Show in Explorer'],
        ['Del', 'Add to / remove from cleanup cart'],
        ['Home / End', 'First / last row'],
      ],
    },
    {
      title: 'Chart',
      keys: [
        ['Click', 'Select'],
        ['Double-click', 'Zoom in'],
        ['Right-click', 'Actions (empty space: zoom out)'],
        ['Backspace', 'Zoom out'],
      ],
    },
  ];
</script>

{#if open}
  <div
    class="fixed inset-0 z-40 bg-black/30"
    transition:fade={{ duration: 140 }}
    onclick={() => (open = false)}
    role="presentation"
  ></div>
  <div
    class="panel fixed top-1/2 left-1/2 z-50 w-[min(640px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-line-strong p-5"
    transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
    role="dialog"
    aria-label="Keyboard shortcuts"
  >
    <div class="mb-4 text-[15px] font-semibold tracking-tight">Keyboard shortcuts</div>
    <div class="grid grid-cols-1 gap-5 sm:grid-cols-3">
      {#each groups as g (g.title)}
        <div>
          <div class="mb-2 text-[11px] font-medium tracking-wide text-fg-subtle uppercase">
            {g.title}
          </div>
          {#each g.keys as [k, label] (k)}
            <div class="flex items-center justify-between gap-2 py-1 text-[12px]">
              <span class="text-fg-muted">{label}</span>
              <kbd class="kbd shrink-0">{k}</kbd>
            </div>
          {/each}
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .panel {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(24px);
  }
</style>
