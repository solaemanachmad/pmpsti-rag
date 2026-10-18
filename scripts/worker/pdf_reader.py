"""Read a PDF page by page: markdown text (deterministic), figure candidates, number check."""
from __future__ import annotations

import re
from dataclasses import dataclass

import pymupdf
import pymupdf4llm

FIGURE_MIN_AREA = 0.10     # images covering >= 10 % of the page are worth describing
COVER_MIN_AREA = 0.85      # ... but a full-page image with little text is a cover
COVER_MAX_TEXT = 300


@dataclass
class Page:
    number: int              # 1-based
    markdown: str            # pymupdf4llm output (tables as markdown)
    raw_text: str            # plain text layer, for the number check
    image_area: float        # share of the page covered by images
    is_figure_candidate: bool


def _image_area(page: pymupdf.Page) -> float:
    total = page.rect.width * page.rect.height or 1.0
    area = 0.0
    for img in page.get_images(full=True):
        for r in page.get_image_rects(img[0]):
            area += max(0.0, (r & page.rect).width * (r & page.rect).height)
    return min(area / total, 1.0)


def read_pdf(data: bytes) -> tuple[list[Page], dict]:
    """Return the pages and document metadata (title, page_count)."""
    doc = pymupdf.open(stream=data, filetype="pdf")
    try:
        chunks = pymupdf4llm.to_markdown(doc, page_chunks=True, show_progress=False)
        pages: list[Page] = []
        for i, page in enumerate(doc):
            md = chunks[i]["text"] if i < len(chunks) else ""
            raw = page.get_text("text")
            area = _image_area(page)
            is_cover = area >= COVER_MIN_AREA and len(raw.strip()) < COVER_MAX_TEXT
            pages.append(Page(
                number=i + 1,
                markdown=md,
                raw_text=raw,
                image_area=round(area, 3),
                is_figure_candidate=area >= FIGURE_MIN_AREA and not is_cover,
            ))
        meta = {"title": (doc.metadata or {}).get("title", "") or "", "page_count": doc.page_count}
        return pages, meta
    finally:
        doc.close()


def render_page_png(data: bytes, page_number: int, dpi: int = 110) -> bytes:
    doc = pymupdf.open(stream=data, filetype="pdf")
    try:
        return doc[page_number - 1].get_pixmap(dpi=dpi).tobytes("png")
    finally:
        doc.close()


# ── Quality check: numbers in the text layer that did not reach any chunk ─────

_NUM = re.compile(r"(?<![\w.,])\d+(?:[.,]\d+)?(?![\w])")


def numbers(text: str) -> set[str]:
    return {n.replace(",", ".") for n in _NUM.findall(text)}


def missing_numbers(raw_text: str, chunk_texts: list[str]) -> set[str]:
    """Numbers (e.g. '3,00', '36', '2016') on a page that appear in no chunk of that page.
    Page numbers printed as running headers/footers are ignored by the caller."""
    found: set[str] = set()
    for t in chunk_texts:
        found |= numbers(t)
    return numbers(raw_text) - found
