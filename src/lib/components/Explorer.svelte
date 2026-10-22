<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Zap from '@lucide/svelte/icons/zap';
  import X from '@lucide/svelte/icons/x';
  import Check from '@lucide/svelte/icons/check';
  import { app } from '$lib/app.svelte';
  import type { Drive } from '$lib/api';
  import { ago, bytes, count, duration } from '$lib/format';
  import TreeList from './TreeList.svelte';
  import VisualPane from './VisualPane.svelte';
  import DiffView from './DiffView.svelte';
  import FolderTree from '@lucide/svelte/icons/folder-tree';
  import History from '@lucide/svelte/icons/history';
  import CopyIcon from '@lucide/svelte/icons/copy';
  import DupesView from './DupesView.svelte';
  import Share from '@lucide/svelte/icons/share';
  import { exportMenu } from '$lib/exporting';

  let { drive }: { drive: Drive } = $props();
  const scan = $derived(app.scans[drive.letter]);
  const scanning = $derived(app.isScanning(drive.letter));

  const tabs = [
    { k: 'browse', l: 'Browse', I: FolderTree },
    { k: 'changes', l: 'Changes', I: History },
    { k: 'duplicates', l: 'Duplicates', I: CopyIcon },
  ] as const;

  let body: HTMLDivElement | undefined = $state();
  let dragging = $state(false);
  function startDrag(e: PointerEvent) {
    dragging = true;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      if (!body) return;
      const r = body.getBoundingClientRect();
      app.setSplit((ev.clientX - r.left) / r.width);
    };
    const up = () => {
      dragging = false;
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }
</script>

<div class="flex h-full flex-col" in:fade={{ duration: 160 }}>
  <div class="flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
    <div class="min-w-0">
      <div class="flex items-baseline gap-2">
        <span class="text-[14px] font-semibold tracking-tight">{drive.letter}:</span>
        <span class="truncate text-fg-muted">{drive.label || 'Local disk'}</span>
      </div>
    </div>
    <div class="tabular hidden truncate text-[12px] text-fg-subtle md:block">
      {bytes(drive.alloc)} in {count(drive.files)} files
      {#if drive.method === 'mft'}<span class="mx-1">·</span><Zap
          class="inline size-3 -translate-y-px text-accent"
        /> Turbo{/if}
      {#if !drive.live}<span class="mx-1">·</span>as of {ago(drive.scannedAt ?? 0)}{/if}
    </div>
    {#if drive.live}
      <span
        class="flex items-center gap-1.5 rounded-full bg-[color-mix(in_oklab,var(--ok)_14%,transparent)] px-2 py-0.5 text-[11px] font-medium text-ok"
        title="Kept up to date as files change"
        in:fade={{ duration: 200 }}
      >
        <span class="live-dot"></span> Live
      </span>
    {/if}

    <div class="ml-auto flex items-center gap-2">
      <div class="tabs relative flex rounded-lg bg-panel-2 p-0.5 text-[12px]">
        <span
          class="thumb"
          style="transform: translateX({tabs.findIndex((t) => t.k === app.tab) * 100}%)"
        ></span>
        {#each tabs as t (t.k)}
          <button
            class="relative z-10 flex h-6 w-[92px] items-center justify-center gap-1.5 rounded-md transition-colors {app.tab ===
            t.k
              ? 'text-fg'
              : 'text-fg-subtle hover:text-fg-muted'}"
            aria-pressed={app.tab === t.k}
            onclick={() => (app.tab = t.k)}
          >
            <t.I class="size-3.5" />{t.l}
          </button>
        {/each}
      </div>
      {#if scan?.finished}
        <span
          class="flex items-center gap-1.5 rounded-full bg-accent-soft px-2.5 py-1 text-[11.5px] text-accent"
          in:fly={{ y: -6, duration: 260, easing: cubicOut }}
          out:fade={{ duration: 200 }}
        >
          <Check class="size-3" /> Scanned in {duration(scan.finished.elapsedMs)}
        </span>
      {:else if scanning && scan}
        <span
          class="tabular flex items-center gap-2 rounded-full border border-line px-2.5 py-1 text-[11.5px] text-fg-muted"
          in:fly={{ y: -6, duration: 260, easing: cubicOut }}
        >
          <span class="dot"></span>
          Rescanning · {count(scan.progress.files)} files
          <button
            class="pressable -mr-1 grid size-4 place-items-center rounded-full hover:bg-hover"
            aria-label="Cancel scan"
            onclick={() => app.cancel(drive.letter)}><X class="size-3" /></button
          >
        </span>
      {/if}
      <button
        class="pressable flex h-7 items-center gap-1.5 rounded-lg border border-line px-2.5 text-[12px] text-fg-muted hover:border-line-strong hover:text-fg"
        title="Export the folder shown in the chart"
        onclick={(e) => exportMenu(e, drive.letter)}
      >
        <Share class="size-3.5" /> Export
      </button>
      <button
        class="pressable flex h-7 items-center gap-1.5 rounded-lg border border-line px-2.5 text-[12px] text-fg-muted hover:border-line-strong hover:text-fg disabled:opacity-50"
        disabled={scanning}
        onclick={() => app.scan(drive.letter)}
      >
        <RefreshCw class="size-3.5 {scanning ? 'animate-spin' : ''}" /> Rescan
      </button>
    </div>
  </div>

  {#if app.tab === 'duplicates'}
    <div class="min-h-0 flex-1">
      <DupesView letter={drive.letter} />
    </div>
  {:else if app.tab === 'changes'}
    <div class="min-h-0 flex-1">
      <DiffView letter={drive.letter} method={drive.method} />
    </div>
  {:else}
    <div class="flex min-h-0 flex-1" bind:this={body} in:fade={{ duration: 160 }}>
      <div class="min-w-0" style="width: {app.split * 100}%">
        <TreeList letter={drive.letter} />
      </div>
      <div
        class="splitter"
        class:dragging
        role="separator"
        aria-orientation="vertical"
        tabindex="-1"
        onpointerdown={startDrag}
      ></div>
      <div class="min-w-0 flex-1">
        <VisualPane letter={drive.letter} />
      </div>
    </div>
  {/if}
</div>

<style>
  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 92px;
    height: 24px;
    border-radius: 6px;
    background: var(--panel);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.25),
      inset 0 0 0 1px var(--line);
    transition: transform 260ms var(--ease-spring);
  }
  .live-dot {
    position: relative;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }
  .live-dot::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: var(--ok);
    animation: ping 1.8s cubic-bezier(0, 0, 0.2, 1) infinite;
  }
  @keyframes ping {
    75%,
    100% {
      transform: scale(2.6);
      opacity: 0;
    }
  }
  .splitter {
    position: relative;
    width: 1px;
    background: var(--line);
    cursor: col-resize;
  }
  .splitter::after {
    content: '';
    position: absolute;
    inset: 0 -4px;
  }
  .splitter::before {
    content: '';
    position: absolute;
    top: 50%;
    left: -1.5px;
    width: 4px;
    height: 32px;
    margin-top: -16px;
    border-radius: 4px;
    background: var(--line-strong);
    opacity: 0;
    transition: opacity 160ms ease;
  }
  .splitter:hover::before,
  .splitter.dragging::before {
    opacity: 1;
    background: var(--accent);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: blink 1s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }
</style>
