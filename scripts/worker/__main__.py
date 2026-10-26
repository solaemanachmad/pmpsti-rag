"""
PDF ingest worker — Google Drive folders/files and website PDFs -> DRAFT chunks.
Design: docs/design/ingest-from-links.md.  Run from scripts/:

    python -m worker                      # process all queued jobs (created in Admin -> Sources)
    python -m worker --recheck            # also queue sources that are due (their own interval; scheduled run)
    python -m worker --source-id 3        # queue + process one source now
    python -m worker --file guide.pdf     # local test: read a PDF and print chunks, no database

Environment (root .env or real env): DATABASE_URL, GEMINI_API_KEY, GOOGLE_API_KEY (Drive),
optional PROD_DATABASE_URL (to recognise production), GEMINI_MODEL.
Writing to the production database needs --allow-production (drafts are not searchable
until an admin approves them, but the flag keeps local runs from touching production by accident).
"""
from __future__ import annotations

import argparse
import os
import re
import sys
import time
import traceback
import unicodedata

import requests

from gemini_embed import GeminiEmbeddings, document_text  # also loads the root .env
from text_enrichment import enrich_text

from . import fetch, store
from .chunker import Chunk, chunk_pages, clean_page_markdown
from .pdf_reader import missing_numbers, read_pdf, render_page_png

MAX_FIGURES = int(os.environ.get("WORKER_MAX_FIGURES", "15"))


def slugify(text: str) -> str:
    text = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    text = re.sub(r"\.pdf$", "", text, flags=re.I)
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:60] or "dokumen"


def vec_literal(v: list[float]) -> str:
    return "[" + ",".join(f"{x:.8f}" for x in v) + "]"


# ── Turning one PDF into chunks ───────────────────────────────────────────────

def build_chunks(data: bytes, doc_title: str, describer=None, max_figures: int = MAX_FIGURES):
    """Return (chunks, warnings, stats). Pure apart from the optional Gemini describer."""
    pages, meta = read_pdf(data)
    warnings: list[str] = []

    texts = [(p.number, enrich_text(clean_page_markdown(p.markdown, p.number))) for p in pages]
    chunks = chunk_pages(texts)

    empty = [p.number for p in pages if len(p.markdown.strip()) < 30 and not p.is_figure_candidate]
    if empty:
        warnings.append(f"Halaman hampir tanpa teks (mungkin hasil scan / kosong): {empty}")

    # Number check: every number on a page should appear in that page's chunks
    by_page: dict[int, list[str]] = {}
    for c in chunks:
        by_page.setdefault(c.page, []).append(c.content)
    for p in pages:
        miss = missing_numbers(p.raw_text, by_page.get(p.number, [])) - {str(p.number)}
        if miss:
            warnings.append(f"hal. {p.number}: angka di PDF tidak ada di chunk: {', '.join(sorted(miss)[:8])}")

    # Figures → own chunks, placed after the text of their page
    figure_pages = [p.number for p in pages if p.is_figure_candidate]
    described = 0
    if describer and figure_pages:
        if len(figure_pages) > max_figures:
            warnings.append(f"{len(figure_pages)} halaman bergambar, hanya {max_figures} pertama dideskripsikan")
        for page_no in figure_pages[:max_figures]:
            try:
                desc = describer.describe(render_page_png(data, page_no))
            except RuntimeError as e:
                warnings.append(f"hal. {page_no}: deskripsi gambar gagal ({e})")
                if "Kuota harian" in str(e):
                    break
                continue
            if not desc:
                continue
            path = next((c.heading_path for c in reversed(chunks) if c.page <= page_no), [])
            pos = max((i + 1 for i, c in enumerate(chunks) if c.page <= page_no), default=0)
            chunks.insert(pos, Chunk(page_no, "figure", f"[Deskripsi gambar, hal. {page_no}]\n{desc}", list(path)))
            described += 1
    elif figure_pages:
        warnings.append(f"Gambar tidak dideskripsikan (dimatikan) pada hal. {figure_pages}")

    stats = {
        "pages": meta["page_count"],
        "chunks": len(chunks),
        "table_chunks": sum(c.kind == "table" for c in chunks),
        "figure_chunks": described,
        "figure_pages": figure_pages,
        "doc_title": doc_title or meta["title"],
    }
    return chunks, warnings, stats


# ── Processing a job ──────────────────────────────────────────────────────────

class Worker:
    def __init__(self, conn, figures: bool):
        self.conn = conn
        self.http = requests.Session()
        self.http.headers["User-Agent"] = fetch.USER_AGENT
        self.api_key = os.environ.get("GOOGLE_API_KEY", "")
        self.embedder = GeminiEmbeddings(task_type="RETRIEVAL_DOCUMENT")
        self.describer = None
        if figures:
            from .figures import FigureDescriber
            self.describer = FigureDescriber()

    def list_files(self, src: store.Source) -> list[fetch.RemoteFile]:
        if src.kind in ("drive_folder", "drive_file") and not self.api_key:
            raise RuntimeError("GOOGLE_API_KEY belum di-set (dibutuhkan untuk Google Drive)")
        if src.kind == "drive_folder":
            return fetch.list_drive_folder(self.http, src.external_id, self.api_key)
        if src.kind == "drive_file":
            return [fetch.get_drive_file(self.http, src.external_id, self.api_key)]
        if src.kind == "web_pdf":
            return [fetch.web_file(src.url)]
        raise RuntimeError(f"Jenis sumber tidak didukung oleh worker: {src.kind}")

    def run_job(self, job_id: int, src: store.Source) -> None:
        log: list[str] = []
        stats: dict = {"files": []}
        state = "done"
        last_file_id = None
        try:
            files = self.list_files(src)
            log.append(f"{len(files)} file ditemukan di {src.url}")
            if not files:
                log.append("Tidak ada PDF. Untuk folder Drive: pastikan berisi PDF dan dibagikan 'Anyone with the link'.")
            failed = 0
            for f in files:
                try:
                    file_stats, file_log, last_file_id = self.process_file(src, f)
                except Exception as e:  # noqa: BLE001 — one bad file must not stop the others
                    self.conn.rollback()
                    failed += 1
                    traceback.print_exc()
                    file_stats, file_log = {"name": f.name, "result": "gagal"}, [f"GAGAL: {e}"]
                    if "Kuota harian" in str(e):
                        stats["files"].append(file_stats)
                        log.extend(f"[{f.name}] {line}" for line in file_log)
                        raise RuntimeError("kuota harian Gemini habis; sisa file diproses pada jalan berikutnya") from e
                stats["files"].append(file_stats)
                log.extend(f"[{f.name}] {line}" for line in file_log)
            if failed and failed == len(files):
                state = "failed"
        except Exception as e:  # noqa: BLE001 — every failure must end up in the job log
            self.conn.rollback()
            state = "failed"
            log.append(f"GAGAL: {e}")
            traceback.print_exc()
        store.finish_job(self.conn, job_id, state, "\n".join(log), stats,
                         last_file_id if len(stats["files"]) == 1 else None)
        print(f"job {job_id} ({src.url}): {state}")
        for line in log:
            print("   ", line)

    def process_file(self, src: store.Source, f: fetch.RemoteFile):
        row = store.upsert_file(self.conn, src.id, f.external_id, f.name, f.citation_url)
        info = {"name": f.name, "result": ""}
        if row.status in ("active", "draft") and not store.document_exists(self.conn, row.document_id):
            row.status = "missing"      # chunks were deleted by hand → process the file again
        # Statuses for which an unchanged file needs no work. "Check now" by an admin also
        # re-processes a file whose draft was rejected.
        settled = ("active", "draft") if src.force else ("active", "draft", "rejected")
        unconditional = row.status not in settled

        if src.kind == "web_pdf":
            dl = fetch.download_web_pdf(self.http, f.external_id, "" if unconditional else row.remote_rev)
        else:
            if f.remote_rev and f.remote_rev == row.remote_rev and not unconditional:
                info["result"] = "tidak berubah"
                return info, ["tidak berubah (Drive checksum sama)"], row.id
            dl = fetch.download_drive_file(self.http, f, self.api_key)

        if dl.data is None or (dl.sha256 == row.sha256 and not unconditional):
            store.update_file(self.conn, row.id, remote_rev=dl.remote_rev)
            info["result"] = "tidak berubah"
            note = "tidak berubah" + (" (pernah ditolak admin)" if row.status == "rejected" else "")
            return info, [note], row.id

        dup = store.find_duplicate(self.conn, dl.sha256, row.id)
        if dup:
            msg = f"isi file sama dengan sumber '{dup['title']}' (dokumen {dup['document_id']}); tidak disimpan dua kali"
            if src.kind == "web_pdf" and dup["kind"] != "web_pdf":
                store.prefer_official_citation(self.conn, dup, f.citation_url)
                msg += "; sitasi diganti ke link website resmi, Drive menjadi mirror"
            store.update_file(self.conn, row.id, sha256=dl.sha256, remote_rev=dl.remote_rev,
                              status="duplicate", document_id=dup["document_id"])
            info["result"] = "duplikat"
            return info, [msg], row.id

        file_title = re.sub(r"\.pdf$", "", f.name, flags=re.I)
        if src.kind == "drive_folder":   # one source, many files: keep each file's own name
            doc_title = f"{src.title} — {file_title}" if src.title else file_title
        else:
            doc_title = src.title or file_title
        if src.version_label:
            doc_title = f"{doc_title} ({src.version_label})"

        document_id = f"{slugify(doc_title)}-{dl.sha256[:8]}"
        if store.document_status(self.conn, document_id) == "active":
            # e.g. a draft was rejected and the file went back to the approved content
            store.update_file(self.conn, row.id, sha256=dl.sha256, remote_rev=dl.remote_rev,
                              status="active", document_id=document_id)
            info["result"] = "tidak berubah"
            return info, ["isi sama dengan versi aktif; tidak ada draft baru"], row.id

        chunks, warnings, cstats = build_chunks(dl.data, doc_title, self.describer)
        if not chunks:
            store.update_file(self.conn, row.id, status="error", sha256=dl.sha256, remote_rev=dl.remote_rev)
            raise RuntimeError(f"{f.name}: tidak ada teks yang bisa dibaca")

        titles = [c.title(doc_title) for c in chunks]
        vectors = self.embedder.embed([document_text(t, c.content) for t, c in zip(titles, chunks)],
                                      "RETRIEVAL_DOCUMENT", titles=titles)
        # Stored citation: may have been switched to the official website link (duplicate on Drive)
        citation = row.citation_url or f.citation_url
        rows = [{
            "title": t, "content": c.content, "source_url": citation,
            "category": src.category, "subcategory": src.subcategory, "file_hash": dl.sha256,
            "embedding": vec_literal(v), "page": c.page, "chunk_index": i, "kind": c.kind,
        } for i, (t, c, v) in enumerate(zip(titles, chunks, vectors))]
        n = store.write_drafts(self.conn, row.id, document_id, rows)
        store.update_file(self.conn, row.id, sha256=dl.sha256, remote_rev=dl.remote_rev,
                          status="draft", document_id=document_id)
        info.update(result="draft", document_id=document_id, **cstats)
        lines = [f"{n} chunk draft dari {cstats['pages']} halaman "
                 f"({cstats['table_chunks']} tabel, {cstats['figure_chunks']} gambar) → menunggu review admin"]
        return info, lines + warnings, row.id


# ── CLI ───────────────────────────────────────────────────────────────────────

def local_preview(path: str, figures: bool) -> int:
    data = open(path, "rb").read()
    describer = None
    if figures:
        from .figures import FigureDescriber
        describer = FigureDescriber()
    chunks, warnings, stats = build_chunks(data, os.path.basename(path), describer)
    for i, c in enumerate(chunks):
        print(f"\n── #{i} hal.{c.page} [{c.kind}] {c.title(stats['doc_title'])}\n{c.content[:400]}")
    print("\nSTATS", stats)
    for w in warnings:
        print("WARN", w)
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--recheck", action="store_true", help="queue sources whose check interval (daily/weekly/N days) has passed; never sooner than --recheck-hours")
    ap.add_argument("--recheck-hours", type=float, default=20)
    ap.add_argument("--source-id", type=int, help="queue this source now, then process the queue")
    ap.add_argument("--max-jobs", type=int, default=20)
    ap.add_argument("--no-figures", action="store_true", help="skip Gemini figure descriptions")
    ap.add_argument("--allow-production", action="store_true")
    ap.add_argument("--file", help="local PDF: print chunks only, no database")
    args = ap.parse_args(argv)

    if args.file:
        return local_preview(args.file, figures=not args.no_figures)

    url = os.environ.get("DATABASE_URL", "")
    missing = [k for k in ("DATABASE_URL", "GEMINI_API_KEY") if not os.environ.get(k)]
    if missing:
        print(f"Secret belum di-set: {', '.join(missing)}. Di GitHub: Settings → Environments → production "
              "(atau Settings → Secrets and variables → Actions → Repository secrets).", file=sys.stderr)
        return 2
    if not os.environ.get("GOOGLE_API_KEY"):
        print("Catatan: GOOGLE_API_KEY kosong — sumber Google Drive akan gagal; PDF dari website tetap diproses.")
    print(f"Database: {store.db_host(url)}")
    prod = os.environ.get("PROD_DATABASE_URL", "")
    if not args.allow_production:
        if not prod:
            print("Set PROD_DATABASE_URL (connection string production) di .env agar worker bisa mengenali "
                  "database production, atau jalankan dengan --allow-production bila memang ke production.",
                  file=sys.stderr)
            return 2
        if store.is_production(url, prod):
            print("DATABASE_URL menunjuk ke PRODUCTION. Tambahkan --allow-production jika memang disengaja.",
                  file=sys.stderr)
            return 2

    conn = store.connect(url)
    store.require_schema(conn)
    store.reset_stale_jobs(conn)
    if args.recheck:
        print(f"{store.queue_rechecks(conn, args.recheck_hours)} sumber dijadwalkan cek ulang")
    if args.source_id:
        store.queue_source(conn, args.source_id, "Dijalankan dari worker CLI")

    worker = Worker(conn, figures=not args.no_figures)
    done = 0
    while done < args.max_jobs:
        job = store.claim_job(conn)
        if not job:
            break
        worker.run_job(*job)
        done += 1
        time.sleep(1)
    print(f"{done} job diproses")
    conn.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
