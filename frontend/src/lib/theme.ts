import { writable } from 'svelte/store';

function currentlyDark(): boolean {
  return typeof document !== 'undefined' && document.documentElement.classList.contains('dark');
}

function createTheme() {
  const { subscribe, set } = writable(currentlyDark());

  function apply(dark: boolean) {
    document.documentElement.classList.toggle('dark', dark);
    try { localStorage.setItem('theme', dark ? 'dark' : 'light'); } catch { /* storage blocked */ }
    set(dark);
  }

  return {
    subscribe,
    /** Re-read the class set by the inline script in app.html. */
    sync: () => set(currentlyDark()),
    toggle: () => apply(!currentlyDark())
  };
}

export const isDark = createTheme();
