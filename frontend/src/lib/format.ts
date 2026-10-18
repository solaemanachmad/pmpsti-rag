const LOCALE = 'en-GB';

export function formatDate(iso: string | null | undefined, withYear = true): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return '—';
  return d.toLocaleDateString(LOCALE, { day: 'numeric', month: 'short', ...(withYear ? { year: 'numeric' } : {}) });
}

export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return '—';
  return d.toLocaleString(LOCALE, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function hostname(url: string | undefined): string {
  if (!url) return '';
  try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return ''; }
}

/** "3, 5–7, 10" — compact page list, same format as the backend's citations. */
export function formatPages(pages: number[] | undefined): string {
  if (!pages?.length) return '';
  const parts: string[] = [];
  for (let i = 0; i < pages.length; i++) {
    const start = pages[i];
    let end = start;
    while (i + 1 < pages.length && pages[i + 1] === end + 1) end = pages[++i];
    parts.push(end > start ? `${start}–${end}` : `${start}`);
  }
  return parts.join(', ');
}
