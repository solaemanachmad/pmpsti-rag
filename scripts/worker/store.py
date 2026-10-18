"""Database access for the worker (tables created by the Rust backend, src/sources.rs).

The worker only ever writes chunks with status = 'draft'. Activation and archiving are
done by the admin through the backend (approve / reject).
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from urllib.parse import urlparse

import psycopg2
import psycopg2.extras


def db_host(url: str) -> str:
    try:
        return urlparse(url).hostname or ""
    except ValueError:
        return ""


def endpoint_id(url: str) -> str:
    """Neon endpoint of a connection string: 'ep-calm-dawn-b4n290kt' for both the pooled
    (ep-…-pooler.region…) and the direct host, so both spellings count as the same database."""
    host = db_host(url).split(".")[0]
    return host[:-len("-pooler")] if host.endswith("-pooler") else host


def is_production(url: str, production_url: str) -> bool:
    return bool(production_url) and endpoint_id(url) == endpoint_id(production_url)


def connect(url: str):
    conn = psycopg2.connect(url)
    conn.autocommit = False
    return conn


@dataclass
class Source:
    id: int
    kind: str
    url: str
    external_id: str
    title: str
    category: str
    subcategory: str
    version_label: str
    force: bool = False       # admin pressed "Check now": also re-process rejected files


@dataclass
class FileRow:
    id: int
    sha256: str
    remote_rev: str
    status: str
    document_id: str
    citation_url: str = ""


def require_schema(conn) -> None:
    with conn.cursor() as cur:
        cur.execute("SELECT to_regclass('public.ingest_jobs') IS NOT NULL")
        if not cur.fetchone()[0]:
            raise RuntimeError("Tabel sources/ingest_jobs belum ada. Jalankan backend versi terbaru "
                               "sekali (cargo run) agar skema dibuat.")


def queue_rechecks(conn, min_hours: float) -> int:
    with conn.cursor() as cur:
        cur.execute(
            """INSERT INTO ingest_jobs (source_id, log)
               SELECT s.id, 'Cek ulang terjadwal' FROM sources s
               WHERE s.auto_check
                 -- due after the source's own interval (daily / weekly / every N days); the
                 -- 4 h slack keeps a daily run from skipping a day when it starts a bit earlier
                 AND (s.last_checked IS NULL OR s.last_checked < NOW() - GREATEST(
                        %s * INTERVAL '1 hour',
                        COALESCE(s.check_every_days, 1) * INTERVAL '1 day' - INTERVAL '4 hours'))
                 AND NOT EXISTS (SELECT 1 FROM ingest_jobs j
                                 WHERE j.source_id = s.id AND j.state IN ('queued','running'))""",
            (min_hours,),
        )
        n = cur.rowcount
    conn.commit()
    return n


def queue_source(conn, source_id: int, note: str) -> bool:
    with conn.cursor() as cur:
        cur.execute("SELECT 1 FROM ingest_jobs WHERE source_id = %s AND state IN ('queued','running')",
                    (source_id,))
        if cur.fetchone():
            conn.commit()
            return False
        cur.execute("INSERT INTO ingest_jobs (source_id, log) VALUES (%s, %s)", (source_id, note))
    conn.commit()
    return True


# Kinds this worker processes. Web pages (web_site, web_page) are read by the backend itself
# (src/web_jobs.rs) with its HTML reader; they share the same job queue.
PDF_KINDS = ("drive_folder", "drive_file", "web_pdf")


def reset_stale_jobs(conn, hours: float = 3) -> int:
    """PDF jobs left 'running' by a crashed worker go back to the queue."""
    with conn.cursor() as cur:
        cur.execute("""UPDATE ingest_jobs j SET state = 'queued', log = j.log || E'\\n[worker] dikembalikan ke antrian'
                       FROM sources s
                       WHERE s.id = j.source_id AND s.kind IN %s
                         AND j.state = 'running' AND j.started_at < NOW() - (%s * INTERVAL '1 hour')""",
                    (PDF_KINDS, hours))
        n = cur.rowcount
    conn.commit()
    return n


# Same text as sources::queue_job's note in src/sources_api.rs (check_source)
ADMIN_RECHECK_NOTE = "Dicek ulang oleh admin"


def claim_job(conn) -> tuple[int, Source] | None:
    with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
        cur.execute(
            """UPDATE ingest_jobs SET state = 'running', started_at = NOW()
               WHERE id = (SELECT j.id FROM ingest_jobs j JOIN sources s ON s.id = j.source_id
                           WHERE j.state = 'queued' AND s.kind IN %s
                           ORDER BY j.id FOR UPDATE OF j SKIP LOCKED LIMIT 1)
               RETURNING id, source_id, log""",
            (PDF_KINDS,),
        )
        job = cur.fetchone()
        if not job:
            conn.commit()
            return None
        cur.execute("""SELECT id, kind, url, COALESCE(external_id,'') AS external_id, COALESCE(title,'') AS title,
                              COALESCE(category,'') AS category, COALESCE(subcategory,'') AS subcategory,
                              COALESCE(version_label,'') AS version_label
                       FROM sources WHERE id = %s""", (job["source_id"],))
        src = cur.fetchone()
    conn.commit()
    if not src:
        finish_job(conn, job["id"], "failed", "Sumber sudah dihapus", {})
        return claim_job(conn)
    force = (job["log"] or "").startswith(ADMIN_RECHECK_NOTE)
    return job["id"], Source(**src, force=force)


def finish_job(conn, job_id: int, state: str, log: str, stats: dict, source_file_id: int | None = None) -> None:
    with conn.cursor() as cur:
        cur.execute(
            """UPDATE ingest_jobs SET state = %s, log = %s, stats = %s::jsonb, finished_at = NOW(),
                      source_file_id = COALESCE(%s, source_file_id)
               WHERE id = %s RETURNING source_id""",
            (state, log, json.dumps(stats), source_file_id, job_id),
        )
        row = cur.fetchone()
        if row:
            cur.execute("UPDATE sources SET last_checked = NOW() WHERE id = %s", (row[0],))
    conn.commit()


def upsert_file(conn, source_id: int, external_id: str, name: str, citation_url: str) -> FileRow:
    with conn.cursor() as cur:
        cur.execute(
            """INSERT INTO source_files (source_id, external_id, name, citation_url)
               VALUES (%s, %s, %s, %s)
               ON CONFLICT (source_id, external_id) DO UPDATE
                 SET name = EXCLUDED.name,
                     citation_url = CASE WHEN source_files.mirror_url <> '' THEN source_files.citation_url
                                         ELSE EXCLUDED.citation_url END
               RETURNING id, COALESCE(sha256,''), COALESCE(remote_rev,''), status, COALESCE(document_id,''),
                         citation_url""",
            (source_id, external_id, name, citation_url),
        )
        row = FileRow(*cur.fetchone())
    conn.commit()
    return row


def document_status(conn, document_id: str) -> str | None:
    """'active' / 'draft' / 'archived' (whichever exists, active first), or None."""
    with conn.cursor() as cur:
        cur.execute("""SELECT status FROM documents WHERE document_id = %s
                       ORDER BY (status = 'active') DESC, (status = 'draft') DESC LIMIT 1""", (document_id,))
        row = cur.fetchone()
    conn.commit()
    return row[0] if row else None


def document_exists(conn, document_id: str) -> bool:
    if not document_id:
        return False
    with conn.cursor() as cur:
        cur.execute("SELECT 1 FROM documents WHERE document_id = %s LIMIT 1", (document_id,))
        found = cur.fetchone() is not None
    conn.commit()
    return found


def update_file(conn, file_id: int, **fields) -> None:
    if not fields:
        return
    cols = ", ".join(f"{k} = %s" for k in fields)
    with conn.cursor() as cur:
        cur.execute(f"UPDATE source_files SET {cols}, updated_at = NOW() WHERE id = %s",
                    (*fields.values(), file_id))
    conn.commit()


def find_duplicate(conn, sha: str, exclude_file_id: int) -> dict | None:
    with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
        cur.execute(
            """SELECT f.id, f.document_id, f.citation_url, f.mirror_url, f.status, s.kind, s.title
               FROM source_files f JOIN sources s ON s.id = f.source_id
               WHERE f.sha256 = %s AND f.id <> %s AND f.status IN ('active','draft')
               ORDER BY f.id LIMIT 1""",
            (sha, exclude_file_id),
        )
        return cur.fetchone()


def prefer_official_citation(conn, dup: dict, official_url: str) -> None:
    """Same file on Drive and on the website: cite the website, keep Drive as mirror."""
    with conn.cursor() as cur:
        cur.execute("""UPDATE source_files SET mirror_url = citation_url, citation_url = %s, updated_at = NOW()
                       WHERE id = %s""", (official_url, dup["id"]))
        if dup["document_id"]:
            cur.execute("UPDATE documents SET source_url = %s WHERE document_id = %s",
                        (official_url, dup["document_id"]))
    conn.commit()


def write_drafts(conn, file_id: int, document_id: str, rows: list[dict]) -> int:
    """Replace this file's draft chunks with `rows` in one transaction."""
    with conn.cursor() as cur:
        cur.execute("DELETE FROM documents WHERE status = 'draft' AND source_file_id = %s", (file_id,))
        cur.execute("DELETE FROM documents WHERE status = 'draft' AND document_id = %s", (document_id,))
        # An archived copy with the same id (the file went back to an older version, or the
        # source was removed and added again) keeps its history under a new id.
        cur.execute("""UPDATE documents SET document_id = document_id || '@archived-' || EXTRACT(EPOCH FROM NOW())::bigint
                       WHERE status = 'archived' AND document_id = %s""", (document_id,))
        psycopg2.extras.execute_values(
            cur,
            """INSERT INTO documents (document_id, title, content, document_type, source_url, category,
                                      subcategory, file_hash, embedding, page_number, chunk_index,
                                      status, source_file_id, chunk_kind, created_at, updated_at)
               VALUES %s""",
            [(
                document_id, r["title"], r["content"], "pdf", r["source_url"], r["category"],
                r["subcategory"], r["file_hash"], r["embedding"], r["page"], r["chunk_index"],
                "draft", file_id, r["kind"],
            ) for r in rows],
            template="(%s,%s,%s,%s,%s,%s,%s,%s,%s::vector,%s,%s,%s,%s,%s,NOW(),NOW())",
            page_size=200,
        )
    conn.commit()
    return len(rows)
