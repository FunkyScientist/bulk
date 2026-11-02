<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import Search from '@lucide/svelte/icons/search';
  import Sun from '@lucide/svelte/icons/sun';
  import Moon from '@lucide/svelte/icons/moon';
  import Monitor from '@lucide/svelte/icons/monitor';
  import ShoppingCart from '@lucide/svelte/icons/shopping-cart';
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
      <rect x="64" y="64" width="896" height="896" rx="224" fill="var(--fg)" />
      <rect x="232" y="232" width="288" height="560" rx="40" fill="var(--app-bg)" />
      <rect x="232" y="504" width="560" height="288" rx="40" fill="var(--app-bg)" />
      <rect x="560" y="232" width="232" height="232" rx="40" fill="var(--logo-1)" />
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
      class="pressable relative mr-1 grid size-8 place-items-center rounded-md hover:bg-hover hover:text-fg {app
        .cart.length
        ? 'text-fg-muted'
        : 'text-fg-subtle'}"
      title="Cleanup cart"
      aria-label="Cleanup cart, {app.cart.length} items"
      onclick={() => (app.cartOpen = !app.cartOpen)}
    >
      {#key app.cartBump}
        <span class="bump grid place-items-center"><ShoppingCart class="size-4" /></span>
      {/key}
      {#if app.cart.length}
        <span
          class="tabular absolute -top-0.5 -right-0.5 grid h-3.5 min-w-3.5 place-items-center rounded-full bg-accent px-1 text-[9px] font-semibold text-accent-fg"
          >{app.cart.length > 99 ? '99+' : app.cart.length}</span
        >
      {/if}
    </button>
    <button
      class="pressable mr-1.5 grid size-8 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
      title={themeLabel}
      aria-label={themeLabel}
      onclick={() => app.cycleTheme()}
    >
      {#key app.theme}
        <span class="theme-icon grid place-items-center">
          {#if app.theme === 'system'}<Monitor class="size-4" />{:else if app.theme === 'dark'}<Moon
              class="size-4"
            />{:else}<Sun class="size-4" />{/if}
        </span>
      {/key}
    </button>

    <button class="winbtn" aria-label="Minimize" onclick={() => win.minimize()}>
      <span aria-hidden="true">&#xE921;</span>
    </button>
    <button
      class="winbtn"
      aria-label={maximized ? 'Restore' : 'Maximize'}
      onclick={() => win.toggleMaximize()}
    >
      <span aria-hidden="true">{@html maximized ? '&#xE923;' : '&#xE922;'}</span>
    </button>
    <button class="winbtn close" aria-label="Close" onclick={() => win.close()}>
      <span aria-hidden="true">&#xE8BB;</span>
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
  .winbtn span {
    font-family: 'Segoe Fluent Icons', 'Segoe MDL2 Assets';
    font-size: 10px;
    line-height: 1;
  }
  .winbtn:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .winbtn.close:hover {
    background: var(--close-hover);
    color: var(--close-fg);
  }
  .bump {
    animation: bump 420ms var(--ease-spring);
  }
  @keyframes bump {
    30% {
      transform: scale(1.35) rotate(-8deg);
    }
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
