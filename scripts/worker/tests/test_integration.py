"""End-to-end worker run against a real PostgreSQL + pgvector with the backend's schema.

Skipped unless TEST_DATABASE_URL points to a THROWAWAY database on which the Rust backend
has run once (it creates the tables). Network and Gemini are replaced by fakes.
"""
import os

import pymupdf
import pytest

from worker import __main__ as cli
from worker import fetch, store

DB = os.environ.get("TEST_DATABASE_URL")
pytestmark = pytest.mark.skipif(not DB, reason="TEST_DATABASE_URL not set")
URL = "https://example.ac.id/files/panduan-uji.pdf"


def make_pdf(version: str) -> bytes:
    doc = pymupdf.open()
    for n in range(1, 4):
        page = doc.new_page()
        page.insert_text((72, 72), f"BAB {n}. Ketentuan Akademik", fontsize=16)
        page.insert_text((72, 110), f"Mahasiswa wajib menempuh 36 SKS dengan IPK minimal 3,00. Versi {version}.", fontsize=11)
        page.insert_text((72, 130), f"Masa studi paling lama {n + 5} semester sesuai peraturan yang berlaku.", fontsize=11)
    data = doc.tobytes()
    doc.close()
    return data


class FakeHTTP:
    def __init__(self, pdf):
        self.pdf, self.headers = pdf, {}

    def get(self, url, headers=None, timeout=None, allow_redirects=True, params=None):
        class R:
            status_code = 200
            content = self.pdf
            headers = {"Content-Type": "application/pdf"}
        return R()


class FakeEmbedder:
    calls = 0

    def embed(self, texts, task_type=None, titles=None):
        FakeEmbedder.calls += 1
        return [[0.001 * (i % 7 + 1)] * 768 for i in range(len(texts))]


@pytest.fixture()
def conn():
    c = store.connect(DB)
    store.require_schema(c)
    with c.cursor() as cur:
        cur.execute("DELETE FROM documents WHERE source_file_id IN (SELECT f.id FROM source_files f "
                    "JOIN sources s ON s.id = f.source_id WHERE s.url = %s)", (URL,))
        cur.execute("DELETE FROM sources WHERE url = %s", (URL,))
        cur.execute("DELETE FROM ingest_jobs WHERE state IN ('queued','running')")  # throwaway DB only
        cur.execute("INSERT INTO sources (kind, url, external_id, title, version_label, category) "
                    "VALUES ('web_pdf', %s, %s, 'Panduan Uji', '2026', 'akademik') RETURNING id", (URL, URL))
        sid = cur.fetchone()[0]
    c.commit()
    yield c, sid
    c.close()


def make_worker(c, pdf):
    w = cli.Worker.__new__(cli.Worker)
    w.conn, w.http, w.api_key, w.describer = c, FakeHTTP(pdf), "", None
    w.embedder = FakeEmbedder()
    return w


def run_once(c, sid, w):
    store.queue_source(c, sid, "test")
    job = store.claim_job(c)
    assert job and job[1].id == sid
    w.run_job(*job)
    with c.cursor() as cur:
        cur.execute("SELECT state, log FROM ingest_jobs WHERE id = %s", (job[0],))
        return cur.fetchone()


def test_draft_then_unchanged_then_new_version(conn):
    c, sid = conn
    w = make_worker(c, make_pdf("A"))
    state, log = run_once(c, sid, w)
    assert state == "done", log
    with c.cursor() as cur:
        cur.execute("SELECT DISTINCT d.status, d.document_id, d.source_url FROM documents d "
                    "JOIN source_files f ON f.id = d.source_file_id WHERE f.source_id = %s", (sid,))
        rows = cur.fetchall()
        assert len(rows) == 1 and rows[0][0] == "draft" and rows[0][2] == URL
        doc_a = rows[0][1]
        assert doc_a.startswith("panduan-uji-2026-")
        cur.execute("SELECT MIN(page_number), MAX(page_number), COUNT(*) FROM documents WHERE document_id = %s", (doc_a,))
        lo, hi, n = cur.fetchone()
        assert (lo, hi) == (1, 3) and n >= 3
        cur.execute("SELECT status FROM source_files WHERE source_id = %s", (sid,))
        assert cur.fetchone()[0] == "draft"

    # same bytes again → unchanged, no new embeddings
    calls = FakeEmbedder.calls
    state, log = run_once(c, sid, w)
    assert state == "done" and "tidak berubah" in log and FakeEmbedder.calls == calls

    # simulate admin approval, then a changed file → new draft beside the active version
    with c.cursor() as cur:
        cur.execute("UPDATE documents SET status = 'active' WHERE document_id = %s", (doc_a,))
        cur.execute("UPDATE source_files SET status = 'active' WHERE source_id = %s", (sid,))
    c.commit()
    w.http = FakeHTTP(make_pdf("B"))
    state, log = run_once(c, sid, w)
    assert state == "done", log
    with c.cursor() as cur:
        cur.execute("SELECT d.status, COUNT(DISTINCT d.document_id) FROM documents d JOIN source_files f "
                    "ON f.id = d.source_file_id WHERE f.source_id = %s GROUP BY d.status ORDER BY d.status", (sid,))
        assert cur.fetchall() == [("active", 1), ("draft", 1)]


def _status_counts(c, sid):
    with c.cursor() as cur:
        cur.execute("SELECT d.status, COUNT(DISTINCT d.document_id) FROM documents d JOIN source_files f "
                    "ON f.id = d.source_file_id WHERE f.source_id = %s GROUP BY d.status ORDER BY d.status", (sid,))
        return dict(cur.fetchall())


def _approve(c, sid):
    """What the backend's approve does for a single-file source (sources::approve_draft)."""
    with c.cursor() as cur:
        cur.execute("""UPDATE documents SET status = 'archived' WHERE status = 'active' AND source_file_id IN
                       (SELECT id FROM source_files WHERE source_id = %s)""", (sid,))
        cur.execute("""UPDATE documents SET status = 'active' WHERE status = 'draft' AND source_file_id IN
                       (SELECT id FROM source_files WHERE source_id = %s)""", (sid,))
        cur.execute("UPDATE source_files SET status = 'active' WHERE source_id = %s", (sid,))
    c.commit()


def test_back_to_an_archived_version_and_rejected_recheck(conn):
    c, sid = conn
    w = make_worker(c, make_pdf("A"))
    assert run_once(c, sid, w)[0] == "done"; _approve(c, sid)          # A active
    w.http = FakeHTTP(make_pdf("B"))
    assert run_once(c, sid, w)[0] == "done"; _approve(c, sid)          # B active, A archived
    w.http = FakeHTTP(make_pdf("A"))
    state, log = run_once(c, sid, w)                                    # back to A: same id as archived A
    assert state == "done", log
    assert _status_counts(c, sid) == {"active": 1, "archived": 1, "draft": 1}   # archived A kept under a new id

    # reject the draft → a normal re-check skips it, "Check now" by an admin processes it again
    with c.cursor() as cur:
        cur.execute("DELETE FROM documents WHERE status = 'draft' AND source_file_id IN "
                    "(SELECT id FROM source_files WHERE source_id = %s)", (sid,))
        cur.execute("UPDATE source_files SET status = 'rejected' WHERE source_id = %s", (sid,))
    c.commit()
    state, log = run_once(c, sid, w)
    assert "ditolak" in log and "draft" not in _status_counts(c, sid)
    with c.cursor() as cur:
        cur.execute("INSERT INTO ingest_jobs (source_id, log) VALUES (%s, %s)", (sid, store.ADMIN_RECHECK_NOTE))
    c.commit()
    job = store.claim_job(c)
    assert job[1].force
    w.run_job(*job)
    assert _status_counts(c, sid).get("draft") == 1


def test_one_failing_file_does_not_break_the_job(conn):
    c, sid = conn

    class Broken(FakeEmbedder):
        def embed(self, *a, **k):
            raise RuntimeError("embedding service down")

    w = make_worker(c, make_pdf("A"))
    w.embedder = Broken()
    state, log = run_once(c, sid, w)
    assert state == "failed" and "embedding service down" in log
    # the connection is still usable and the next run works
    w.embedder = FakeEmbedder()
    state, log = run_once(c, sid, w)
    assert state == "done", log
