"""Split per-page markdown into chunks that never cross a page boundary.

- Headings (`#`, `##`, `###`, or a bold-only line such as `**B. Staf Pengajar**`) update a
  heading path that carries over to the next pages and goes into the chunk title.
- Paragraphs are packed up to MAX_CHARS.
- Markdown tables are their own chunks (kind 'table'); long tables are split into row groups,
  each repeating the header row, so every chunk keeps its column names.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field

MAX_CHARS = 1200
MIN_CHARS = 40

_HEADING = re.compile(r"^(#{1,4})\s+(.*\S)\s*$")
_BOLD_LINE = re.compile(r"^\*\*([^*]{3,120})\*\*\s*$")
_TABLE_SEP = re.compile(r"^\|?\s*:?-{3,}")


@dataclass
class Chunk:
    page: int
    kind: str                       # text | table | figure
    content: str
    heading_path: list[str] = field(default_factory=list)

    def title(self, doc_title: str) -> str:
        path = " > ".join(self.heading_path)
        return f"{doc_title} — {path}" if path else doc_title


def clean_heading(text: str) -> str:
    text = re.sub(r"</?u>", "", text)
    return re.sub(r"[*_`•]+", "", text).strip(" .:")


def heading_level(text: str, markdown_level: int) -> int:
    """Outline level from the numbering used in Indonesian guidebooks: I. > A. > 1. > other."""
    if re.match(r"^[IVX]+\.\s", text):
        return 1
    if re.match(r"^[A-H]\.\s", text):
        return 2
    if re.match(r"^\d+\.\s", text):
        return 3
    return max(markdown_level, 3) if markdown_level else 4


def _blocks(markdown: str) -> list[str]:
    """Paragraph blocks; consecutive table lines stay together as one block."""
    blocks, cur, in_table = [], [], False
    for line in markdown.splitlines():
        is_table = line.lstrip().startswith("|")
        if not line.strip():
            if cur and not in_table:
                blocks.append("\n".join(cur)); cur = []
            continue
        if cur and is_table != in_table:
            blocks.append("\n".join(cur)); cur = []
        in_table = is_table
        cur.append(line.rstrip())
    if cur:
        blocks.append("\n".join(cur))
    return blocks


def _split_table(block: str, caption: str) -> list[str]:
    lines = block.splitlines()
    if len(lines) >= 2 and _TABLE_SEP.match(lines[1].strip().lstrip("|")):
        header, rows = lines[:2], lines[2:]
    else:
        header, rows = [], lines
    prefix = (caption + "\n") if caption else ""
    out, cur = [], []
    base = len(prefix) + sum(len(h) + 1 for h in header)
    size = base
    for row in rows:
        if cur and size + len(row) + 1 > MAX_CHARS:
            out.append(prefix + "\n".join(header + cur)); cur, size = [], base
        cur.append(row); size += len(row) + 1
    if cur or not out:
        out.append(prefix + "\n".join(header + cur))
    return out


def chunk_pages(pages: list[tuple[int, str]], heading_path: list[str] | None = None) -> list[Chunk]:
    """pages = [(page_number, markdown)] in order."""
    path: list[str] = list(heading_path or [])
    chunks: list[Chunk] = []

    for page_no, markdown in pages:
        buf: list[str] = []

        def flush():
            text = "\n\n".join(buf).strip()
            if len(text) >= MIN_CHARS:
                chunks.append(Chunk(page_no, "text", text, list(path)))
            elif text and chunks and chunks[-1].page == page_no and chunks[-1].kind == "text":
                chunks[-1].content += "\n\n" + text      # tiny tail: merge into previous chunk
            buf.clear()

        blocks = _blocks(markdown)
        skip: set[int] = set()
        for bi, block in enumerate(blocks):
            if bi in skip:
                continue
            first = block.splitlines()[0]
            m = _HEADING.match(first) or (_BOLD_LINE.match(first) if "\n" not in block else None)
            if m and m.re is _BOLD_LINE and _looks_like_sentence(m.group(1)):
                m = None
            if m and not block.lstrip().startswith("|"):
                text = clean_heading(m.group(2) if m.re is _HEADING else m.group(1))
                if text:
                    flush()
                    level = heading_level(text, len(m.group(1)) if m.re is _HEADING else 0)
                    del path[level - 1:]
                    path.append(text)
                rest = "\n".join(block.splitlines()[1:]).strip()
                if rest:
                    buf.append(rest)
                continue
            if block.lstrip().startswith("|"):
                flush()
                # a caption line ("Tabel 3.1 …") right after the table belongs to it
                nxt = blocks[bi + 1] if bi + 1 < len(blocks) else ""
                caption = nxt if re.match(r"^\**\s*Tabel\s+\d", nxt, re.I) and len(nxt) < 200 else ""
                if caption:
                    skip.add(bi + 1)
                for part in _split_table(block, clean_heading(caption)):
                    chunks.append(Chunk(page_no, "table", part, list(path)))
                continue
            if sum(len(b) for b in buf) + len(block) > MAX_CHARS:
                flush()
            if len(block) > MAX_CHARS:            # one huge paragraph: cut at sentence ends
                for piece in _split_long(block):
                    buf.append(piece); flush()
            else:
                buf.append(block)
        flush()
    return chunks


def _split_long(text: str) -> list[str]:
    sentences = re.split(r"(?<=[.;:])\s+", text)
    out, cur = [], ""
    for s in sentences:
        if cur and len(cur) + len(s) + 1 > MAX_CHARS:
            out.append(cur); cur = ""
        cur = f"{cur} {s}".strip()
    if cur:
        out.append(cur)
    return out


def _looks_like_sentence(text: str) -> bool:
    """A bold line that is a list item or sentence ("**2. Tidak diperkenankan cuti …**"), not a heading."""
    t = text.strip()
    if re.match(r"^\d+[.)]\s", t) and len(t.split()) > 5:   # "1. Minimal diajukan pada awal semester 3"
        return True
    return t.endswith((".", ",", ";")) or (t.endswith(":") and len(t) > 60) or len(t) > 90


_SUP_FRACTION = re.compile(r"(\d)<sup>(\d+)</sup>\s*/\s*(\d+)")
_TAGS = re.compile(r"</?(sup|sub|br)\s*/?>", re.I)


def clean_page_markdown(markdown: str, page_number: int) -> str:
    """Remove the printed page number (running header/footer) and inline HTML tags."""
    lines = [ln for ln in markdown.splitlines() if ln.strip() != str(page_number)]
    text = "\n".join(lines)
    text = _SUP_FRACTION.sub(r"\1 \2/\3", text)        # "1<sup>1</sup> /2" -> "1 1/2"
    return _TAGS.sub(" ", text)
