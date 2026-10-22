<script lang="ts">
  import { fade, fly, scale } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import X from '@lucide/svelte/icons/x';
  import Folder from '@lucide/svelte/icons/folder';
  import File from '@lucide/svelte/icons/file';
  import Trash from '@lucide/svelte/icons/trash-2';
  import ShoppingCart from '@lucide/svelte/icons/shopping-cart';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import { api } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, count } from '$lib/format';

  const total = $derived(app.cart.reduce((s, c) => s + c.alloc, 0));
  let busy = $state(false);
  let progress = $state<[number, number] | null>(null);
  let confirming = $state(false);
  let understood = $state(false);

  onMount(() => {
    const un = listen<[number, number]>('cart-progress', (e) => (progress = e.payload));
    return () => void un.then((f) => f());
  });

  async function run(permanent: boolean) {
    busy = true;
    confirming = false;
    progress = [0, app.cart.length];
    try {
      const r = await api.cartDelete(permanent);
      if (r.deleted) {
        const what = `${count(r.deleted)} item${r.deleted === 1 ? '' : 's'}`;
        app.toast(
          permanent
            ? `Deleted ${what} · freed ${bytes(r.freed)}`
            : `Moved ${what} (${bytes(r.freed)}) to the Recycle Bin`,
          {
            tone: 'ok',
            action:
              r.undo !== null ? { label: 'Undo', run: () => undo(r.undo as number) } : undefined,
          },
        );
      }
      if (r.failed.length)
        app.toast(`${r.failed.length} couldn't be deleted: ${r.failed[0].error}`, {
          tone: 'error',
          ms: 8000,
        });
      if (!app.cart.length || r.deleted) app.cartOpen = r.failed.length > 0;
    } finally {
      busy = false;
      progress = null;
      understood = false;
    }
  }

  async function undo(id: number) {
    const r = await api.cartUndo(id);
    if (r.restored)
      app.toast(`Restored ${count(r.restored)} item${r.restored === 1 ? '' : 's'}`, { tone: 'ok' });
    if (r.failed.length)
      app.toast(`Couldn't restore ${r.failed.length}: ${r.failed[0].error}`, { tone: 'error' });
    for (const l of r.needsRescan)
      app.toast(`Rescan ${l}: to see restored items`, {
        action: { label: 'Rescan', run: () => app.scan(l) },
      });
  }
</script>

{#if app.cartOpen}
  <div
    class="fixed inset-0 z-30 bg-black/25"
    transition:fade={{ duration: 160 }}
    onclick={() => !busy && (app.cartOpen = false)}
    role="presentation"
  ></div>
  <aside
    class="panel fixed top-12 right-3 bottom-3 z-30 flex w-[min(400px,92vw)] flex-col overflow-hidden rounded-2xl border border-line-strong"
    transition:fly={{ x: 40, duration: 280, easing: cubicOut }}
    aria-label="Cleanup cart"
  >
    <div class="flex items-center gap-2 border-b border-line px-4 py-3">
      <ShoppingCart class="size-4 text-accent" />
      <div class="flex-1">
        <div class="font-semibold">Cleanup cart</div>
        <div class="tabular text-[11.5px] text-fg-subtle">
          {count(app.cart.length)} item{app.cart.length === 1 ? '' : 's'} · {bytes(total)}
        </div>
      </div>
      {#if app.cart.length}
        <button
          class="pressable rounded-md px-2 py-1 text-[12px] text-fg-muted hover:bg-hover hover:text-fg"
          disabled={busy}
          onclick={() => api.cartClear()}>Clear</button
        >
      {/if}
      <button
        class="pressable grid size-7 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
        aria-label="Close"
        onclick={() => (app.cartOpen = false)}><X class="size-4" /></button
      >
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto p-1.5">
      {#if !app.cart.length}
        <div class="flex h-full flex-col items-center justify-center px-8 text-center" in:fade>
          <div class="mb-3 grid size-12 place-items-center rounded-2xl bg-panel-2 text-fg-muted">
            <ShoppingCart class="size-5" />
          </div>
          <div class="font-medium">Nothing here yet</div>
          <p class="mt-1 text-[12px] leading-relaxed text-fg-muted">
            Right-click anything in the list or chart, or press Delete, to collect it here. Nothing
            is removed until you confirm.
          </p>
        </div>
      {/if}
      {#each app.cart as item (item.path)}
        <div
          class="group flex items-center gap-2.5 rounded-lg px-2.5 py-2 hover:bg-hover"
          animate:flip={{ duration: 220 }}
          in:fly={{ x: 12, duration: 220 }}
          out:fly={{ x: 24, duration: 180 }}
        >
          <span class="shrink-0 {item.dir ? 'text-accent' : 'text-fg-subtle'}">
            {#if item.dir}<Folder class="size-4" />{:else}<File class="size-4" />{/if}
          </span>
          <div class="min-w-0 flex-1">
            <div class="truncate">{item.name}</div>
            <div class="truncate text-[11px] text-fg-subtle" title={item.path}>{item.path}</div>
          </div>
          <span class="tabular shrink-0 text-[12px]">{bytes(item.alloc)}</span>
          <button
            class="grid size-6 shrink-0 place-items-center rounded-md text-fg-subtle opacity-0 group-hover:opacity-100 hover:bg-hover hover:text-fg"
            aria-label="Remove {item.name}"
            disabled={busy}
            onclick={() => app.removeFromCart([item.path])}><X class="size-3.5" /></button
          >
        </div>
      {/each}
    </div>

    {#if app.cart.length}
      <div class="border-t border-line p-3">
        {#if progress}
          <div class="mb-2.5 h-1 overflow-hidden rounded-full bg-panel-2">
            <div
              class="h-full rounded-full bg-accent transition-[width] duration-200"
              style="width: {((progress[0] + 1) / Math.max(1, progress[1])) * 100}%"
            ></div>
          </div>
        {/if}
        <button
          class="pressable flex h-9 w-full items-center justify-center gap-2 rounded-lg bg-accent text-[12.5px] font-medium text-accent-fg hover:brightness-110 disabled:opacity-60"
          disabled={busy}
          onclick={() => run(false)}
        >
          <Trash class="size-4" />
          {busy ? 'Working…' : `Move to Recycle Bin · ${bytes(total)}`}
        </button>
        <div class="mt-2 flex items-center justify-between text-[11.5px]">
          <span class="text-fg-subtle">Space is freed once the bin is emptied.</span>
          <button
            class="text-fg-muted underline-offset-2 hover:text-danger hover:underline"
            disabled={busy}
            onclick={() => (confirming = true)}>Delete permanently…</button
          >
        </div>
      </div>
    {/if}
  </aside>
{/if}

{#if confirming}
  <div
    class="fixed inset-0 z-50 bg-black/40"
    transition:fade={{ duration: 140 }}
    role="presentation"
    onclick={() => (confirming = false)}
  ></div>
  <div
    class="panel fixed top-1/2 left-1/2 z-50 w-[min(420px,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-line-strong p-5"
    transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
    role="alertdialog"
    aria-labelledby="perm-title"
  >
    <div class="flex items-start gap-3">
      <div
        class="grid size-9 shrink-0 place-items-center rounded-xl bg-[color-mix(in_oklab,var(--danger)_15%,transparent)] text-danger"
      >
        <TriangleAlert class="size-4" />
      </div>
      <div>
        <div id="perm-title" class="font-semibold">
          Delete {count(app.cart.length)} items permanently?
        </div>
        <p class="mt-1 text-[12.5px] leading-relaxed text-fg-muted">
          {bytes(total)} will be removed right away. They won't go to the Recycle Bin and this can't be
          undone.
        </p>
      </div>
    </div>
    <label class="mt-4 flex items-center gap-2 text-[12.5px]">
      <input type="checkbox" bind:checked={understood} class="accent-[var(--danger)]" />
      I understand these files can't be recovered
    </label>
    <div class="mt-4 flex justify-end gap-2">
      <button
        class="pressable h-8 rounded-lg px-3 text-[12.5px] hover:bg-hover"
        onclick={() => (confirming = false)}>Cancel</button
      >
      <button
        class="pressable h-8 rounded-lg bg-danger px-3 text-[12.5px] font-medium text-white disabled:opacity-40"
        disabled={!understood}
        onclick={() => run(true)}>Delete permanently</button
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
