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
  import CommandPalette from '$lib/components/CommandPalette.svelte';
  import Shortcuts from '$lib/components/Shortcuts.svelte';
  import ContextMenu from '$lib/components/ContextMenu.svelte';
  import Toasts from '$lib/components/Toasts.svelte';
  import CartPanel from '$lib/components/CartPanel.svelte';
  import SettingsDialog from '$lib/components/SettingsDialog.svelte';
  import RescanDialog from '$lib/components/RescanDialog.svelte';

  let palette = $state(false);
  let shortcuts = $state(false);
  let settings = $state(false);
  let rescan = $state(false);

  $effect(() => {
    document.documentElement.dataset.theme = app.resolvedTheme;
  });

  onMount(async () => {
    try {
      document.documentElement.dataset.backdrop = await api.backdrop();
      await app.init();
      const opened = await api.takeOpenPath();
      if (opened) void app.openPath(opened);
    } finally {
      // Hidden windows don't paint (so no requestAnimationFrame); a short timeout lets
      // the first layout settle before the window appears.
      setTimeout(() => getCurrentWindow().show(), 16);
    }
  });

  function typing(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null;
    return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
  }

  function onKey(e: KeyboardEvent) {
    const key = e.key.toLowerCase();
    const ctrl = e.ctrlKey && !e.altKey;
    if (ctrl && (key === 'k' || key === 'f')) {
      e.preventDefault();
      palette = !palette;
      return;
    }
    if (palette || typing(e)) return;
    if (ctrl && key === 'b') {
      e.preventDefault();
      app.toggleSidebar();
    } else if ((ctrl && key === 'r') || e.key === 'F5') {
      e.preventDefault();
      if (drive) app.scan(drive.letter);
    } else if (ctrl && key === 'e' && drive && app.selected !== null) {
      e.preventDefault();
      const letter = drive.letter;
      api.nodePath(letter, app.selected).then((p) => api.showInExplorer(p));
    } else if (ctrl && key === '1') {
      e.preventDefault();
      app.setVisual('treemap');
    } else if (ctrl && key === '2') {
      e.preventDefault();
      app.setVisual('sunburst');
    } else if (e.key === '?') {
      e.preventDefault();
      shortcuts = !shortcuts;
    } else if (e.key === 'Escape' && (shortcuts || settings || rescan || app.cartOpen)) {
      shortcuts = settings = rescan = app.cartOpen = false;
    } else if (e.key === 'Delete' && drive && app.selected !== null && app.selected !== 0) {
      e.preventDefault();
      app.toggleCart(drive.letter, app.selected);
    } else if (e.key === 'Backspace' && drive) {
      e.preventDefault();
      app.zoomOut();
    } else if (e.key.startsWith('Arrow') && document.activeElement === document.body) {
      // Nothing focused: hand arrow keys to the file list.
      const tree = document.querySelector<HTMLElement>('[role="tree"]');
      if (tree) {
        e.preventDefault();
        tree.focus({ preventScroll: true });
        tree.dispatchEvent(new KeyboardEvent('keydown', { key: e.key, bubbles: false }));
      }
    }
  }

  const drive = $derived(app.drive);
  const scanning = $derived(drive ? app.isScanning(drive.letter) : false);
</script>

<svelte:window onkeydown={onKey} />

<div class="flex h-full flex-col">
  <TitleBar onSearch={() => (palette = true)} />
  <div class="flex min-h-0 flex-1">
    <Sidebar onRescan={() => (rescan = true)} onSettings={() => (settings = true)} />
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

<CommandPalette
  bind:open={palette}
  onShortcuts={() => (shortcuts = true)}
  onSettings={() => (settings = true)}
  onRescanDrives={() => (rescan = true)}
/>
<Shortcuts bind:open={shortcuts} />
<CartPanel />
<SettingsDialog bind:open={settings} />
<RescanDialog bind:open={rescan} />
<ContextMenu />
<Toasts />

<style>
  .surface {
    box-shadow: var(--shadow);
  }
</style>
