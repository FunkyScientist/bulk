<script lang="ts">
  import { onMount } from 'svelte';
  import { Tween } from 'svelte/motion';
  import { cubicOut } from 'svelte/easing';
  import { fade, scale } from 'svelte/transition';
  import Zap from '@lucide/svelte/icons/zap';
  import Folder from '@lucide/svelte/icons/folder';
  import { app } from '$lib/app.svelte';
  import type { Drive } from '$lib/api';
  import { bytesParts, count, duration } from '$lib/format';

  let { drive }: { drive: Drive } = $props();

  const s = $derived(app.scans[drive.letter]);
  const files = new Tween(0, { duration: 260, easing: cubicOut });
  const found = new Tween(0, { duration: 260, easing: cubicOut });
  const frac = new Tween(0, { duration: 300, easing: cubicOut });

  $effect(() => {
    if (!s) return;
    files.target = s.progress.files;
    found.target = s.progress.bytes;
    // The walker can't know the total, so estimate against used space.
    const used = drive.total - drive.free;
    frac.target =
      s.progress.total > 0
        ? s.progress.done / s.progress.total
        : used > 0
          ? Math.min(0.97, s.progress.bytes / used)
          : 0;
  });

  let now = $state(Date.now());
  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 100);
    return () => clearInterval(t);
  });

  const [num, unit] = $derived(bytesParts(found.current));
</script>

<div class="grid h-full place-items-center" in:fade={{ duration: 200 }}>
  <div
    class="flex w-[380px] flex-col items-center text-center"
    in:scale={{ start: 0.96, duration: 360, easing: cubicOut }}
  >
    <div class="orb mb-6 grid size-16 place-items-center rounded-2xl bg-accent-soft text-accent">
      <Folder class="size-7" />
    </div>

    <div class="text-[13px] text-fg-muted">
      Scanning {drive.letter}: {drive.label ? `· ${drive.label}` : ''}
    </div>
    <div class="tabular mt-1 text-[40px] leading-none font-semibold tracking-tight">
      {count(files.current)}
    </div>
    <div class="mt-1.5 text-fg-subtle">files found</div>

    <div class="mt-7 h-1.5 w-full overflow-hidden rounded-full bg-panel-2">
      <div
        class="h-full rounded-full bg-accent"
        style="width: {Math.max(2, frac.current * 100)}%; box-shadow: 0 0 12px var(--accent)"
      ></div>
    </div>

    <div class="tabular mt-3 flex w-full justify-between text-[12px] text-fg-subtle">
      <span>{num} {unit} indexed</span>
      <span>{s ? duration(now - s.startedAt) : ''}</span>
    </div>

    <div class="mt-6 flex items-center gap-3">
      {#if s?.method}
        <span
          class="flex items-center gap-1 rounded-full border border-line px-2.5 py-1 text-[11.5px] text-fg-muted"
          in:fade
        >
          {#if s.method === 'mft'}<Zap class="size-3 text-accent" /> Fast scan{:else}Folder scan{/if}
        </span>
      {/if}
      <button
        class="pressable h-7 rounded-lg px-3 text-[12px] text-fg-muted hover:bg-hover hover:text-fg"
        onclick={() => app.cancel(drive.letter)}
      >
        Cancel
      </button>
    </div>
  </div>
</div>

<style>
  .orb {
    animation: breathe 2.4s ease-in-out infinite;
  }
  @keyframes breathe {
    50% {
      transform: scale(1.06);
      box-shadow:
        0 0 0 10px transparent,
        0 0 40px var(--accent-soft);
    }
  }
</style>
