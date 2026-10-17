import { writable } from 'svelte/store';
import { chat } from '$lib/api/client';
import type { SessionListItem } from '$lib/api/client';

// Shared between the sidebar (history list) and the chat page.
export const sessions = writable<SessionListItem[]>([]);
export const activeSessionId = writable<string | undefined>(undefined);

export async function refreshSessions() {
  try { sessions.set(await chat.sessions()); } catch { sessions.set([]); }
}

export async function renameSession(id: string, title: string) {
  await chat.renameSession(id, title);
  sessions.update(list => list.map(s => (s.id === id ? { ...s, title } : s)));
}

export async function removeSession(id: string) {
  await chat.deleteSession(id);
  sessions.update(list => list.filter(s => s.id !== id));
}
