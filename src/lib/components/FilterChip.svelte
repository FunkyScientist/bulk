<script lang="ts">
  import Funnel from '@lucide/svelte/icons/funnel';
  import X from '@lucide/svelte/icons/x';
  import { app } from '$lib/app.svelte';

  let { letter }: { letter: string } = $props();
  const query = $derived(app.filters[letter]?.query ?? null);
</script>

{#if query}
  <span
    class="flex min-w-0 items-center gap-1.5 rounded-md bg-accent-soft py-0.5 pr-0.5 pl-2 text-[12px] text-fg"
    title="The Browse filter applies here too"
  >
    <Funnel class="size-3.5 shrink-0 text-accent" />
    <span class="shrink-0">Only</span>
    <span class="truncate font-mono text-[11.5px]">{query}</span>
    <button
      class="grid size-5 shrink-0 place-items-center rounded text-fg-subtle hover:bg-hover hover:text-fg"
      title="Clear the filter"
      aria-label="Clear the filter"
      onclick={() => app.setFilter(letter, null)}><X class="size-3.5" /></button
    >
  </span>
{/if}
