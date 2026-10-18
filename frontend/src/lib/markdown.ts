import { Marked } from 'marked';
import DOMPurify from 'dompurify';

const md = new Marked();

/** Turn "[1]" into a citation mark everywhere (paragraphs, lists, tables), but not inside code. */
function markCitations(html: string): string {
  return html
    .split(/(<pre[\s\S]*?<\/pre>|<code[\s\S]*?<\/code>)/)
    .map((part, i) => (i % 2 ? part : part.replace(/\[(\d+)\]/g, '<sup class="citation-ref">[$1]</sup>')))
    .join('');
}

export function renderMarkdown(text: string): string {
  return DOMPurify.sanitize(markCitations(md.parse(text) as string), { ADD_ATTR: ['target', 'rel'] });
}
