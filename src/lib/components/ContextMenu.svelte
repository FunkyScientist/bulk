<script lang="ts">
  import { tick } from 'svelte';
  import { scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { app } from '$lib/app.svelte';

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: 0, y: 0 });
  let active = $state(-1);

  // Keep the menu on screen: flip left/up when it would overflow.
  $effect(() => {
    const m = app.menu;
    if (!m) return;
    active = -1;
    pos = { x: m.x, y: m.y };
    tick().then(() => {
      if (!el) return;
      const r = el.getBoundingClientRect();
      pos = {
        x: m.x + r.width > innerWidth - 8 ? m.x - r.width : m.x,
        y: m.y + r.height > innerHeight - 8 ? Math.max(8, m.y - r.height) : m.y,
      };
      el.focus();
    });
  });

  function close() {
    app.menu = null;
  }

  function onKey(e: KeyboardEvent) {
    const n = app.menu?.items.length ?? 0;
    if (e.key === 'Escape') close();
    else if (e.key === 'ArrowDown') active = (active + 1) % n;
    else if (e.key === 'ArrowUp') active = (active - 1 + n) % n;
    else if (e.key === 'Enter' && active >= 0) {
      app.menu?.items[active].run();
      close();
    } else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

{#if app.menu}
  <div
    class="fixed inset-0 z-[70]"
    role="presentation"
    onpointerdown={close}
    oncontextmenu={(e) => {
      e.preventDefault();
      close();
    }}
  ></div>
  <div
    bind:this={el}
    class="menu fixed z-[70] min-w-[200px] rounded-xl border border-line-strong p-1 outline-none"
    style="left: {pos.x}px; top: {pos.y}px"
    role="menu"
    tabindex="-1"
    onkeydown={onKey}
    transition:scale={{ start: 0.95, duration: 120, easing: cubicOut }}
  >
    {#each app.menu.items as item, i (item.label)}
      <button
        class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[12.5px] {item.danger
          ? 'text-danger'
          : ''} {active === i ? 'bg-hover' : ''}"
        role="menuitem"
        onpointerenter={() => (active = i)}
        onclick={() => {
          item.run();
          close();
        }}
      >
        {#if item.icon}<item.icon class="size-3.5 shrink-0 text-fg-muted" />{/if}
        <span class="flex-1">{item.label}</span>
        {#if item.hint}<span class="text-[11px] text-fg-subtle">{item.hint}</span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .menu {
    background: var(--popover);
    box-shadow: var(--shadow);
    backdrop-filter: blur(20px);
    transform-origin: top left;
  }
</style>
