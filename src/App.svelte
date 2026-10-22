<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  onMount(() => {
    getCurrentWindow().show();
  });
</script>

<main class="grid h-full place-items-center text-neutral-200">
  <h1 class="text-2xl font-semibold tracking-tight">Bulk</h1>
</main>
