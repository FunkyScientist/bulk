<script lang="ts">
  import type { Snippet } from 'svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';

  let {
    children,
    content,
    side = 'right',
    disabled = false,
    delay = 300,
    class: cls = '',
  }: {
    children: Snippet;
    content: Snippet | string;
    side?: 'right' | 'bottom';
    disabled?: boolean;
    delay?: number;
    class?: string;
  } = $props();

  let trigger: HTMLDivElement;
  let open = $state(false);
  let pos = $state({ x: 0, y: 0 });
  let timer: ReturnType<typeof setTimeout> | undefined;

  function show() {
    if (disabled) return;
    clearTimeout(timer);
    timer = setTimeout(() => {
      const r = trigger.getBoundingClientRect();
      pos =
        side === 'right'
          ? { x: r.right + 10, y: r.top + r.height / 2 }
          : { x: r.left + r.width / 2, y: r.bottom + 8 };
      open = true;
    }, delay);
  }

  function hide() {
    clearTimeout(timer);
    open = false;
  }

  $effect(() => {
    if (disabled) hide();
  });
</script>

<svelte:window onblur={hide} onwheel={hide} />

<div
  bind:this={trigger}
  class={cls}
  role="presentation"
  onpointerenter={show}
  onpointerleave={hide}
  onpointerdown={hide}
  onfocusin={show}
  onfocusout={hide}
>
  {@render children()}
</div>

{#if open}
  <div
    class="tip pointer-events-none fixed z-50 max-w-[280px] rounded-lg border border-line px-2.5 py-1.5 text-[12px]"
    style="left: {pos.x}px; top: {pos.y}px; transform: {side === 'right'
      ? 'translateY(-50%)'
      : 'translateX(-50%)'}"
    role="tooltip"
    transition:fly={{
      x: side === 'right' ? -4 : 0,
      y: side === 'bottom' ? -4 : 0,
      duration: 140,
      easing: cubicOut,
    }}
  >
    {#if typeof content === 'string'}{content}{:else}{@render content()}{/if}
  </div>
{/if}

<style>
  .tip {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(14px);
  }
</style>
