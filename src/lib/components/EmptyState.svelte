<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import ScanSearch from '@lucide/svelte/icons/scan-search';
  import { app } from '$lib/app.svelte';
  import type { Drive } from '$lib/api';
  import { bytes } from '$lib/format';

  let { drive }: { drive: Drive } = $props();
  const error = $derived(app.scans[drive.letter]?.error);
</script>

<div class="grid h-full place-items-center" in:fade={{ duration: 180 }}>
  <div
    class="flex max-w-[360px] flex-col items-center text-center"
    in:scale={{ start: 0.97, duration: 320, easing: cubicOut }}
  >
    <div class="mb-5 grid size-14 place-items-center rounded-2xl bg-panel-2 text-fg-muted">
      <ScanSearch class="size-6" />
    </div>
    <h2 class="text-[17px] font-semibold tracking-tight">
      {drive.letter}: {drive.label || 'Local disk'}
    </h2>
    <p class="mt-1.5 leading-relaxed text-fg-muted">
      {#if error}
        The last scan didn't finish: {error}
      {:else if drive.kind === 'virtual'}
        This is a cloud drive. Scanning it lists files that may only exist online, so it isn't
        scanned automatically.
      {:else}
        {bytes(drive.total - drive.free)} used. Scan it to see what's taking up space.
      {/if}
    </p>
    <button
      class="pressable mt-5 h-8 rounded-lg bg-accent px-4 text-[12.5px] font-medium text-accent-fg hover:brightness-110"
      onclick={() => app.scan(drive.letter)}
    >
      {error ? 'Try again' : `Scan ${drive.letter}:`}
    </button>
  </div>
</div>
