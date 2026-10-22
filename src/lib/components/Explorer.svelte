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
  import { api } from '$lib/api';
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import FolderSync from '@lucide/svelte/icons/folder-sync';
  import HardDrive from '@lucide/svelte/icons/hard-drive';

  let { drive }: { drive: Drive } = $props();

  // The folder open in the chart; "Rescan" covers just that by default.
  let folderName = $state<string | null>(null);
  $effect(() => {
    const id = app.focus[drive.letter] ?? null;
    const letter = drive.letter;
    if (id === null) {
      folderName = null;
      return;
    }
    api.nodeInfo(letter, id).then((i) => {
      if ((app.focus[letter] ?? null) === id) folderName = i.name;
    });
  });

  function rescanMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const at = { clientX: r.right - 220, clientY: r.bottom + 6, preventDefault() {} } as MouseEvent;
    const letter = drive.letter;
    app.openMenu(at, [
      ...(folderName
        ? [
            {
              label: `Rescan ${folderName}`,
              icon: FolderSync,
              hint: 'Ctrl R',
              run: () => app.scanFolder(letter),
            },
          ]
        : []),
      {
        label: `Rescan whole ${letter}: drive`,
        icon: HardDrive,
        hint: folderName ? 'Ctrl Shift R' : 'Ctrl R',
        run: () => app.scan(letter),
      },
    ]);
  }
  const scan = $derived(app.scans[drive.letter]);
  const scanning = $derived(app.isScanning(drive.letter));

  const tabs = [
    { k: 'browse', l: 'Browse', I: FolderTree },
    { k: 'changes', l: 'Changes', I: History },
    { k: 'duplicates', l: 'Duplicates', I: CopyIcon },
  ] as const;

  let body: HTMLDivElement | undefined = $state();
  const rows = $derived(app.layout === 'rows');
  let dragging = $state(false);
  function startDrag(e: PointerEvent) {
    dragging = true;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      if (!body) return;
      const r = body.getBoundingClientRect();
      app.setSplit(rows ? (ev.clientY - r.top) / r.height : (ev.clientX - r.left) / r.width);
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
          Rescanning{scan.folder ? ` ${scan.folder}` : ''} · {count(scan.progress.files)} files
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
<div
        class="flex h-7 shrink-0 items-stretch rounded-lg border border-line text-[12px] whitespace-nowrap text-fg-muted transition-colors hover:border-line-strong"
        class:opacity-50={scanning}
      >
        <button
          class="pressable flex items-center gap-1.5 rounded-l-lg pr-2 pl-2.5 hover:bg-hover hover:text-fg"
          disabled={scanning}
          title={folderName ? `Rescan ${folderName} (Ctrl R)` : 'Rescan the drive (Ctrl R)'}
          onclick={() => app.scanFolder(drive.letter)}
        >
          <RefreshCw class="size-3.5 {scanning ? 'animate-spin' : ''}" />
          {folderName ? 'Rescan folder' : 'Rescan'}
        </button>
        <span class="my-1.5 w-px bg-line"></span>
        <button
          class="pressable grid w-6 place-items-center rounded-r-lg hover:bg-hover hover:text-fg"
          disabled={scanning}
          aria-label="More rescan options"
          onclick={rescanMenu}
        >
          <ChevronDown class="size-3.5" />
        </button>
      </div>
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
    <div
      class="flex min-h-0 flex-1 {rows ? 'flex-col' : ''}"
      bind:this={body}
      in:fade={{ duration: 160 }}
    >
      <div class="min-h-0 min-w-0" style="{rows ? 'height' : 'width'}: {app.split * 100}%">
        <TreeList letter={drive.letter} />
      </div>
      <div
        class="splitter"
        class:dragging
        class:rows
        role="separator"
        aria-orientation={rows ? 'horizontal' : 'vertical'}
        tabindex="-1"
        onpointerdown={startDrag}
      ></div>
      <div class="min-h-0 min-w-0 flex-1">
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
  .splitter.rows {
    width: auto;
    height: 1px;
    cursor: row-resize;
  }
  .splitter.rows::after {
    inset: -4px 0;
  }
  .splitter.rows::before {
    top: -1.5px;
    left: 50%;
    width: 32px;
    height: 4px;
    margin-top: 0;
    margin-left: -16px;
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
