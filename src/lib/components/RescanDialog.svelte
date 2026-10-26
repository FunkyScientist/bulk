<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Cloud from '@lucide/svelte/icons/cloud';
  import Usb from '@lucide/svelte/icons/usb';
  import Zap from '@lucide/svelte/icons/zap';
  import { app } from '$lib/app.svelte';
  import { ago, bytes } from '$lib/format';

  let { open = $bindable(false) }: { open: boolean } = $props();

  let picked = $state<Record<string, boolean>>({});

  // Local drives are ticked by default; cloud and network drives are opt-in.
  $effect(() => {
    if (open) picked = Object.fromEntries(app.drives.map((d) => [d.letter, d.autoScan]));
  });

  const count = $derived(Object.values(picked).filter(Boolean).length);

  async function go() {
    const letters = app.drives.filter((d) => picked[d.letter]).map((d) => d.letter);
    open = false;
    // One at a time, so drives on the same disk don't fight over it.
    for (const l of letters) await app.scan(l);
  }
</script>

{#if open}
  <div
    class="fixed inset-0 z-40 bg-[var(--backdrop)]"
    transition:fade={{ duration: 140 }}
    onclick={() => (open = false)}
    role="presentation"
  ></div>
  <div
    class="panel fixed top-1/2 left-1/2 z-50 w-[min(420px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-line-strong p-4"
    transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
    role="dialog"
    aria-label="Rescan drives"
  >
    <div class="mb-1 text-[15px] font-semibold tracking-tight">Rescan drives</div>
    <p class="mb-3 text-[12px] text-fg-muted">
      Live drives already stay up to date; a rescan rebuilds everything from scratch.
    </p>
    <div class="flex flex-col gap-1">
      {#each app.drives as d (d.letter)}
        {@const Icon =
          d.kind === 'virtual' || d.kind === 'network'
            ? Cloud
            : d.kind === 'removable'
              ? Usb
              : HardDrive}
        <label class="flex items-center gap-3 rounded-lg px-2 py-2 hover:bg-hover">
          <input type="checkbox" bind:checked={picked[d.letter]} class="accent-[var(--accent)]" />
          <Icon class="size-4 text-fg-muted" />
          <div class="min-w-0 flex-1">
            <div class="truncate">{d.letter}: {d.label || 'Local disk'}</div>
            <div class="truncate text-[11.5px] text-fg-subtle">
              {bytes(d.total - d.free)} used · {d.scanned
                ? `scanned ${ago(d.scannedAt ?? 0)}`
                : 'not scanned'}
              {#if d.kind === 'virtual'}
                · cloud drive{/if}
            </div>
          </div>
          {#if d.fs === 'NTFS' && app.helper === 'ready'}<Zap class="size-3.5 text-accent" />{/if}
        </label>
      {/each}
    </div>
    <div class="mt-4 flex justify-end gap-2">
      <button
        class="pressable h-8 rounded-lg px-3 text-[12.5px] hover:bg-hover"
        onclick={() => (open = false)}>Cancel</button
      >
      <button
        class="pressable h-8 rounded-lg bg-accent px-3 text-[12.5px] font-medium text-accent-fg hover:brightness-110 disabled:opacity-40"
        disabled={!count}
        onclick={go}>Rescan {count} drive{count === 1 ? '' : 's'}</button
      >
    </div>
  </div>
{/if}

<style>
  .panel {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(24px);
  }
</style>
