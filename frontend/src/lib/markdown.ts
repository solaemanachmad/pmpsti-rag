import { Marked } from 'marked';
import DOMPurify from 'dompurify';
import type { SourceRef } from '$lib/api/client';

const md = new Marked();

function escapeAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Turn "[1]" into a citation mark everywhere (paragraphs, lists, tables), but not inside code.
 * With `sources`, a citation of a source that has a web address becomes a link (opens in a new
 * tab) and carries `data-cite` so the chat can show a preview on hover or tap.
 */
function markCitations(html: string, sources?: SourceRef[]): string {
  return html
    .split(/(<pre[\s\S]*?<\/pre>|<code[\s\S]*?<\/code>|<a[\s>][\s\S]*?<\/a>)/)
    .map((part, i) => (i % 2 ? part : part.replace(/\[(\d+)\]/g, (whole, n: string) => {
      const src = sources?.[Number(n) - 1];
      if (!src) return `<sup class="citation-ref">${whole}</sup>`;
      const url = src.source_url?.startsWith('http') ? src.source_url : '';
      const label = escapeAttr(`Source ${n}: ${src.title || 'document'}`);
      return url
        ? `<sup class="citation-ref"><a href="${escapeAttr(url)}" target="_blank" rel="noopener noreferrer" data-cite="${n}" aria-label="${label}">[${n}]</a></sup>`
        : `<sup class="citation-ref"><span tabindex="0" role="button" data-cite="${n}" aria-label="${label}">[${n}]</span></sup>`;
    })))
    .join('');
}

export function renderMarkdown(text: string, sources?: SourceRef[]): string {
  return DOMPurify.sanitize(markCitations(md.parse(text) as string, sources), { ADD_ATTR: ['target', 'rel'] });
}
