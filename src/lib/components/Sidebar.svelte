<script lang="ts">
  import { fade, fly, slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Usb from '@lucide/svelte/icons/usb';
  import Cloud from '@lucide/svelte/icons/cloud';
  import Network from '@lucide/svelte/icons/network';
  import Zap from '@lucide/svelte/icons/zap';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Settings from '@lucide/svelte/icons/settings';
  import PanelLeftClose from '@lucide/svelte/icons/panel-left-close';
  import PanelLeftOpen from '@lucide/svelte/icons/panel-left-open';
  import { app } from '$lib/app.svelte';
  import type { Drive } from '$lib/api';
  import { ago, bytes, count } from '$lib/format';
  import UsageRing from './UsageRing.svelte';
  import Tooltip from './Tooltip.svelte';

  let { onRescan, onSettings }: { onRescan: () => void; onSettings?: () => void } = $props();

  const collapsed = $derived(app.sidebarCollapsed);

  function icon(d: Drive) {
    if (d.kind === 'removable') return Usb;
    if (d.kind === 'virtual') return Cloud;
    if (d.kind === 'network') return Network;
    return HardDrive;
  }

  function label(d: Drive) {
    return d.label || (d.kind === 'removable' ? 'USB drive' : 'Local disk');
  }

  function status(d: Drive) {
    const s = app.scans[d.letter];
    if (s?.error) return 'Scan failed';
    if (s && !s.finished) return `Scanning · ${count(s.progress.files)} files`;
    if (d.scanned && d.live) return `${count(d.files)} files · live`;
    if (d.scanned) return `${count(d.files)} files · as of ${ago(d.scannedAt ?? 0)}`;
    if (!d.autoScan)
      return d.kind === 'virtual' ? 'Cloud drive · scan on demand' : 'Scan on demand';
    return 'Not scanned yet';
  }

  let enabling = $state(false);
  async function enable() {
    enabling = true;
    await app.allowFastMode();
    enabling = false;
  }

  const needsHelper = $derived(app.helper === 'notInstalled' || app.helper === 'outdated');
  const helperLabel = $derived(enabling ? 'Waiting for Windows…' : 'Allow');
</script>

<aside class="sidebar flex shrink-0 flex-col gap-1 pb-2.5" class:collapsed>
  <div
    class="flex h-8 items-center pt-1 pb-1.5 {collapsed
      ? 'justify-center'
      : 'justify-between pl-2'}"
  >
    {#if !collapsed}
      <span
        class="text-[11px] font-medium tracking-wide whitespace-nowrap text-fg-subtle uppercase"
        in:fade={{ duration: 160, delay: 120 }}>Drives</span
      >
    {/if}
    <div class="flex items-center gap-0.5">
      {#if !collapsed}
        <span in:fade={{ duration: 160, delay: 120 }}>
          <Tooltip content="Rescan drives" side="bottom">
            <button
              class="pressable grid size-7 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
              aria-label="Rescan drives"
              onclick={onRescan}
            >
              <RefreshCw class="size-4" />
            </button>
          </Tooltip>
        </span>
      {/if}
      <Tooltip
        content={collapsed ? 'Expand sidebar (Ctrl+B)' : 'Collapse sidebar (Ctrl+B)'}
        side={collapsed ? 'right' : 'bottom'}
      >
        <button
          class="pressable grid size-7 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
          aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          aria-expanded={!collapsed}
          onclick={() => app.toggleSidebar()}
        >
          {#if collapsed}<PanelLeftOpen class="size-4" />{:else}<PanelLeftClose
              class="size-4"
            />{/if}
        </button>
      </Tooltip>
    </div>
  </div>

  <nav class="flex flex-col gap-1 overflow-x-hidden overflow-y-auto">
    {#each app.drives as d, i (d.letter)}
      {@const Icon = icon(d)}
      {@const used = d.total - d.free}
      {@const active = app.current === d.letter}
      {@const scanning = app.isScanning(d.letter)}
      <div in:fly={{ y: 8, duration: 360, delay: 40 + i * 50, easing: cubicOut }}>
        <Tooltip disabled={!collapsed}>
          {#snippet content()}
            <div class="font-medium">{d.letter}: {label(d)}</div>
            <div class="tabular text-[11.5px] text-fg-muted">
              {bytes(d.free)} free of {bytes(d.total)}
            </div>
            <div
              class="tabular text-[11.5px] {app.scans[d.letter]?.error
                ? 'text-danger'
                : 'text-fg-subtle'}"
            >
              {status(d)}
            </div>
          {/snippet}
          <button
            class="drive pressable relative flex w-full items-center rounded-xl text-left {collapsed
              ? 'flex-col gap-0.5 px-0 py-1.5'
              : 'gap-3 px-2.5 py-2'}"
            class:active
            aria-label="{d.letter}: {label(d)}"
            onclick={() => app.select(d.letter)}
          >
            <div class="relative grid shrink-0 place-items-center">
              <UsageRing value={d.total ? used / d.total : 0} active={scanning} />
              <Icon class="absolute size-3.5 {active ? 'text-fg' : 'text-fg-muted'}" />
              {#if collapsed && d.live && !scanning}
                <span class="dot ok" in:fade></span>
              {/if}
            </div>
            {#if collapsed}
              <span class="text-[11px] font-medium {active ? 'text-fg' : 'text-fg-muted'}"
                >{d.letter}:</span
              >
            {:else}
              <div class="min-w-0 flex-1 whitespace-nowrap" in:fade={{ duration: 160, delay: 100 }}>
                <div class="flex items-baseline gap-1.5">
                  <span class="font-medium">{d.letter}:</span>
                  <span class="truncate text-fg-muted">{label(d)}</span>
                </div>
                <div class="tabular truncate text-[11.5px] text-fg-subtle">
                  {bytes(d.free)} free of {bytes(d.total)}
                </div>
                <div
                  class="tabular truncate text-[11px] {app.scans[d.letter]?.error
                    ? 'text-danger'
                    : 'text-fg-subtle'}"
                >
                  {status(d)}
                </div>
              </div>
            {/if}
            {#if scanning}
              <span class="scanbar" transition:slide={{ duration: 200, axis: 'x' }}></span>
            {/if}
          </button>
        </Tooltip>
      </div>
    {/each}
  </nav>

  <div class="mt-auto flex flex-col gap-2">
    {#if needsHelper}
      {#if collapsed}
        <div class="flex justify-center" in:fade={{ duration: 160 }}>
          <Tooltip>
            {#snippet content()}
              <div class="font-medium">Fast mode needs permission</div>
              <div class="text-[11.5px] text-fg-muted">
                Click to let Bulk read drives directly. Windows asks once.
              </div>
            {/snippet}
            <button
              class="pressable grid size-9 place-items-center rounded-xl bg-accent-soft text-accent hover:brightness-125 disabled:opacity-60"
              aria-label={helperLabel}
              disabled={enabling}
              onclick={enable}
            >
              <Zap class="size-4 {enabling ? 'animate-pulse' : ''}" />
            </button>
          </Tooltip>
        </div>
      {:else}
        <div
          class="rounded-xl border border-line bg-panel p-3"
          in:fly={{ y: 10, duration: 300, delay: 100, easing: cubicOut }}
        >
          <div class="flex items-center gap-2 font-medium whitespace-nowrap">
            <span class="grid size-5 place-items-center rounded-md bg-accent-soft text-accent"
              ><Zap class="size-3" /></span
            >
            Fast mode needs permission
          </div>
          <p class="mt-1.5 text-[11.5px] leading-relaxed text-fg-muted">
            Bulk reads drives directly for scans in seconds and live updates. Windows asks once.
          </p>
          <button
            class="pressable mt-2.5 h-7 w-full rounded-lg bg-accent text-[12px] font-medium text-accent-fg hover:brightness-110 disabled:opacity-60"
            disabled={enabling}
            onclick={enable}
          >
            {helperLabel}
          </button>
        </div>
      {/if}
    {/if}

    <Tooltip content="Settings" disabled={!collapsed}>
      <button
        class="pressable flex h-9 w-full items-center gap-2.5 rounded-lg text-fg-muted hover:bg-hover hover:text-fg {collapsed
          ? 'justify-center'
          : 'px-2.5'}"
        aria-label="Settings"
        onclick={() => onSettings?.()}
      >
        <Settings class="shrink-0 {collapsed ? 'size-[18px]' : 'size-4'}" />
        {#if !collapsed}<span in:fade={{ duration: 160, delay: 100 }}>Settings</span>{/if}
      </button>
    </Tooltip>
  </div>
</aside>

<style>
  .sidebar {
    width: 244px;
    padding-left: 10px;
    padding-right: 10px;
    transition:
      width 280ms var(--ease-out-soft),
      padding 280ms var(--ease-out-soft);
  }
  .sidebar.collapsed {
    width: 68px;
    padding-left: 8px;
    padding-right: 8px;
  }
  .drive {
    transition:
      background-color 160ms ease,
      transform 140ms var(--ease-out-soft);
  }
  .drive:hover {
    background: var(--hover);
  }
  .drive.active {
    background: var(--panel-2);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .drive.active::before {
    content: '';
    position: absolute;
    left: -8px;
    top: 30%;
    bottom: 30%;
    width: 3px;
    border-radius: 3px;
    background: var(--accent);
    animation: grow 260ms var(--ease-spring);
  }
  @keyframes grow {
    from {
      transform: scaleY(0);
    }
  }
  .dot {
    position: absolute;
    right: -1px;
    top: -1px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 2px solid var(--app-bg);
  }
  .dot.ok {
    background: var(--live);
  }
  .scanbar {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 3px;
    height: 2px;
    border-radius: 2px;
    background: linear-gradient(90deg, transparent, var(--accent), transparent);
    background-size: 50% 100%;
    background-repeat: no-repeat;
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      background-position: -100% 0;
    }
    to {
      background-position: 200% 0;
    }
  }
</style>
