<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import Search from '@lucide/svelte/icons/search';
  import Sun from '@lucide/svelte/icons/sun';
  import Moon from '@lucide/svelte/icons/moon';
  import Monitor from '@lucide/svelte/icons/monitor';
  import { app } from '$lib/app.svelte';

  let { onSearch }: { onSearch?: () => void } = $props();

  const win = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    win.isMaximized().then((m) => (maximized = m));
    const un = win.onResized(async () => (maximized = await win.isMaximized()));
    return () => void un.then((f) => f());
  });

  const themeLabel = $derived(
    app.theme === 'system'
      ? 'Theme: system'
      : app.theme === 'dark'
        ? 'Theme: dark'
        : 'Theme: light',
  );
</script>

<header class="relative flex h-10 shrink-0 items-center" data-tauri-drag-region>
  <div class="pointer-events-none flex items-center gap-2 pl-4" data-tauri-drag-region>
    <svg viewBox="0 0 1024 1024" class="size-4" aria-hidden="true">
      <rect x="64" y="64" width="896" height="896" rx="208" fill="var(--panel-2)" />
      <rect x="208" y="208" width="368" height="608" rx="56" fill="var(--accent)" />
      <rect x="608" y="208" width="208" height="288" rx="48" fill="#3fd0c9" />
      <rect x="608" y="528" width="208" height="128" rx="40" fill="#f5b544" />
      <rect x="608" y="688" width="208" height="128" rx="32" fill="#ff6b8b" />
    </svg>
    <span class="text-[12.5px] font-medium tracking-tight text-fg-muted">Bulk</span>
  </div>

  <div class="absolute inset-x-0 flex justify-center pointer-events-none" data-tauri-drag-region>
    <button
      class="pressable pointer-events-auto flex h-7 w-[min(420px,40vw)] items-center gap-2 rounded-lg border border-line bg-panel px-2.5 text-[12.5px] text-fg-subtle hover:border-line-strong hover:text-fg-muted"
      onclick={() => onSearch?.()}
    >
      <Search class="size-3.5" />
      <span>Search files and folders</span>
      <kbd class="ml-auto rounded border border-line px-1.5 font-sans text-[10.5px] text-fg-subtle"
        >Ctrl K</kbd
      >
    </button>
  </div>

  <div class="ml-auto flex h-full items-center">
    <button
      class="pressable mr-1 grid size-7 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
      title={themeLabel}
      aria-label={themeLabel}
      onclick={() => app.cycleTheme()}
    >
      {#key app.theme}
        <span class="theme-icon grid place-items-center">
          {#if app.theme === 'system'}<Monitor
              class="size-3.5"
            />{:else if app.theme === 'dark'}<Moon class="size-3.5" />{:else}<Sun
              class="size-3.5"
            />{/if}
        </span>
      {/key}
    </button>

    <button class="winbtn" aria-label="Minimize" onclick={() => win.minimize()}>
      <svg viewBox="0 0 10 10"><path d="M0 5h10" /></svg>
    </button>
    <button
      class="winbtn"
      aria-label={maximized ? 'Restore' : 'Maximize'}
      onclick={() => win.toggleMaximize()}
    >
      {#if maximized}
        <svg viewBox="0 0 10 10"><path d="M2 0h8v8M0 2h8v8H0z" /></svg>
      {:else}
        <svg viewBox="0 0 10 10"><path d="M0 0h10v10H0z" /></svg>
      {/if}
    </button>
    <button class="winbtn close" aria-label="Close" onclick={() => win.close()}>
      <svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" /></svg>
    </button>
  </div>
</header>

<style>
  .winbtn {
    display: grid;
    place-items: center;
    width: 46px;
    height: 100%;
    color: var(--fg-muted);
    transition:
      background-color 120ms ease,
      color 120ms ease;
  }
  .winbtn svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
  }
  .winbtn:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .winbtn.close:hover {
    background: #c42b1c;
    color: white;
  }
  .theme-icon {
    animation: spin-in 320ms var(--ease-spring);
  }
  @keyframes spin-in {
    from {
      transform: rotate(-90deg) scale(0.6);
      opacity: 0;
    }
  }
</style>
