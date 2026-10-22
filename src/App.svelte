<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import TitleBar from '$lib/components/TitleBar.svelte';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import ScanProgress from '$lib/components/ScanProgress.svelte';
  import EmptyState from '$lib/components/EmptyState.svelte';
  import Explorer from '$lib/components/Explorer.svelte';

  $effect(() => {
    document.documentElement.dataset.theme = app.resolvedTheme;
  });

  onMount(async () => {
    try {
      document.documentElement.dataset.backdrop = await api.backdrop();
      await app.init();
    } finally {
      // Hidden windows don't paint (so no requestAnimationFrame); a short timeout lets
      // the first layout settle before the window appears.
      setTimeout(() => getCurrentWindow().show(), 16);
    }
  });

  const drive = $derived(app.drive);
  const scanning = $derived(drive ? app.isScanning(drive.letter) : false);
</script>

<div class="flex h-full flex-col">
  <TitleBar />
  <div class="flex min-h-0 flex-1">
    <Sidebar onRescan={() => drive && app.scan(drive.letter)} />
    <main class="min-w-0 flex-1 pr-2.5 pb-2.5">
      <div class="surface relative h-full overflow-hidden rounded-2xl border border-line bg-panel">
        {#if drive}
          {#key drive.letter}
            {#if !drive.scanned && scanning}
              <ScanProgress {drive} />
            {:else if !drive.scanned}
              <EmptyState {drive} />
            {:else}
              <Explorer {drive} />
            {/if}
          {/key}
        {/if}
      </div>
    </main>
  </div>
</div>

<style>
  .surface {
    box-shadow: var(--shadow);
  }
</style>
