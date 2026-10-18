import { writable } from 'svelte/store';

/** Desktop sidebar hidden or shown; remembered per browser. */
function createSidebar() {
  let initial = false;
  try { initial = typeof localStorage !== 'undefined' && localStorage.getItem('sidebar') === 'collapsed'; } catch { /* storage blocked */ }
  const { subscribe, update } = writable(initial);
  return {
    subscribe,
    toggle: () => update(v => {
      try { localStorage.setItem('sidebar', !v ? 'collapsed' : 'open'); } catch { /* storage blocked */ }
      return !v;
    })
  };
}

export const sidebarCollapsed = createSidebar();
