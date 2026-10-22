<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import Trash from '@lucide/svelte/icons/trash-2';
  import Folder from '@lucide/svelte/icons/folder';
  import File from '@lucide/svelte/icons/file';
  import { api, type NodeInfo } from '$lib/api';
  import { app } from '$lib/app.svelte';
  import { bytes, count } from '$lib/format';

  let info = $state<NodeInfo | null>(null);
  let busy = $state(false);
  let recycleBtn: HTMLButtonElement | undefined = $state();

  const req = $derived(app.deleteRequest);

  $effect(() => {
    const r = req;
    info = null;
    if (!r) return;
    api.nodeInfo(r.letter, r.id).then((i) => {
      if (app.deleteRequest === r) info = i;
    });
  });

  // Enter confirms the safe choice.
  $effect(() => {
    if (info) recycleBtn?.focus();
  });

  function close() {
    if (!busy) app.deleteRequest = null;
  }

  async function run(permanent: boolean) {
    const r = req;
    if (!r || busy) return;
    busy = true;
    try {
      const res = await api.deleteNodes(r.letter, [r.id], permanent);
      app.reportDelete(res, permanent);
      if (res.deleted && app.selected === r.id) app.selected = null;
    } catch (e) {
      app.toast(`Couldn't delete: ${e}`, { tone: 'error' });
    } finally {
      busy = false;
      app.deleteRequest = null;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }
</script>

{#if req && info}
  <div
    class="fixed inset-0 z-50 bg-black/40"
    transition:fade={{ duration: 140 }}
    role="presentation"
    onclick={close}
  ></div>
  <div
    class="panel fixed top-1/2 left-1/2 z-50 w-[min(440px,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-line-strong p-5"
    transition:scale={{ start: 0.96, duration: 180, easing: cubicOut }}
    role="alertdialog"
    aria-labelledby="del-title"
    tabindex="-1"
    onkeydown={onKey}
  >
    <div class="flex items-start gap-3">
      <div
        class="grid size-9 shrink-0 place-items-center rounded-xl bg-[color-mix(in_oklab,var(--danger)_15%,transparent)] text-danger"
      >
        <Trash class="size-4" />
      </div>
      <div class="min-w-0">
        <div id="del-title" class="font-semibold">
          Delete this {info.dir ? 'folder' : 'file'}?
        </div>
        <div class="mt-2 flex items-center gap-2 rounded-lg bg-panel-2 px-2.5 py-2">
          <span class="shrink-0 {info.dir ? 'text-accent' : 'text-fg-subtle'}">
            {#if info.dir}<Folder class="size-3.5" />{:else}<File class="size-3.5" />{/if}
          </span>
          <div class="min-w-0">
            <div class="truncate font-medium">{info.name}</div>
            <div class="truncate text-[11.5px] text-fg-subtle" title={info.path}>{info.path}</div>
          </div>
          <div class="tabular ml-auto shrink-0 pl-2 text-right text-[12px] text-fg-muted">
            {bytes(info.alloc)}
            {#if info.dir}<div class="text-[11px] text-fg-subtle">
                {count(info.files)} files
              </div>{/if}
          </div>
        </div>
      </div>
    </div>
    <div class="mt-5 flex items-center gap-2">
      <button
        class="pressable h-8 rounded-lg px-3 text-[12.5px] text-danger hover:bg-[color-mix(in_oklab,var(--danger)_12%,transparent)] disabled:opacity-40"
        disabled={busy}
        onclick={() => run(true)}>Delete permanently</button
      >
      <div class="ml-auto flex gap-2">
        <button
          class="pressable h-8 rounded-lg px-3 text-[12.5px] hover:bg-hover"
          disabled={busy}
          onclick={close}>Cancel</button
        >
        <button
          bind:this={recycleBtn}
          class="pressable h-8 rounded-lg bg-accent px-3 text-[12.5px] font-medium text-accent-fg hover:brightness-110 disabled:opacity-40"
          disabled={busy}
          onclick={() => run(false)}>{busy ? 'Deleting…' : 'Move to Recycle Bin'}</button
        >
      </div>
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
