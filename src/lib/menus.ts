import ShoppingCart from '@lucide/svelte/icons/shopping-cart';
import FolderOpen from '@lucide/svelte/icons/folder-open';
import ExternalLink from '@lucide/svelte/icons/external-link';
import Copy from '@lucide/svelte/icons/copy';
import Focus from '@lucide/svelte/icons/scan';
import Undo from '@lucide/svelte/icons/undo-2';
import { api } from './api';
import { app, type MenuItem } from './app.svelte';

/** Right-click menu for a file or folder, shared by the list and the charts. */
export async function nodeMenu(e: MouseEvent, letter: string, id: number, extra: MenuItem[] = []) {
  e.preventDefault();
  const at = { clientX: e.clientX, clientY: e.clientY, preventDefault() {} } as MouseEvent;
  const info = await api.nodeInfo(letter, id);
  const inCart = app.inCart(info.path);
  const items: MenuItem[] = [];
  if (info.parent !== null) {
    items.push(
      inCart
        ? {
            label: 'Remove from cleanup cart',
            icon: Undo,
            hint: 'Del',
            run: () => app.removeFromCart([info.path]),
          }
        : {
            label: 'Add to cleanup cart',
            icon: ShoppingCart,
            hint: 'Del',
            run: () => app.addToCart(letter, [id]),
          },
    );
  }
  if (info.dir)
    items.push({
      label: 'Show in chart',
      icon: Focus,
      hint: 'Enter',
      run: () => app.setFocus(letter, info.parent === null ? null : id),
    });
  else items.push({ label: 'Open', icon: ExternalLink, run: () => api.openPath(info.path) });
  items.push(
    {
      label: 'Show in Explorer',
      icon: FolderOpen,
      hint: 'Ctrl E',
      run: () => api.showInExplorer(info.path),
    },
    {
      label: 'Copy path',
      icon: Copy,
      run: async () => {
        await navigator.clipboard.writeText(info.path);
        app.toast('Path copied');
      },
    },
    ...extra,
  );
  app.openMenu(at, items);
}
