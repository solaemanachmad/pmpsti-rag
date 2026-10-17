import { Marked } from 'marked';
import DOMPurify from 'dompurify';

// Own instance so the citation renderer is configured once, not per page.
const md = new Marked({
  renderer: {
    paragraph(token) {
      const html = this.parser.parseInline(token.tokens ?? []);
      return `<p>${html.replace(/\[(\d+)\]/g, '<sup class="citation-ref">[$1]</sup>')}</p>`;
    }
  }
});

export function renderMarkdown(text: string): string {
  return DOMPurify.sanitize(md.parse(text) as string, { ADD_ATTR: ['target', 'rel'] });
}
