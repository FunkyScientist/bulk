<script lang="ts">
  import { tick } from 'svelte';
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import Funnel from '@lucide/svelte/icons/funnel';
  import X from '@lucide/svelte/icons/x';
  import { app } from '$lib/app.svelte';
  import { bytes, count } from '$lib/format';

  let { letter, total }: { letter: string; total: number } = $props();

  const info = $derived(app.filters[letter] ?? null);
  let text = $state('');
  let input: HTMLInputElement | undefined = $state();

  // Show the applied query when it changes (from Types, the palette, or here). Live
  // updates refresh `info` constantly; they mustn't overwrite what's being typed.
  let synced: string | null = null;
  $effect(() => {
    const q = info?.query ?? '';
    if (q !== synced) {
      synced = q;
      text = q;
    }
  });

  $effect(() => {
    if (app.filterFocus) tick().then(() => input?.select());
  });

  function apply() {
    const q = text.trim();
    if (q !== (info?.query ?? '')) void app.setFilter(letter, q || null);
  }

  function clear() {
    text = '';
    if (info) void app.setFilter(letter, null);
    app.filterOpen = false;
  }

  const examples = ['kind:video', 'size:>500MB', 'modified:>1y', 'ext:zip,rar,7z', '-in:appdata'];
</script>

<div
  class="flex shrink-0 items-center gap-2.5 border-b border-line px-4 py-1.5 text-[12px]"
  transition:slide={{ duration: 180, easing: cubicOut }}
>
  <Funnel class="size-3.5 shrink-0 {info ? 'text-accent' : 'text-fg-subtle'}" />
  <input
    bind:this={input}
    bind:value={text}
    class="h-7 min-w-0 flex-1 rounded-md border border-line bg-transparent px-2 font-mono text-[12px] outline-none placeholder:font-sans placeholder:text-fg-subtle focus:border-accent"
    placeholder="Show only matching files, like search: kind:video size:>100MB -in:appdata"
    spellcheck="false"
    autocomplete="off"
    onkeydown={(e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        apply();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        if (text.trim() && text !== (info?.query ?? '')) text = info?.query ?? '';
        else clear();
      }
    }}
  />
  {#if info}
    <span class="tabular shrink-0 text-fg-subtle">
      {count(info.files)} files · <span class="text-fg">{bytes(info.alloc)}</span>{total
        ? ` · ${((info.alloc / total) * 100).toFixed(info.alloc / total < 0.01 ? 2 : 1)}% of the drive`
        : ''}
    </span>
  {:else}
    <span class="hidden shrink-0 items-center gap-1 lg:flex">
      {#each examples as ex (ex)}
        <button
          class="pressable rounded-md border border-line px-1.5 py-0.5 font-mono text-[11px] text-fg-muted hover:border-line-strong hover:text-fg"
          onclick={() => {
            text = ex;
            apply();
          }}>{ex}</button
        >
      {/each}
    </span>
  {/if}
  {#if text.trim() !== (info?.query ?? '')}
    <button
      class="pressable h-7 shrink-0 rounded-md bg-accent px-2.5 text-[12px] font-medium text-accent-fg"
      onclick={apply}>Apply</button
    >
  {/if}
  <button
    class="pressable grid size-7 shrink-0 place-items-center rounded-md text-fg-subtle hover:bg-hover hover:text-fg"
    title={info ? 'Clear the filter (Esc)' : 'Close'}
    aria-label="Clear the filter"
    onclick={clear}><X class="size-4" /></button
  >
</div>
