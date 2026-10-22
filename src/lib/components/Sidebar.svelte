<script lang="ts">
  import { fly, slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Usb from '@lucide/svelte/icons/usb';
  import Cloud from '@lucide/svelte/icons/cloud';
  import Network from '@lucide/svelte/icons/network';
  import Zap from '@lucide/svelte/icons/zap';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Settings from '@lucide/svelte/icons/settings';
  import { app } from '$lib/app.svelte';
  import type { Drive } from '$lib/api';
  import { ago, bytes, count } from '$lib/format';
  import UsageRing from './UsageRing.svelte';

  let { onRescan, onSettings }: { onRescan: () => void; onSettings?: () => void } = $props();

  function icon(d: Drive) {
    if (d.kind === 'removable') return Usb;
    if (d.kind === 'virtual') return Cloud;
    if (d.kind === 'network') return Network;
    return HardDrive;
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
    await app.enableFastScan();
    enabling = false;
  }
</script>

<aside class="flex w-[244px] shrink-0 flex-col gap-1 px-2.5 pb-2.5">
  <div class="flex items-center justify-between px-2 pt-1 pb-1.5">
    <span class="text-[11px] font-medium tracking-wide text-fg-subtle uppercase">Drives</span>
    <button
      class="pressable grid size-6 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
      title="Rescan drives…"
      aria-label="Rescan drives"
      onclick={onRescan}
    >
      <RefreshCw class="size-3.5" />
    </button>
  </div>

  <nav class="flex flex-col gap-1 overflow-y-auto">
    {#each app.drives as d, i (d.letter)}
      {@const Icon = icon(d)}
      {@const used = d.total - d.free}
      {@const active = app.current === d.letter}
      {@const scanning = app.isScanning(d.letter)}
      <button
        class="drive pressable group relative flex items-center gap-3 rounded-xl px-2.5 py-2 text-left"
        class:active
        onclick={() => app.select(d.letter)}
        in:fly={{ y: 8, duration: 360, delay: 40 + i * 50, easing: cubicOut }}
      >
        <div class="relative grid shrink-0 place-items-center">
          <UsageRing value={d.total ? used / d.total : 0} active={scanning} />
          <Icon class="absolute size-3.5 {active ? 'text-accent' : 'text-fg-muted'}" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex items-baseline gap-1.5">
            <span class="font-medium">{d.letter}:</span>
            <span class="truncate text-fg-muted"
              >{d.label || (d.kind === 'removable' ? 'USB drive' : 'Local disk')}</span
            >
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
        {#if scanning}
          <span class="scanbar" transition:slide={{ duration: 200, axis: 'x' }}></span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="mt-auto flex flex-col gap-2">
    {#if app.helper === 'notInstalled' || app.helper === 'outdated'}
      <div
        class="rounded-xl border border-line bg-panel p-3"
        transition:fly={{ y: 10, duration: 300, easing: cubicOut }}
      >
        <div class="flex items-center gap-2 font-medium">
          <span class="grid size-5 place-items-center rounded-md bg-accent-soft text-accent"
            ><Zap class="size-3" /></span
          >
          Turbo scan is off
        </div>
        <p class="mt-1.5 text-[11.5px] leading-relaxed text-fg-muted">
          Read drives directly for scans up to 10× faster. Windows asks for permission once.
        </p>
        <button
          class="pressable mt-2.5 h-7 w-full rounded-lg bg-accent text-[12px] font-medium text-accent-fg hover:brightness-110 disabled:opacity-60"
          disabled={enabling}
          onclick={enable}
        >
          {enabling
            ? 'Waiting for Windows…'
            : app.helper === 'outdated'
              ? 'Update Turbo scan'
              : 'Turn on Turbo scan'}
        </button>
      </div>
    {/if}

    <button
      class="pressable flex h-8 items-center gap-2 rounded-lg px-2.5 text-fg-muted hover:bg-hover hover:text-fg"
      onclick={() => onSettings?.()}
    >
      <Settings class="size-3.5" />
      Settings
    </button>
  </div>
</aside>

<style>
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
    left: -10px;
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
