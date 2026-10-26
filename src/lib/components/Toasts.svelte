<script lang="ts">
  import { fly } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import X from '@lucide/svelte/icons/x';
  import Check from '@lucide/svelte/icons/check';
  import AlertCircle from '@lucide/svelte/icons/circle-alert';
  import { app } from '$lib/app.svelte';
</script>

<div
  class="pointer-events-none fixed bottom-5 left-1/2 z-[80] flex -translate-x-1/2 flex-col items-center gap-2"
>
  {#each app.toasts as t (t.id)}
    <div
      class="toast pointer-events-auto flex items-center gap-3 rounded-xl border border-line-strong py-2 pr-2 pl-3.5 text-[12.5px]"
      in:fly={{ y: 16, duration: 260, easing: cubicOut }}
      out:fly={{ y: 8, duration: 180 }}
      animate:flip={{ duration: 220 }}
      role="status"
    >
      {#if t.tone === 'error'}
        <AlertCircle class="size-4 shrink-0 text-danger" />
      {:else if t.tone === 'ok'}
        <Check class="size-4 shrink-0 text-ok" />
      {/if}
      <span>{t.message}</span>
      {#if t.action}
        <button
          class="pressable rounded-md px-2 py-1 font-medium text-accent hover:bg-accent-soft"
          onclick={() => {
            t.action?.run();
            app.dismissToast(t.id);
          }}>{t.action.label}</button
        >
      {/if}
      <button
        class="grid size-6 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
        aria-label="Dismiss"
        onclick={() => app.dismissToast(t.id)}><X class="size-3.5" /></button
      >
    </div>
  {/each}
</div>

<style>
  .toast {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(20px);
  }
</style>
