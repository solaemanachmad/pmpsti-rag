"""
ingest.py  —  Pipeline ingest dokumen ke pmpsti_rag
====================================================
Mendukung: PDF, DOCX, dan Quarto/HTML site

Cara pakai:
    # Ingest semua PDF & DOCX di folder docs/
    python ingest.py --files

    # Ingest Quarto site dari URL
    python ingest.py --site-url https://your-quarto-site.com --category "panduan"

    # Ingest Quarto site dari folder _site/ lokal
    python ingest.py --site-local ./docs/_site --category "panduan"

    # Semua sekaligus
    python ingest.py --files --site-url https://your-quarto-site.com --category "panduan"
"""

import argparse
import json
import logging
import logging.handlers
import hashlib
import io
import os
import re
import sys
import time
import urllib.parse
from collections import Counter
from pathlib import Path

import docx
import docx.table
import docx.text.paragraph
import fitz
import psycopg2
import pymupdf4llm
import pytesseract
import requests
from bs4 import BeautifulSoup
from langchain_experimental.text_splitter import SemanticChunker
from langchain_huggingface import HuggingFaceEmbeddings
from langchain_text_splitters import MarkdownHeaderTextSplitter, RecursiveCharacterTextSplitter
from pgvector.psycopg2 import register_vector
from PIL import Image
from psycopg2.extras import execute_values
from text_enrichment import enrich_text, enrich_chunk
from gemini_embed import GeminiEmbeddings

# ──────────────────────────────────────────────────────────────────
#  KONFIGURASI
# ──────────────────────────────────────────────────────────────────

BASE_DIR = Path(__file__).resolve().parent.parent / "docs"
# Bisa di-override lewat env DATABASE_URL (mis. Neon) atau argumen --db
DB_URL   = os.environ.get("DATABASE_URL", "postgresql://admin:admin@localhost:5432/pmpsti_rag")

MIN_IMAGE_WIDTH  = 100
MIN_IMAGE_HEIGHT = 100

# Quarto HTML — selector konten utama (urutan prioritas)
QUARTO_CONTENT_SELECTORS = [
    "main#quarto-document-content",
    "main.content",
    "div#quarto-content",
    "article.content",
    "div.page-content",
    "main",
    "article",
]

# Elemen Quarto yang di-strip sebelum ekstrak teks
QUARTO_STRIP = [
    "nav", "header", "footer",
    "div.sidebar", "div#quarto-sidebar",
    "div#quarto-toc-sidebar", "div.toc-actions",
    "div#quarto-header", "div.navbar",
    "div.page-navigation", "div.nav-page",
    "button", "script", "style", "noscript",
    "div.announcement", "div.quarto-title-banner",
    "div#quarto-appendix",
]

if sys.platform == "win32":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# ──────────────────────────────────────────────────────────────────
#  EMBEDDING MODEL  (load sekali, dipakai bersama)
# ──────────────────────────────────────────────────────────────────

# 1) Embedding yang DISIMPAN ke DB → Gemini gemini-embedding-001 (768-dim).
#    WAJIB sama dengan model yang dipakai backend Rust untuk meng-embed pertanyaan.
embeddings_model = GeminiEmbeddings(task_type="RETRIEVAL_DOCUMENT")

# 2) Model lokal HANYA untuk SemanticChunker (menentukan batas chunk).
#    Vektornya tidak disimpan, jadi boleh beda model dan tidak memakan kuota Gemini.
#    Set CHUNKER_MODEL=gemini untuk memakai Gemini juga (lebih banyak request API).
_chunker_model = None

def get_chunker_model():
    global _chunker_model
    if _chunker_model is None:
        if os.environ.get("CHUNKER_MODEL", "local") == "gemini":
            _chunker_model = GeminiEmbeddings(task_type="SEMANTIC_SIMILARITY")
        else:
            print("Loading model lokal untuk chunking (multilingual-e5-base)...")
            _chunker_model = HuggingFaceEmbeddings(model_name="intfloat/multilingual-e5-base")
            print("[INFO] Model chunker siap.\n")
    return _chunker_model


# ══════════════════════════════════════════════════════════════════
#  DATABASE
# ══════════════════════════════════════════════════════════════════

def connect_db(url=None):
    conn = psycopg2.connect(url or DB_URL)
    conn.cursor().execute("CREATE EXTENSION IF NOT EXISTS vector;")
    conn.commit()
    register_vector(conn)
    return conn


def setup_database(conn):
    cursor = conn.cursor()
    cursor.execute("CREATE EXTENSION IF NOT EXISTS vector;")
    cursor.execute("""
        CREATE TABLE IF NOT EXISTS documents (
            id            SERIAL PRIMARY KEY,
            document_id   TEXT NOT NULL,
            title         TEXT,
            content       TEXT,
            document_type TEXT,
            source_url    TEXT,
            category      TEXT DEFAULT '',
            subcategory   TEXT DEFAULT '',
            file_hash     TEXT DEFAULT '',
            embedding     vector(768),
            page_number   INTEGER,
            chunk_index   INTEGER,
            created_at    TIMESTAMPTZ DEFAULT NOW(),
            updated_at    TIMESTAMPTZ DEFAULT NOW()
        );
    """)
    cursor.execute("""
        CREATE INDEX IF NOT EXISTS documents_embedding_idx
        ON documents USING hnsw (embedding vector_cosine_ops);
    """)
    cursor.execute("""
        CREATE INDEX IF NOT EXISTS documents_fts_idx
        ON documents USING gin (to_tsvector('indonesian', content));
    """)
    # Index tambahan untuk performa incremental ingest
    # UNIQUE pada (document_id, chunk_index) karena 1 dokumen = banyak chunks
    cursor.execute("""
        CREATE UNIQUE INDEX IF NOT EXISTS documents_doc_chunk_key
        ON documents(document_id, chunk_index);
    """)
    cursor.execute("""
        CREATE INDEX IF NOT EXISTS documents_category_idx
        ON documents(category, subcategory);
    """)
    cursor.execute("""
        CREATE INDEX IF NOT EXISTS documents_source_url_idx
        ON documents(source_url);
    """)
    # Safe migrations untuk database yang sudah ada
    migrations = [
        "ALTER TABLE documents ALTER COLUMN document_id SET NOT NULL",
        "ALTER TABLE documents ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ DEFAULT NOW()",
        "ALTER TABLE documents ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT NOW()",
        "CREATE UNIQUE INDEX IF NOT EXISTS documents_doc_chunk_key ON documents(document_id, chunk_index)",
        "CREATE INDEX IF NOT EXISTS documents_category_idx ON documents(category, subcategory)",
        "CREATE INDEX IF NOT EXISTS documents_source_url_idx ON documents(source_url)",
    ]
    for sql in migrations:
        try:
            cursor.execute(sql)
        except Exception as e:
            conn.rollback()
            print(f"[WARN] Migration skipped: {e}")
    conn.commit()
    cursor.close()
    print("[INFO] Database schema & index ready.")


def print_db_stats(conn):
    cursor = conn.cursor()
    print("\n" + "=" * 50)
    print("  OUTPUT_DB: Statistics Docs in PostgreSQL")
    print("=" * 50)

    cursor.execute("SELECT COUNT(*) FROM documents;")
    total_chunks = cursor.fetchone()[0]
    cursor.execute("SELECT COUNT(DISTINCT document_id) FROM documents;")
    total_docs = cursor.fetchone()[0]
    cursor.execute("SELECT category, COUNT(*) FROM documents GROUP BY category ORDER BY count DESC;")
    categories = cursor.fetchall()

    print(f"Total Dokumen Unik  : {total_docs}")
    print(f"Total Chunks        : {total_chunks}")
    if categories:
        print("\nDistribusi per Kategori:")
        for cat, count in categories:
            print(f"  - {cat or 'General'}: {count} chunks")
    print("=" * 50 + "\n")
    cursor.close()


def delete_document(conn, document_id: str):
    with conn.cursor() as cur:
        cur.execute("DELETE FROM documents WHERE document_id = %s", (document_id,))
    conn.commit()


def get_existing_hash(conn, document_id: str) -> str | None:
    with conn.cursor() as cur:
        cur.execute("SELECT file_hash FROM documents WHERE document_id = %s LIMIT 1", (document_id,))
        row = cur.fetchone()
    return row[0] if row else None


def bulk_insert(conn, records: list[list]):
    """records: list of [doc_id, title, content, type, url, cat, subcat, hash, embedding, page, idx]"""
    insert_query = """
        INSERT INTO documents
            (document_id, title, content, document_type, source_url,
             category, subcategory, file_hash, embedding, page_number, chunk_index,
             created_at, updated_at)
        VALUES %s
    """
    import datetime
    now = datetime.datetime.now(datetime.timezone.utc)
    # Append created_at dan updated_at ke setiap record
    records_with_ts = [list(r) + [now, now] for r in records]
    with conn.cursor() as cur:
        execute_values(cur, insert_query, records_with_ts)
    conn.commit()


# ══════════════════════════════════════════════════════════════════
#  CHUNKING PIPELINE  (shared antara file & site)
# ══════════════════════════════════════════════════════════════════

def build_splitters():
    markdown_splitter = MarkdownHeaderTextSplitter(headers_to_split_on=[
        ("#", "Chapter"), ("##", "Subchapter"), ("###", "Section")
    ])
    semantic_chunker = SemanticChunker(
        get_chunker_model(),
        breakpoint_threshold_type="percentile"
    )
    fallback_splitter = RecursiveCharacterTextSplitter(
        chunk_size=1500,
        chunk_overlap=150
    )
    return markdown_splitter, semantic_chunker, fallback_splitter


def markdown_to_chunks(md_text: str, splitters: tuple) -> list:
    """Jalankan MarkdownHeader → Semantic → Fallback pipeline."""
    markdown_splitter, semantic_chunker, fallback_splitter = splitters

    chapter_chunks = markdown_splitter.split_text(md_text)
    final_chunks = []

    for chunk in chapter_chunks:
        try:
            content = chunk.page_content if hasattr(chunk, "page_content") else str(chunk)
            semantic_splits = semantic_chunker.create_documents([content])
            for sem_chunk in semantic_splits:
                if hasattr(chunk, "metadata"):
                    sem_chunk.metadata.update(chunk.metadata)
                if len(sem_chunk.page_content) > 1500:
                    final_chunks.extend(fallback_splitter.split_documents([sem_chunk]))
                else:
                    final_chunks.append(sem_chunk)
        except Exception as e:
            print(f"  [Skip Chunk] {e}")

    return final_chunks


def chunks_to_records(
    final_chunks: list,
    document_id: str,
    doc_type: str,
    source_url: str,
    category: str,
    subcategory: str,
    file_hash: str,
) -> tuple[list[str], list[list]]:
    """
    Kembalikan (texts_to_embed, db_records).
    db_records[i][8] = None  → diisi embedding setelah batch embed.
    """
    texts_to_embed = []
    db_records = []

    for chunk_index, doc in enumerate(final_chunks):
        text_content = doc.page_content.strip()
        if len(text_content) < 20:
            continue

        metadata = getattr(doc, "metadata", {})
        hierarchy = [
            str(metadata.get(h, ""))
            for h in ["Chapter", "Subchapter", "Section"]
            if metadata.get(h)
        ]
        hierarchy_str = " > ".join(hierarchy)

        # Teks + breadcrumb untuk embedding (tanpa prefix E5 — tidak dipakai Gemini)
        text_for_embed = enrich_chunk(text_content, metadata, document_id)
        title = f"{document_id} ({hierarchy_str})" if hierarchy_str else document_id

        texts_to_embed.append(text_for_embed)
        db_records.append([
            document_id, title, text_content, doc_type, source_url,
            category, subcategory, file_hash,
            None,  # embedding — diisi nanti
            metadata.get("page"), chunk_index
        ])

    return texts_to_embed, db_records


def embed_and_insert(conn, texts_to_embed: list[str], db_records: list[list]):
    if not texts_to_embed:
        return
    print(f"  Embedding {len(texts_to_embed)} chunks (batch)...")
    vectors = embeddings_model.embed_documents(texts_to_embed)
    for i, vec in enumerate(vectors):
        db_records[i][8] = vec
    bulk_insert(conn, db_records)
    print(f"  [OK] {len(db_records)} chunks tersimpan.")


# ══════════════════════════════════════════════════════════════════
#  PDF & DOCX EXTRACTION
# ══════════════════════════════════════════════════════════════════

def _ocr_image_bytes(img_bytes: bytes) -> str:
    try:
        image = Image.open(io.BytesIO(img_bytes)).convert("RGB")
        return pytesseract.image_to_string(image, lang="ind+eng").strip()
    except Exception:
        return ""


def extract_pdf_markdown(filepath: Path) -> str:
    try:
        doc = fitz.open(str(filepath))
        parts = []
        for page_num in range(len(doc)):
            page_md = pymupdf4llm.to_markdown(doc, pages=[page_num], show_warning=False)
            parts.append(page_md)
            for img_info in doc[page_num].get_images(full=True):
                base = doc.extract_image(img_info[0])
                if base["width"] < MIN_IMAGE_WIDTH or base["height"] < MIN_IMAGE_HEIGHT:
                    continue
                ocr = _ocr_image_bytes(base["image"])
                if ocr:
                    parts.append(f"\n\n> [OCR Page {page_num + 1}]:\n{ocr}\n")
        doc.close()
        return "\n".join(parts)
    except Exception as e:
        print(f"  [ERROR] PDF: {e}")
        return ""


def _table_to_markdown(table: docx.table.Table) -> str:
    rows = table.rows
    if not rows:
        return ""
    lines = []
    header = [c.text.replace("\n", " ").strip() for c in rows[0].cells]
    lines.append("| " + " | ".join(header) + " |")
    lines.append("| " + " | ".join("---" for _ in header) + " |")
    for row in rows[1:]:
        lines.append("| " + " | ".join(c.text.replace("\n", " ").strip() for c in row.cells) + " |")
    return "\n".join(lines)


def extract_docx_markdown(filepath: Path) -> str:
    try:
        doc_obj = docx.Document(str(filepath))
    except Exception as e:
        print(f"  [ERROR] DOCX: {e}")
        return ""

    image_texts = {}
    for rel in doc_obj.part.rels.values():
        if "image" in rel.reltype:
            try:
                t = _ocr_image_bytes(rel.target_part.blob)
                if t:
                    image_texts[rel.rId] = t
            except Exception:
                pass

    if image_texts:
        print(f"  [OCR] {len(image_texts)} gambar ditemukan di DOCX")

    lines = []
    for block in doc_obj.element.body:
        local = block.tag.split("}")[-1] if "}" in block.tag else block.tag
        if local == "p":
            para = docx.text.paragraph.Paragraph(block, doc_obj)
            text = para.text.strip()
            style = para.style.name if para.style else ""
            has_img = block.find(
                ".//{http://schemas.openxmlformats.org/drawingml/2006/main}blip"
            ) is not None

            if style.startswith("Heading 1") and text:   lines.append(f"\n# {text}")
            elif style.startswith("Heading 2") and text: lines.append(f"\n## {text}")
            elif style.startswith("Heading 3") and text: lines.append(f"\n### {text}")
            elif style.startswith("Heading 4") and text: lines.append(f"\n#### {text}")
            elif text:                                    lines.append(text)

            if has_img and image_texts:
                ocr = next(iter(image_texts.values()))
                lines.append(f"\n\n{ocr}\n")
                image_texts = {}

        elif local == "tbl":
            md = _table_to_markdown(docx.table.Table(block, doc_obj))
            if md:
                lines.append("\n" + md + "\n")

    if image_texts:
        lines.append("\n## Extracted Image Text")
        lines.extend(image_texts.values())

    return "\n\n".join(lines)


# ══════════════════════════════════════════════════════════════════
#  SITE (QUARTO/HTML) EXTRACTION
# ══════════════════════════════════════════════════════════════════

def extract_site_meta(soup: BeautifulSoup, url: str) -> dict:
    title = ""
    for sel in ["h1.title", "h1#title-block-header", "main h1", "h1"]:
        h1 = soup.select_one(sel)
        if h1:
            title = h1.get_text(strip=True)
            break
    if not title:
        t = soup.find("title")
        if t:
            title = t.get_text(strip=True).split("|")[0].strip()
    if not title:
        og = soup.find("meta", property="og:title")
        if og:
            title = og.get("content", "").strip()
    if not title:
        title = urllib.parse.urlparse(url).path.rstrip("/").split("/")[-1].replace("-", " ").title()
    return {"title": title or url}


def extract_site_markdown(soup: BeautifulSoup) -> str:
    """Ekstrak konten Quarto → teks berformat Markdown."""
    soup = BeautifulSoup(str(soup), "lxml")

    for sel in QUARTO_STRIP:
        for el in soup.select(sel):
            el.decompose()

    content_el = None
    for sel in QUARTO_CONTENT_SELECTORS:
        content_el = soup.select_one(sel)
        if content_el:
            break
    if content_el is None:
        content_el = soup.find("body") or soup

    # Konversi heading ke format Markdown
    for tag in content_el.find_all(["h1", "h2", "h3", "h4", "h5", "h6"]):
        level = int(tag.name[1])
        prefix = "#" * level
        tag.replace_with(f"\n\n{prefix} {tag.get_text(strip=True)}\n\n")

    # List
    for li in content_el.find_all("li"):
        li.replace_with(f"- {li.get_text(separator=' ', strip=True)}\n")

    # Tabel — termasuk baris yang di-hidden oleh DataTable pagination
    for table in content_el.find_all("table"):
        # Unhide semua <tr> yang disembunyikan DataTable (display:none)
        for tr in table.find_all("tr"):
            style = tr.get("style", "")
            if "display" in style and "none" in style:
                del tr["style"]  # unhide agar ikut diproses

        # Ambil semua rows — thead + tbody (termasuk yang tadinya hidden)
        rows = table.find_all("tr")
        if not rows:
            table.decompose()
            continue

        md_rows = []
        header_done = False
        for row in rows:
            cells = [td.get_text(separator=" ", strip=True) for td in row.find_all(["th", "td"])]
            if not any(cells):
                continue
            md_rows.append("| " + " | ".join(cells) + " |")
            if not header_done:
                md_rows.append("| " + " | ".join("---" for _ in cells) + " |")
                header_done = True

        table.replace_with("\n" + "\n".join(md_rows) + "\n")

    text = content_el.get_text(separator="\n")
    text = re.sub(r"\n{3,}", "\n\n", text)
    text = re.sub(r"[ \t]+", " ", text)
    return text.strip()


class SiteCrawler:
    """
    Crawl Quarto site dari URL.

    Strategi (urutan prioritas):
      1. sitemap.xml  — Quarto selalu generate ini, paling lengkap & cepat
      2. Link-follow  — fallback jika sitemap tidak ada
    """

    SKIP_EXT = (".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico",
                ".css", ".js", ".woff", ".woff2", ".ttf", ".eot",
                ".zip", ".pdf", ".docx", ".xlsx", ".mp4", ".mp3")

    def __init__(self, base_url: str, max_pages: int = 500):
        # Kalau user kasih URL sitemap langsung, ambil parent-nya sebagai base
        if base_url.endswith(".xml"):
            base_url = base_url.rsplit("/", 1)[0]
        self.base_url  = base_url.rstrip("/")
        self.base_host = urllib.parse.urlparse(base_url).netloc
        self.max_pages = max_pages
        self.session   = requests.Session()
        self.session.headers["User-Agent"] = "pmpsti-rag-ingest/1.0"

    def _valid_url(self, url: str) -> bool:
        p = urllib.parse.urlparse(url)
        if p.netloc and p.netloc != self.base_host:
            return False
        return not any(p.path.lower().endswith(e) for e in self.SKIP_EXT)

    def _normalize(self, url: str, base: str) -> str | None:
        url = urllib.parse.urljoin(base, url).split("#")[0].split("?")[0].rstrip("/")
        return url if url.startswith(self.base_url) else None

    def _fetch(self, url: str):
        try:
            resp = self.session.get(url, timeout=15)
            if resp.status_code != 200:
                return None
            if "text/html" not in resp.headers.get("content-type", ""):
                return None
            return BeautifulSoup(resp.text, "lxml")
        except Exception as e:
            print(f"  [err] {url} -> {e}")
            return None

    def _urls_from_sitemap(self, sitemap_url: str) -> list:
        """Baca sitemap.xml (termasuk sitemap index yang nesting)."""
        urls = []
        try:
            resp = self.session.get(sitemap_url, timeout=15)
            if resp.status_code != 200:
                return []
            from xml.etree import ElementTree as ET
            ns = {"sm": "http://www.sitemaps.org/schemas/sitemap/0.9"}
            root = ET.fromstring(resp.text)
            # Sitemap index
            for sitemap_el in root.findall("sm:sitemap", ns):
                loc = sitemap_el.findtext("sm:loc", namespaces=ns)
                if loc:
                    urls.extend(self._urls_from_sitemap(loc))
            # URL list
            for url_el in root.findall("sm:url", ns):
                loc = url_el.findtext("sm:loc", namespaces=ns)
                if loc and self._valid_url(loc) and loc.startswith(self.base_url):
                    urls.append(loc.rstrip("/"))
        except Exception as e:
            print(f"  [sitemap err] {sitemap_url} -> {e}")
        return urls

    def crawl(self):
        # 1. Coba sitemap — cek beberapa kandidat URL
        all_urls = []
        sitemap_candidates = []

        # Kalau base_url sudah menunjuk langsung ke sitemap
        if self.base_url.endswith(".xml"):
            sitemap_candidates.append(self.base_url)
        else:
            sitemap_candidates += [
                f"{self.base_url}/sitemap.xml",
                f"{self.base_url}/sitemap-index.xml",
                f"{self.base_url}/wp-sitemap.xml",        # WordPress 5.5+
                f"{self.base_url}/news-sitemap.xml",      # Yoast News
                f"{self.base_url}/page-sitemap.xml",      # Yoast Pages
                f"{self.base_url}/post-sitemap.xml",      # Yoast Posts
            ]

        for sm_url in sitemap_candidates:
            found = self._urls_from_sitemap(sm_url)
            if found:
                all_urls = list(dict.fromkeys(found))
                print(f"  [sitemap] {len(all_urls)} URL ditemukan dari {sm_url}")
                break

        if all_urls:
            for url in all_urls[:self.max_pages]:
                soup = self._fetch(url)
                if soup is None:
                    print(f"  [skip] {url}")
                    continue
                print(f"  [ok]  {url}")
                yield url, soup
                time.sleep(0.2)
        else:
            # 2. Fallback: BFS link-follow
            print("  [info] Sitemap tidak ditemukan, pakai BFS link-follow...")
            visited = set()
            queue   = [self.base_url]
            while queue and len(visited) < self.max_pages:
                url = queue.pop(0)
                if url in visited:
                    continue
                visited.add(url)
                soup = self._fetch(url)
                if soup is None:
                    continue
                for a in soup.find_all("a", href=True):
                    n = self._normalize(a["href"], url)
                    if n and n not in visited and self._valid_url(n):
                        queue.append(n)
                print(f"  [ok]  {url}")
                yield url, soup
                time.sleep(0.3)


class LocalSiteCrawler:
    """Crawl folder _site/ hasil quarto render."""
    def __init__(self, site_dir: str, base_url: str = "https://localhost"):
        self.site_dir = Path(site_dir)
        self.base_url = base_url.rstrip("/")

    def crawl(self):
        for html_file in sorted(self.site_dir.rglob("*.html")):
            if html_file.name in ("404.html", "search.html"):
                continue
            rel = html_file.relative_to(self.site_dir)
            url = (self.base_url + "/" + str(rel).replace("\\", "/"))
            url = url.replace("/index.html", "").replace(".html", "")
            try:
                soup = BeautifulSoup(html_file.read_text(encoding="utf-8"), "lxml")
                print(f"  [ok]  {url}")
                yield url, soup
            except Exception as e:
                print(f"  [err] {html_file} → {e}")



# ══════════════════════════════════════════════════════════════════
#  PLAYWRIGHT CRAWLER
#  Render JavaScript penuh (DataTable, OJS, GT, dll)
#  Install: pip install playwright && playwright install chromium
# ══════════════════════════════════════════════════════════════════

class PlaywrightCrawler:
    """
    Crawl site dengan headless Chromium via Playwright.
    JavaScript dieksekusi penuh — DataTable, OJS, GT semua terbaca.

    Cara pakai:
        python ingest.py \
            --site-url https://your-site.com/sitemap.xml \
            --js \
            --category "panduan"
    """

    SKIP_EXT = (".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico",
                ".css", ".js", ".woff", ".woff2", ".ttf", ".eot",
                ".zip", ".pdf", ".docx", ".xlsx", ".mp4", ".mp3")

    def __init__(self, base_url: str, max_pages: int = 500,
                 wait_for: str = "networkidle", timeout: int = 20000):
        """
        wait_for  : "networkidle" (tunggu JS selesai) atau "domcontentloaded"
        timeout   : maks milli-detik tunggu per halaman
        """
        if base_url.endswith(".xml"):
            base_url = base_url.rsplit("/", 1)[0]
        self.base_url  = base_url.rstrip("/")
        self.base_host = urllib.parse.urlparse(base_url).netloc
        self.max_pages = max_pages
        self.wait_for  = wait_for
        self.timeout   = timeout

    def _valid_url(self, url: str) -> bool:
        p = urllib.parse.urlparse(url)
        if p.netloc and p.netloc != self.base_host:
            return False
        return not any(p.path.lower().endswith(e) for e in self.SKIP_EXT)

    def _normalize(self, url: str, base: str) -> str | None:
        url = urllib.parse.urljoin(base, url).split("#")[0].split("?")[0].rstrip("/")
        return url if url.startswith(self.base_url) else None

    def _urls_from_sitemap(self, session_req, sitemap_url: str) -> list[str]:
        from xml.etree import ElementTree as ET
        try:
            resp = session_req(sitemap_url)
            if not resp or resp.status != 200:
                return []
            ns  = {"sm": "http://www.sitemaps.org/schemas/sitemap/0.9"}
            root = ET.fromstring(resp.text())
            urls = []
            for el in root.findall("sm:sitemap", ns):
                loc = el.findtext("sm:loc", namespaces=ns)
                if loc:
                    urls.extend(self._urls_from_sitemap(session_req, loc))
            for el in root.findall("sm:url", ns):
                loc = el.findtext("sm:loc", namespaces=ns)
                if loc and self._valid_url(loc) and loc.startswith(self.base_url):
                    urls.append(loc.rstrip("/"))
            return urls
        except Exception as e:
            print(f"  [sitemap err] {sitemap_url} -> {e}")
            return []

    def crawl(self):
        from playwright.sync_api import sync_playwright

        with sync_playwright() as pw:
            browser = pw.chromium.launch(headless=True)
            context = browser.new_context(
                user_agent="Mozilla/5.0 pmpsti-rag-ingest/1.0",
                java_script_enabled=True,
            )
            page = context.new_page()

            # Helper fetch untuk sitemap (tanpa JS)
            import requests as _req
            _sess = _req.Session()
            def _sitemap_fetch(url):
                class _R:
                    def __init__(self, r):
                        self._r = r
                    @property
                    def status(self): return self._r.status_code
                    def text(self): return self._r.text
                try:
                    return _R(_sess.get(url, timeout=10))
                except Exception:
                    return None

            # 1. Ambil URL dari sitemap
            all_urls = []
            for sm_url in [f"{self.base_url}/sitemap.xml",
                           f"{self.base_url}/sitemap-index.xml"]:
                found = self._urls_from_sitemap(_sitemap_fetch, sm_url)
                if found:
                    all_urls = list(dict.fromkeys(found))
                    print(f"  [sitemap] {len(all_urls)} URL dari {sm_url}")
                    break

            if not all_urls:
                print("  [info] Sitemap tidak ada, BFS link-follow...")
                all_urls = [self.base_url]

            visited = set()
            queue   = list(all_urls)

            while queue and len(visited) < self.max_pages:
                url = queue.pop(0)
                if url in visited:
                    continue
                visited.add(url)

                try:
                    page.goto(url, wait_until=self.wait_for, timeout=self.timeout)

                    # Tunggu tabel selesai render
                    try:
                        page.wait_for_selector("table", timeout=5000)
                    except Exception:
                        pass

                    # ── Expand DataTable / GT / Reactable → tampilkan semua baris ──
                    js_check  = "() => typeof window.$ !== 'undefined' && typeof window.$.fn !== 'undefined' && typeof window.$.fn.dataTable !== 'undefined'"
                    js_expand = "() => { try { const api = window.$.fn.dataTable.tables({ api: true }); api.page.len(-1).draw(false); } catch(e) {} }"
                    js_unhide = 'document.querySelectorAll("tr").forEach(tr => { if(tr.style.display==="none") tr.style.display=""; }); document.querySelectorAll("[data-reactable-pager]").forEach(el => el.style.display="none");'

                    has_dt = page.evaluate(js_check)
                    if has_dt:
                        try:
                            page.evaluate(js_expand)
                            page.wait_for_timeout(800)
                            print("    [dt] DataTable: all rows expanded")
                        except Exception:
                            pass
                    else:
                        try:
                            page.evaluate(js_unhide)
                            page.wait_for_timeout(300)
                        except Exception:
                            pass

                    html = page.content()
                    soup = BeautifulSoup(html, "lxml")

                    # BFS: kumpulkan link baru (hanya kalau tidak dari sitemap)
                    if len(all_urls) <= 1:
                        for a in soup.find_all("a", href=True):
                            n = self._normalize(a["href"], url)
                            if n and n not in visited and self._valid_url(n):
                                queue.append(n)

                    print(f"  [js]  {url}")
                    yield url, soup

                    time.sleep(0.5)  # Sopan ke server

                except Exception as e:
                    print(f"  [err] {url} -> {e}")

            context.close()
            browser.close()

# ══════════════════════════════════════════════════════════════════
#  QMD SOURCE READER
#  Khusus untuk halaman yang datanya dirender via JavaScript
#  (DataTable, OJS, GT, dll) — baca file .qmd sumber langsung
# ══════════════════════════════════════════════════════════════════

def extract_qmd_markdown(qmd_text: str) -> str:
    """
    Bersihkan file .qmd → teks Markdown siap di-chunk.
    Hapus YAML front matter dan code fence yang bukan konten teks.
    """
    # Hapus YAML front matter (--- ... ---)
    qmd_text = re.sub("(?s)^---\\n.*?---\\n", "", qmd_text)

    lines   = qmd_text.splitlines(keepends=True)
    result  = []
    in_fence = False
    fence_lang = ""

    for line in lines:
        stripped = line.strip()

        # Deteksi code fence buka/tutup
        if stripped.startswith("```") or stripped.startswith(":::"):
            lang = stripped.lstrip("`").lstrip(":").strip().lower().split()[0] if stripped.lstrip("`").lstrip(":").strip() else ""
            if not in_fence:
                in_fence   = True
                fence_lang = lang
                # Block markdown/text tetap masuk
                if fence_lang in ("", "markdown", "md", "text"):
                    in_fence = False
                continue
            else:
                in_fence   = False
                fence_lang = ""
                continue

        if in_fence:
            # Hanya masukkan fence yang berisi data tabel (csv, dsv, dll)
            if fence_lang in ("csv", "tsv", "dsv"):
                result.append(line)
            continue

        result.append(line)

    return "".join(result).strip()


class QmdCrawler:
    """
    Baca file .qmd langsung dari direktori source Quarto.
    Berguna untuk halaman yang pakai DataTable / OJS / GT
    karena data tidak ada di HTML yang di-render.

    Cara pakai:
        python ingest.py --qmd-source ./panduan_pmpsti \
            --site-base-url https://solaemanachmad.github.io/panduan_pmpsti \
            --category "panduan"
    """
    def __init__(self, source_dir: str, base_url: str = "https://localhost"):
        self.source_dir = Path(source_dir)
        self.base_url   = base_url.rstrip("/")

    def crawl(self):
        skip_dirs = {"_site", "_book", "_freeze", ".quarto", "node_modules", ".git"}
        skip_files = {"_quarto.yml", "README.qmd", "index.qmd"}  # index bisa tetap diproses

        for qmd_file in sorted(self.source_dir.rglob("*.qmd")):
            # Skip folder internal Quarto
            if any(part in skip_dirs for part in qmd_file.parts):
                continue
            if qmd_file.name in skip_files:
                continue

            rel = qmd_file.relative_to(self.source_dir)
            # Buat URL konsisten dengan hasil render
            url = self.base_url + "/" + rel.as_posix().replace(".qmd", ".html")
            url = url.replace("/index.html", "")

            try:
                raw = qmd_file.read_text(encoding="utf-8")
                md  = extract_qmd_markdown(raw)
                print(f"  [qmd]  {url}  ({len(md)} chars)")
                yield url, md, qmd_file.stem
            except Exception as e:
                print(f"  [err]  {qmd_file} -> {e}")


# ══════════════════════════════════════════════════════════════════
#  HELPERS
# ══════════════════════════════════════════════════════════════════

def get_category(filepath: Path, base_dir: Path) -> str:
    parts = filepath.relative_to(base_dir).parts
    return parts[0] if len(parts) > 1 else "General"


def get_subcategory(filepath: Path, base_dir: Path) -> str:
    parts = filepath.relative_to(base_dir).parts
    return parts[1] if len(parts) > 2 else ""


def file_hash_md5(filepath: Path) -> str:
    h = hashlib.md5()
    with open(filepath, "rb") as f:
        while chunk := f.read(8192):
            h.update(chunk)
    return h.hexdigest()


def text_hash_md5(text: str) -> str:
    return hashlib.md5(text.encode()).hexdigest()


def clean_markdown(text: str) -> str:
    lines = text.splitlines()
    repeated = {l for l, c in Counter(l.strip() for l in lines if len(l.strip()) < 60).items() if c >= 3 and l}
    text = "\n".join(l for l in lines if l.strip() not in repeated)
    text = re.sub(r"\n{3,}", "\n\n", text)
    text = re.sub(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\u200b\u200c\u200d\ufeff]", "", text)
    text = re.sub(r"(\w)-\n(\w)", r"\1\2", text)
    return text.strip()



def table_to_prose(table_md: str) -> str:
    """
    Konversi tabel Markdown ke kalimat natural language.
    Tujuan: agar model embedding bisa memahami konteks tiap baris data.
    
    Input:
        | Kode | Mata Kuliah | SKS |
        | --- | --- | --- |
        | MKU101 | Kalkulus I | 3 |
    
    Output:
        Kode: MKU101, Mata Kuliah: Kalkulus I, SKS: 3.
        Kode: MKU102, Mata Kuliah: Fisika Dasar, SKS: 3.
    """
    lines = [l.strip() for l in table_md.strip().splitlines() if l.strip()]
    rows  = [l for l in lines if l.startswith("|") and "---" not in l]
    if len(rows) < 2:
        return table_md

    headers = [h.strip() for h in rows[0].strip("|").split("|") if h.strip()]
    sentences = []
    for row in rows[1:]:
        cells = [c.strip() for c in row.strip("|").split("|")]
        cells = [c for c in cells if c]
        if not cells or not any(c and c != "-" for c in cells):
            continue
        parts = [f"{h}: {c}" for h, c in zip(headers, cells) if c and c != "-"]
        if parts:
            sentences.append(", ".join(parts) + ".")

    return "\n".join(sentences) if sentences else table_md


def pre_process_tables(md_text: str) -> str:
    """
    Konversi semua tabel Markdown ke prosa natural language,
    sebelum masuk ke chunking pipeline.

    Tujuan:
    - Tabel tidak dipotong per baris oleh SemanticChunker
    - Konten tabel bisa di-embed dengan baik oleh model bahasa

    Cara kerja: scan baris per baris, kumpulkan blok tabel
    (baris yang diawali '|'), lalu konversi sekaligus.
    """
    lines  = md_text.splitlines(keepends=True)
    result = []
    i      = 0

    while i < len(lines):
        line = lines[i]
        # Awal blok tabel: baris dimulai dengan "|"
        if line.lstrip().startswith("|"):
            table_lines = []
            while i < len(lines) and lines[i].lstrip().startswith("|"):
                table_lines.append(lines[i].rstrip())
                i += 1
            table_md = "\n".join(table_lines)
            prose    = table_to_prose(table_md)
            result.append(prose + "\n")
        else:
            result.append(line)
            i += 1

    return "".join(result)


def site_id_from_url(base_url: str) -> str:
    """
    Buat document_id dari base URL site.
    https://user.github.io/panduan_pmpsti  →  panduan_pmpsti
    https://example.com                    →  example.com
    """
    parsed = urllib.parse.urlparse(base_url)
    # Ambil segmen path terakhir yang tidak kosong
    path_parts = [p for p in parsed.path.strip("/").split("/") if p]
    if path_parts:
        slug = path_parts[-1]
    return re.sub("[^a-zA-Z0-9_-]", "_", slug)
    # Fallback ke hostname
    return re.sub("[^a-zA-Z0-9_-]", "_", parsed.netloc)


def url_to_document_id(url: str, base_url: str) -> str:
    rel = url.replace(base_url, "").strip("/")
    # Buang ekstensi .html/.qmd sebelum sanitasi karakter
    rel = re.sub(r"\.(html|qmd)$", "", rel)
    # Hanya izinkan huruf, angka, slash, strip, underscore
    rel = re.sub(r"[^\w/\-]", "_", rel)
    rel = rel.strip("_/")
    return rel or "index"



# ══════════════════════════════════════════════════════════════════
#  LOGGING SETUP
# ══════════════════════════════════════════════════════════════════

def setup_logging(log_dir: str = "logs") -> logging.Logger:
    """Konfigurasi logging ke stdout dan ke file logs/ingest_YYYYMMDD.log."""
    from datetime import datetime
    os.makedirs(log_dir, exist_ok=True)
    log_file = os.path.join(log_dir, f"ingest_{datetime.now().strftime('%Y%m%d')}.log")

    logger = logging.getLogger("ingest")
    logger.setLevel(logging.DEBUG)

    fmt = logging.Formatter("%(asctime)s [%(levelname)s] %(message)s", datefmt="%H:%M:%S")

    # Console handler
    ch = logging.StreamHandler(sys.stdout)
    ch.setLevel(logging.INFO)
    ch.setFormatter(fmt)

    # File handler (rotating, 10 MB, 5 backup)
    fh = logging.handlers.RotatingFileHandler(log_file, maxBytes=10_000_000, backupCount=5, encoding="utf-8")
    fh.setLevel(logging.DEBUG)
    fh.setFormatter(fmt)

    if not logger.handlers:
        logger.addHandler(ch)
        logger.addHandler(fh)

    logger.info(f"Log file: {os.path.abspath(log_file)}")
    return logger


# ══════════════════════════════════════════════════════════════════
#  CHECKPOINT (resume crawl)
# ══════════════════════════════════════════════════════════════════

def checkpoint_path(base_url: str) -> str:
    """Buat nama file checkpoint dari base_url."""
    site_slug = re.sub(r"[^\w]", "_", base_url)[:60]
    return os.path.join("logs", f"checkpoint_{site_slug}.json")


def load_checkpoint(base_url: str) -> dict:
    """Muat checkpoint jika ada.  Format: {url: {hash, status}}."""
    path = checkpoint_path(base_url)
    if os.path.exists(path):
        try:
            with open(path) as f:
                return json.load(f)
        except Exception:
            pass
    return {}


def save_checkpoint(base_url: str, data: dict):
    """Simpan checkpoint ke file JSON."""
    os.makedirs("logs", exist_ok=True)
    path = checkpoint_path(base_url)
    with open(path, "w") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)


def clear_checkpoint(base_url: str):
    """Hapus checkpoint setelah ingest selesai."""
    path = checkpoint_path(base_url)
    if os.path.exists(path):
        os.remove(path)


# ══════════════════════════════════════════════════════════════════
#  AUTO-KATEGORI BERDASARKAN URL PATH
# ══════════════════════════════════════════════════════════════════
#
#  Aturan dicocokkan dari atas ke bawah — PERTAMA yang cocok menang.
#  Untuk site baru, tambahkan entri di SITE_CATEGORY_RULES dengan
#  key = domain (atau substring URL), value = list rule.
#
#  Format tiap rule: (regex_path, category, subcategory)
#  Regex dicocokkan ke path+slug URL (case-insensitive).
# ══════════════════════════════════════════════════════════════════

# Aturan generik yang berlaku untuk semua situs
_GENERIC_RULES: list[tuple[str, str, str]] = [
    # ── Pendaftaran & Admisi ──────────────────────────────────────
    (r"pendaftaran|admisi|registrasi|seleksi|daftar",
     "pendaftaran", "prosedur"),

    # ── Beasiswa ─────────────────────────────────────────────────
    (r"beasiswa.*(dalam|lokal|domestik)|dalam.negeri",
     "keuangan", "beasiswa_dalam_negeri"),
    (r"beasiswa.*(luar|internasional|asing)|luar.negeri",
     "keuangan", "beasiswa_luar_negeri"),
    (r"beasiswa",
     "keuangan", "beasiswa"),

    # ── Biaya / Keuangan ─────────────────────────────────────────
    (r"ukt|biaya.studi|biaya.kuliah|spp|keuangan",
     "keuangan", "biaya"),

    # ── Kurikulum & Program Studi ─────────────────────────────────
    (r"kurikulum|mata.kuliah|silabus|rps",
     "akademik", "kurikulum"),
    (r"program.studi|magister|doktor|s2|s3|prodi",
     "akademik", "program_studi"),

    # ── Tesis / Disertasi ─────────────────────────────────────────
    (r"tesis|disertasi|tugas.akhir|ujian.pra|pendadaran|wisuda",
     "akademik", "tesis"),

    # ── Kalender / Jadwal ─────────────────────────────────────────
    (r"kalender|jadwal|semester|akademik",
     "akademik", "jadwal"),

    # ── Penelitian & Publikasi ────────────────────────────────────
    (r"jurnal|publikasi|paper|konferensi|seminar|riset|penelitian",
     "penelitian", "publikasi"),

    # ── SDM — Dosen & Staff ───────────────────────────────────────
    (r"dosen|tenaga.pendidik|pengajar|pembimbing|promotor",
     "sdm", "dosen"),
    (r"staff|tenaga.kependidikan|pegawai",
     "sdm", "staff"),

    # ── Fasilitas ─────────────────────────────────────────────────
    (r"fasilitas|laboratorium|lab|perpustakaan|sarana",
     "informasi", "fasilitas"),

    # ── Profil / Tentang ─────────────────────────────────────────
    (r"profil|tentang|sejarah|visi|misi|akreditasi|about",
     "informasi", "profil"),

    # ── Pengumuman / Berita ───────────────────────────────────────
    (r"pengumuman|announcement|berita|news|agenda|event|kegiatan",
     "informasi", "berita"),

    # ── Layanan Akademik ─────────────────────────────────────────
    (r"layanan|pelayanan|dokumen|surat.keterangan|legalisir",
     "akademik", "layanan"),

    # ── Alumni ───────────────────────────────────────────────────
    (r"alumni|lulusan|tracer",
     "informasi", "alumni"),

    # ── Internasional ─────────────────────────────────────────────
    (r"international|foreign|global|oia|kerjasama",
     "informasi", "internasional"),
]

# Override khusus per domain — didahulukan sebelum aturan generik
# Key: substring yang ada di URL (domain atau path prefix unik)
SITE_CATEGORY_RULES: dict[str, list[tuple[str, str, str]]] = {
    "pasca.jteti.ugm.ac.id": [
        # Slug WordPress /YYYY/MM/DD/ → ekstrak dari slug judul
        # Aturan tambahan spesifik situs ini jika diperlukan
        (r"category/beasiswa_dalam_negeri", "keuangan", "beasiswa_dalam_negeri"),
        (r"category/beasiswa_luar_negeri",  "keuangan", "beasiswa_luar_negeri"),
        (r"category/pengumuman",            "informasi", "berita"),
        (r"privacy.policy",                 "informasi", "profil"),
        # WordPress /YYYY/MM/DD/slug → tangkap slug
        (r"\d{4}/\d{2}/\d{2}/.*beasiswa",  "keuangan",   "beasiswa"),
        (r"\d{4}/\d{2}/\d{2}/.*pendaftaran|prosedur", "pendaftaran", "prosedur"),
        (r"\d{4}/\d{2}/\d{2}/.*program.studi|magister|doktor", "akademik", "program_studi"),
        (r"\d{4}/\d{2}/\d{2}/.*tesis|ujian|wisuda",  "akademik", "tesis"),
    ],
    # ── Tambahkan situs baru di sini ──────────────────────────────
    # "example.ugm.ac.id": [
    #     (r"/khusus/path", "kategori", "subkategori"),
    # ],
}


def auto_category(url: str, default_cat: str = "informasi", default_subcat: str = "umum") -> tuple[str, str]:
    """
    Tentukan (category, subcategory) dari URL secara otomatis.

    Urutan pencocokan:
    1. Override spesifik per domain (SITE_CATEGORY_RULES)
    2. Aturan generik (_GENERIC_RULES)
    3. Fallback ke (default_cat, default_subcat)

    Parameter:
        url          : URL lengkap halaman
        default_cat  : fallback category jika tidak ada yang cocok
        default_subcat: fallback subcategory

    Contoh:
        auto_category("https://pasca.jteti.ugm.ac.id/beasiswa-dalam-negeri/")
        → ("keuangan", "beasiswa_dalam_negeri")

        auto_category("https://pasca.jteti.ugm.ac.id/2022/10/12/program-studi-magister/")
        → ("akademik", "program_studi")
    """
    url_lower = url.lower()

    # 1. Cek override per domain
    for domain, rules in SITE_CATEGORY_RULES.items():
        if domain in url_lower:
            for pattern, cat, subcat in rules:
                if re.search(pattern, url_lower):
                    return cat, subcat
            break  # domain cocok tapi tidak ada rule yang match → lanjut ke generik

    # 2. Aturan generik
    for pattern, cat, subcat in _GENERIC_RULES:
        if re.search(pattern, url_lower):
            return cat, subcat

    # 3. Fallback
    return default_cat, default_subcat



# ══════════════════════════════════════════════════════════════════
#  PROSES FILE (PDF & DOCX)
# ══════════════════════════════════════════════════════════════════

def process_files(conn, base_dir: Path, splitters: tuple, force: bool = False):
    extensions = {".pdf", ".docx"}
    files = [
        Path(root) / fname
        for root, _, filenames in os.walk(base_dir)
        for fname in filenames
        if os.path.splitext(fname)[1].lower() in extensions
    ]
    print(f"\nDitemukan {len(files)} file di {base_dir}")

    for i, filepath in enumerate(files):
        rel_path    = str(filepath.relative_to(base_dir))
        document_id = filepath.stem
        ext         = filepath.suffix.lower()
        print(f"\n[{i + 1}/{len(files)}] {rel_path}")

        fhash = file_hash_md5(filepath)
        existing = get_existing_hash(conn, document_id)

        if existing:
            if existing == fhash and not force:
                print("  [SKIP] Tidak ada perubahan.")
                continue
            print("  [UPDATE] File berubah, hapus data lama...")
            delete_document(conn, document_id)

        # Ekstrak
        if ext == ".pdf":
            md_text  = extract_pdf_markdown(filepath)
            doc_type = "pdf"
        else:
            md_text  = extract_docx_markdown(filepath)
            doc_type = "docx"

        md_text = clean_markdown(md_text)
        md_text = pre_process_tables(md_text)  # konversi tabel → prosa
        md_text = enrich_text(md_text)            # acronym expansion + repair
        if len(md_text) < 50:
            print("  [WARN] Konten terlalu pendek, skip.")
            continue

        category    = get_category(filepath, base_dir)
        subcategory = get_subcategory(filepath, base_dir)

        print("  Chunking...")
        final_chunks = markdown_to_chunks(md_text, splitters)
        print(f"  {len(final_chunks)} chunks")

        texts, records = chunks_to_records(
            final_chunks, document_id, doc_type, rel_path,
            category, subcategory, fhash
        )
        embed_and_insert(conn, texts, records)


# ══════════════════════════════════════════════════════════════════
#  PROSES SITE (QUARTO/HTML)
# ══════════════════════════════════════════════════════════════════

def process_site(
    conn, crawler, base_url: str,
    category: str, subcategory: str,
    doc_type: str, splitters: tuple,
    force: bool = False,
    logger: "logging.Logger | None" = None,
):
    """
    Ingest halaman satu per satu (incremental).
    - Setiap URL punya document_id sendiri.
    - Hash per halaman: skip jika tidak berubah.
    - Checkpoint: simpan progres, bisa lanjut jika putus.
    - Logging: ke stdout dan ke file logs/ingest_*.log.
    """
    if logger is None:
        logger = logging.getLogger("ingest")

    pages_processed = 0
    pages_skipped   = 0
    pages_error     = 0

    # Muat checkpoint sebelumnya (resume jika ada)
    checkpoint = load_checkpoint(base_url)
    if checkpoint and not force:
        done_urls = {u for u, v in checkpoint.items() if v.get("status") == "ok"}
        logger.info(f"[resume] Melanjutkan crawl — {len(done_urls)} halaman sudah diproses sebelumnya")
    else:
        done_urls = set()
        if force:
            checkpoint = {}

    logger.info(f"[site] Mulai crawl: {base_url}  (force={force})")

    for url, soup in crawler.crawl():
        # Lewati halaman yang sudah diproses di run sebelumnya
        if url in done_urls:
            logger.debug(f"  [checkpoint-skip] {url}")
            pages_skipped += 1
            continue

        try:
            meta    = extract_site_meta(soup, url)
            md_text = extract_site_markdown(soup)
            md_text = clean_markdown(md_text)
            md_text = pre_process_tables(md_text)
            md_text = enrich_text(md_text)

            if len(md_text.strip()) < 50:
                logger.info(f"  [skip] Konten terlalu pendek: {url}")
                pages_skipped += 1
                checkpoint[url] = {"status": "skip", "reason": "too_short"}
                save_checkpoint(base_url, checkpoint)
                continue

            # Sisipkan judul halaman sebagai H1
            if meta["title"]:
                md_text = "# " + meta["title"] + "\n\n" + md_text

            content_hash = text_hash_md5(md_text)

            # document_id = per URL (bukan per site)
            doc_id = url_to_document_id(url, base_url)

            # Auto-kategori jika category/subcategory tidak di-set eksplisit
            eff_category    = category
            eff_subcategory = subcategory
            if not eff_category:
                eff_category, eff_subcategory = auto_category(url)
                logger.debug(f"    auto-category: {eff_category}/{eff_subcategory}")
            elif not eff_subcategory:
                _, eff_subcategory = auto_category(url, default_subcat="umum")
                logger.debug(f"    auto-subcategory: {eff_subcategory}")

            # Cek hash di database — skip jika sama dan tidak --force
            existing_hash = get_existing_hash(conn, doc_id)
            if existing_hash == content_hash and not force:
                logger.info(f"  [unchanged] {doc_id}")
                pages_skipped += 1
                checkpoint[url] = {"status": "ok", "hash": content_hash}
                save_checkpoint(base_url, checkpoint)
                continue

            # Hapus data lama untuk doc_id ini sebelum re-insert
            if existing_hash is not None:
                delete_document(conn, doc_id)
                logger.info(f"  [update] {doc_id}")
            else:
                logger.info(f"  [new] {doc_id}")

            page_path = url.replace(base_url, "").strip("/") or "index"
            logger.debug(f"    Chunking: {page_path}")
            final_chunks = markdown_to_chunks(md_text, splitters)
            logger.info(f"    {len(final_chunks)} chunks  |  source: {url}")

            texts, records = chunks_to_records(
                final_chunks, doc_id, doc_type, url,
                eff_category, eff_subcategory, content_hash
            )

            embed_and_insert(conn, texts, records)
            pages_processed += 1

            # Simpan checkpoint setelah berhasil
            checkpoint[url] = {"status": "ok", "hash": content_hash}
            save_checkpoint(base_url, checkpoint)

        except KeyboardInterrupt:
            logger.warning(f"  [interrupt] Proses dihentikan manual. Checkpoint disimpan.")
            save_checkpoint(base_url, checkpoint)
            raise

        except Exception as exc:
            pages_error += 1
            logger.error(f"  [error] {url}: {exc}")
            checkpoint[url] = {"status": "error", "error": str(exc)}
            save_checkpoint(base_url, checkpoint)
            # Lanjut ke halaman berikutnya

    if pages_error > 0:
        logger.warning(f"[site] Selesai: {pages_processed} diproses, {pages_skipped} dilewati, {pages_error} error")
        logger.warning(f"  Jalankan lagi tanpa --force untuk retry halaman yang error")
    else:
        logger.info(f"[site] Selesai: {pages_processed} diproses, {pages_skipped} dilewati")
        clear_checkpoint(base_url)  # hapus checkpoint jika semua berhasil

    return pages_processed, pages_skipped


# ══════════════════════════════════════════════════════════════════


def process_qmd(
    conn, crawler: "QmdCrawler",
    category: str, subcategory: str,
    doc_type: str, splitters: tuple,
    force: bool = False,
):
    """Ingest dari file .qmd source langsung — setiap file = document_id sendiri."""
    processed = 0
    skipped   = 0

    for url, md_text, stem in crawler.crawl():
        # document_id unik per file: path relatif dari base_url
        doc_id = url_to_document_id(url, crawler.base_url)
        if not doc_id or doc_id == "index":
            doc_id = stem  # fallback ke nama file

        md_text = clean_markdown(md_text)
        md_text = pre_process_tables(md_text)
        md_text = enrich_text(md_text)

        if len(md_text.strip()) < 50:
            print(f"  [skip] Konten terlalu pendek: {url}")
            skipped += 1
            continue

        if not re.match("^# ", md_text):
            title_str = stem.replace("_", " ").replace("-", " ").title()
            md_text = "# " + title_str + "\n\n" + md_text

        content_hash = text_hash_md5(md_text)

        # Cek hash — skip jika tidak berubah dan tidak --force
        existing_hash = get_existing_hash(conn, doc_id)
        if existing_hash == content_hash and not force:
            print(f"  [unchanged] {doc_id}")
            skipped += 1
            continue
        if existing_hash is not None:
            print(f"  [update] Hapus data lama '{doc_id}'...")
            delete_document(conn, doc_id)
        else:
            print(f"  [new] {doc_id}")

        print(f"  Chunking: {stem}  ->  {doc_id}")
        final_chunks = markdown_to_chunks(md_text, splitters)
        print(f"    {len(final_chunks)} chunks  |  {url}")

        texts, records = chunks_to_records(
            final_chunks, doc_id, doc_type, url,
            category, subcategory, content_hash
        )
        embed_and_insert(conn, texts, records)
        processed += 1

    return processed, skipped



def main():
    parser = argparse.ArgumentParser(description="Ingest dokumen ke pmpsti_rag")

    # Sumber
    parser.add_argument("--files",      action="store_true",
                        help=f"Ingest PDF & DOCX dari {BASE_DIR}")
    parser.add_argument("--files-dir",  default=str(BASE_DIR),
                        help="Override folder dokumen (default: ../docs)")
    parser.add_argument("--site-url",   help="Base URL Quarto site")
    parser.add_argument("--site-local", help="Path ke folder _site/ Quarto lokal")
    parser.add_argument("--site-base-url", default="https://localhost",
                        help="Base URL palsu untuk --site-local (untuk source_url di DB)")
    parser.add_argument("--qmd-source",  help="Path ke folder source .qmd Quarto (untuk halaman DataTable/OJS)")
    parser.add_argument("--qmd-base-url", default="https://localhost",
                        help="Base URL untuk --qmd-source")

    # Metadata site
    parser.add_argument("--category",    default="",
                        help="Kategori dokumen (opsional — jika kosong, ditentukan otomatis dari URL). "
                             "Contoh: akademik, informasi, keuangan, pendaftaran, penelitian, sdm")
    parser.add_argument("--subcategory", default="",
                        help="Sub-kategori (opsional — jika category di-set tapi subcategory kosong, "
                             "subcategory juga ditentukan otomatis)")
    parser.add_argument("--document-type", default="web", help="Tipe dokumen site (default: web)")
    parser.add_argument("--max-pages",     type=int, default=300)
    parser.add_argument("--js",            action="store_true",
                        help="Gunakan Playwright (headless browser) untuk render JavaScript. "
                             "Wajib untuk halaman DataTable/OJS/GT. "
                             "Install: pip install playwright && playwright install chromium")

    # Umum
    parser.add_argument("--db",    default=DB_URL)
    parser.add_argument("--force", action="store_true",
                        help="Re-ingest meski hash sama")

    args = parser.parse_args()

    if not args.files and not args.site_url and not args.site_local and not args.qmd_source:
        parser.print_help()
        sys.exit(1)

    # Setup logging (stdout + file)
    logger = setup_logging()

    logger.info("=" * 55)
    logger.info("  PMPSTI RAG — Unified Ingest Pipeline")
    logger.info("  Auto-kategori: aktif (URL-based, dapat di-override)")
    logger.info("=" * 55)

    conn = connect_db(args.db)
    setup_database(conn)
    splitters = build_splitters()

    # ── Ingest file ──
    if args.files:
        print("\n▶ Mode: File (PDF & DOCX)")
        base_dir = Path(args.files_dir)
        if not base_dir.exists():
            print(f"[ERROR] Folder tidak ditemukan: {base_dir}")
            sys.exit(1)
        process_files(conn, base_dir, splitters, force=args.force)

    # ── Ingest site dari URL ──
    if args.site_url:
        site_url = args.site_url
        if site_url.endswith(".xml"):
            site_url = site_url.rsplit("/", 1)[0]
        site_url = site_url.rstrip("/")

        if args.js:
            print(f"\n▶ Mode: Site URL + JavaScript/Playwright  ({site_url})")
            crawler = PlaywrightCrawler(args.site_url, max_pages=args.max_pages)
        else:
            print(f"\n▶ Mode: Site URL  ({site_url})")
            crawler = SiteCrawler(args.site_url, max_pages=args.max_pages)

        n, s = process_site(
            conn, crawler, site_url,
            args.category, args.subcategory, args.document_type,
            splitters, force=args.force, logger=logger
        )
        logger.info(f"  Site selesai — {n} halaman diproses, {s} di-skip.")

    # ── Ingest site lokal ──
    if args.site_local:
        print(f"\n▶ Mode: Site Lokal  ({args.site_local})")
        crawler = LocalSiteCrawler(args.site_local, base_url=args.site_base_url)
        n, s = process_site(
            conn, crawler, args.site_base_url,
            args.category, args.subcategory, args.document_type,
            splitters, force=args.force, logger=logger
        )
        logger.info(f"  Site selesai — {n} halaman diproses, {s} di-skip.")

    # ── Ingest dari .qmd source langsung ──
    if args.qmd_source:
        qmd_base = args.qmd_base_url
        if qmd_base == "https://localhost" and args.site_url:
            # Gunakan site_url sebagai base kalau tidak di-set eksplisit
            qmd_base = args.site_url.rsplit("/", 1)[0] if args.site_url.endswith(".xml") else args.site_url
        qmd_base = qmd_base.rstrip("/")
        print(f"\n▶ Mode: QMD Source  ({args.qmd_source})")
        print(f"  Base URL: {qmd_base}")
        crawler = QmdCrawler(args.qmd_source, base_url=qmd_base)
        n, s = process_qmd(
            conn, crawler,
            args.category, args.subcategory, args.document_type,
            splitters, force=args.force
        )
        print(f"  QMD selesai — {n} file diproses, {s} di-skip.")

    print_db_stats(conn)
    conn.close()
    logger.info("All done!")


if __name__ == "__main__":
    main()