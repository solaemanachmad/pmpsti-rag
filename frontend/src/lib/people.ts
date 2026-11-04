// Pages about one person follow the title convention "<Role> — <Name> — <Topic>", e.g.
// "Dosen — Sunu Wibirama — Penelitian" or "Dosen — Guntur D. Putra". The name in the title
// is also what the embedding and the chat source card see, so pages of different lecturers
// stay apart. Admin → Documents uses this to group a category's pages per person.

/** Roles that start a person page title (compared case-insensitively). */
export const PERSON_ROLES = ['dosen', 'tendik', 'tenaga kependidikan', 'lecturer'];

/** Only the em dash separates the parts. Website page titles use an en dash before the site
 *  name ("Dosen – jteti ft ugm"), and plain hyphens appear inside names and topics. */
const SEPARATOR = /\s+—\s+/;

/**
 * The person a page is about, or null when the title does not follow the convention.
 * "Dosen — Sunu Wibirama — Penelitian" → "Sunu Wibirama"
 */
export function personFromTitle(title: string | null | undefined): string | null {
  const parts = (title ?? '').trim().split(SEPARATOR).map(p => p.trim());
  if (parts.length < 2 || !PERSON_ROLES.includes(parts[0].toLowerCase())) return null;
  const name = parts[1].replace(/\s+/g, ' ');
  return name || null;
}

/** Group key for a person: the same name with different case or spacing is one person. */
export function personKey(name: string): string {
  return name.toLowerCase().replace(/[.,]/g, '').replace(/\s+/g, ' ').trim();
}
