import type { TemplateData, TemplateMode } from './api';
import { setFileColors } from './palette';

type Mode = 'dark' | 'light';

/** UI colours a template can set, as CSS custom properties (`--name`). */
export const UI_KEYS = [
  'app-bg',
  'app-tint',
  'sidebar-tint',
  'panel',
  'panel-2',
  'popover',
  'hover',
  'line',
  'line-strong',
  'fg',
  'fg-muted',
  'fg-subtle',
  'switch-off',
  'accent',
  'accent-fg',
  'accent-soft',
  'danger',
  'ok',
  'warn',
  'shadow',
  'usage',
  'usage-low',
  'usage-full',
  'stale',
  'live',
  'added',
  'removed',
  'grew',
  'shrank',
  'cart',
  'danger-fg',
  'switch-knob',
  'close-hover',
  'close-fg',
  'logo-1',
  'logo-2',
  'logo-3',
  'logo-4',
  'bar-end',
  'backdrop',
  'chart-folder',
  'chart-hatch',
  'chart-hover',
  'chart-hover-line',
  'chart-select',
  'chart-label',
  'chart-label-folder',
] as const;

/** One mode of a template, with anything it leaves out taken from `base`. */
export function resolve(
  t: TemplateData | null | undefined,
  base: TemplateData | null | undefined,
  mode: Mode,
): Required<TemplateMode> {
  const a = t?.[mode] ?? {};
  const b = base?.[mode] ?? {};
  return {
    ui: { ...b.ui, ...a.ui },
    files: { ...b.files, ...a.files },
    branches: a.branches?.length ? a.branches : (b.branches ?? []),
  };
}

/** Use a template: CSS variables for the UI, and colours for the charts. Values
 * the template doesn't give fall back to the stylesheet (and the built-in palette). */
export function apply(m: Required<TemplateMode>) {
  const style = document.documentElement.style;
  for (const k of UI_KEYS) {
    const v = m.ui[k];
    if (typeof v === 'string' && v.trim()) style.setProperty(`--${k}`, v);
    else style.removeProperty(`--${k}`);
  }
  setFileColors(m.files, m.branches);
}
