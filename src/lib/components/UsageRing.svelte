<script lang="ts">
  import { Tween } from 'svelte/motion';
  import { cubicOut } from 'svelte/easing';

  let {
    value,
    size = 36,
    stroke = 3.5,
    active = false,
  }: { value: number; size?: number; stroke?: number; active?: boolean } = $props();

  const r = $derived((size - stroke) / 2);
  const c = $derived(2 * Math.PI * r);
  const shown = new Tween(0, { duration: 900, easing: cubicOut });
  $effect(() => {
    shown.target = Math.max(0, Math.min(1, value));
  });

  // Fills turn amber then red as a drive gets full.
  const color = $derived(
    value > 0.92 ? 'var(--usage-full)' : value > 0.8 ? 'var(--usage-low)' : 'var(--usage)',
  );
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 {size} {size}"
  class="-rotate-90"
  class:spinning={active}
>
  <circle
    cx={size / 2}
    cy={size / 2}
    {r}
    fill="none"
    stroke="var(--line-strong)"
    stroke-width={stroke}
  />
  <circle
    cx={size / 2}
    cy={size / 2}
    {r}
    fill="none"
    stroke={color}
    stroke-width={stroke}
    stroke-linecap="round"
    stroke-dasharray={c}
    stroke-dashoffset={c * (1 - shown.current)}
    style="transition: stroke 300ms ease"
  />
</svg>

<style>
  .spinning {
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.55;
    }
  }
</style>
