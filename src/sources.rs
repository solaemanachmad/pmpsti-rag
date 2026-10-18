//! Document sources fetched from links (Google Drive folder/file, PDF on a website)
//! and the draft → active → archived lifecycle of their chunks.
//! Design: docs/design/ingest-from-links.md (§3 source types, §4 data model, §5 flow).
//!
//! The Rust backend only records sources and jobs and handles review; the Python worker
//! (`scripts/worker/`) does the fetching, PDF reading, chunking and embedding.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

// ══════════════════════════════════════════════════════════════════
//  SCHEMA
// ══════════════════════════════════════════════════════════════════

pub async fn setup_tables(pool: &PgPool) -> anyhow::Result<()> {
    let statements = [
        "CREATE TABLE IF NOT EXISTS sources (
            id            BIGSERIAL PRIMARY KEY,
            kind          TEXT NOT NULL,
            url           TEXT NOT NULL UNIQUE,
            external_id   TEXT DEFAULT '',
            title         TEXT DEFAULT '',
            category      TEXT DEFAULT '',
            subcategory   TEXT DEFAULT '',
            version_label TEXT DEFAULT '',
            valid_from    DATE,
            supersedes_id BIGINT REFERENCES sources(id) ON DELETE SET NULL,
            auto_check    BOOLEAN NOT NULL DEFAULT TRUE,
            last_checked  TIMESTAMPTZ,
            created_by    BIGINT,
            created_at    TIMESTAMPTZ DEFAULT NOW()
        )",
        "CREATE TABLE IF NOT EXISTS source_files (
            id            BIGSERIAL PRIMARY KEY,
            source_id     BIGINT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            external_id   TEXT NOT NULL,
            name          TEXT DEFAULT '',
            citation_url  TEXT NOT NULL,
            mirror_url    TEXT DEFAULT '',
            sha256        TEXT DEFAULT '',
            remote_rev    TEXT DEFAULT '',
            document_id   TEXT DEFAULT '',
            status        TEXT NOT NULL DEFAULT 'pending',
            updated_at    TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE (source_id, external_id)
        )",
        "CREATE TABLE IF NOT EXISTS ingest_jobs (
            id             BIGSERIAL PRIMARY KEY,
            source_id      BIGINT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            source_file_id BIGINT REFERENCES source_files(id) ON DELETE SET NULL,
            state          TEXT NOT NULL DEFAULT 'queued',
            log            TEXT NOT NULL DEFAULT '',
            stats          JSONB,
            created_at     TIMESTAMPTZ DEFAULT NOW(),
            started_at     TIMESTAMPTZ,
            finished_at    TIMESTAMPTZ
        )",
        "CREATE INDEX IF NOT EXISTS ingest_jobs_state_idx ON ingest_jobs(state, created_at)",
        "CREATE INDEX IF NOT EXISTS source_files_source_idx ON source_files(source_id)",
    ];
    for sql in statements {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

// ══════════════════════════════════════════════════════════════════
//  LINK CLASSIFICATION (pure — unit tested)
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLink {
    pub kind:        &'static str, // drive_folder | drive_file | web_pdf
    pub external_id: String,       // Drive id, or the URL itself for web_pdf
    pub url:         String,       // normalised URL stored in `sources.url`
}

fn drive_id_after<'a>(url: &'a str, marker: &str) -> Option<&'a str> {
    let start = url.find(marker)? + marker.len();
    let id: &str = url[start..]
        .split(|c: char| c == '/' || c == '?' || c == '#' || c == '&')
        .next()?;
    let valid = id.len() >= 10 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then_some(id)
}

/// Recognise a Google Drive folder, a Google Drive file, or a direct PDF/web link.
pub fn parse_source_link(raw: &str) -> Result<ParsedLink, String> {
    let url = raw.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Link harus diawali http:// atau https://".into());
    }
    let host = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_lowercase();

    if host == "drive.google.com" || host == "docs.google.com" {
        if let Some(id) = drive_id_after(url, "/folders/") {
            return Ok(ParsedLink {
                kind: "drive_folder",
                external_id: id.to_string(),
                url: format!("https://drive.google.com/drive/folders/{id}"),
            });
        }
        let file_id = drive_id_after(url, "/file/d/")
            .or_else(|| drive_id_after(url, "/document/d/"))
            .or_else(|| drive_id_after(url, "id="));
        if let Some(id) = file_id {
            return Ok(ParsedLink {
                kind: "drive_file",
                external_id: id.to_string(),
                url: format!("https://drive.google.com/file/d/{id}/view"),
            });
        }
        return Err("Link Google Drive tidak dikenali (butuh link folder atau file)".into());
    }

    let url = url.split('#').next().unwrap_or(url).to_string();
    Ok(ParsedLink { kind: "web_pdf", external_id: url.clone(), url })
}

/// Link shown to students for a cited source. Direct PDF links open at the first cited page;
/// the Google Drive viewer ignores `#page=N`, so Drive links are left unchanged.
pub fn citation_link(source_url: &str, pages: &[i32]) -> String {
    let base = source_url.split('#').next().unwrap_or(source_url);
    let path = base.split('?').next().unwrap_or(base).to_lowercase();
    match pages.first() {
        Some(p) if path.ends_with(".pdf") && !base.contains("drive.google.com") => format!("{base}#page={p}"),
        _ => source_url.to_string(),
    }
}

// ══════════════════════════════════════════════════════════════════
//  MODELS
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct NewSource {
    pub url:           String,
    #[serde(default)] pub title:         String,
    #[serde(default)] pub category:      String,
    #[serde(default)] pub subcategory:   String,
    #[serde(default)] pub version_label: String,
    /// YYYY-MM-DD
    #[serde(default)] pub valid_from:    Option<String>,
    #[serde(default)] pub supersedes_id: Option<i64>,
    #[serde(default = "default_true")] pub auto_check: bool,
}
fn default_true() -> bool { true }

#[derive(Debug, Serialize)]
pub struct SourceRow {
    pub id:            i64,
    pub kind:          String,
    pub url:           String,
    pub title:         String,
    pub category:      String,
    pub subcategory:   String,
    pub version_label: String,
    pub valid_from:    Option<String>,
    pub supersedes_id: Option<i64>,
    pub auto_check:    bool,
    pub last_checked:  Option<String>,
    pub created_at:    String,
    pub file_count:    i64,
    pub active_files:  i64,
    pub draft_files:   i64,
    pub last_job_state: Option<String>,
    pub last_job_log:   Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobRow {
    pub id:          i64,
    pub source_id:   i64,
    pub file_name:   Option<String>,
    pub state:       String,
    pub log:         String,
    pub stats:       Option<serde_json::Value>,
    pub created_at:  String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DraftDocument {
    pub document_id:   String,
    pub title:         String,
    pub source_id:     Option<i64>,
    pub source_title:  Option<String>,
    pub version_label: Option<String>,
    pub citation_url:  String,
    pub chunk_count:   i64,
    pub figure_chunks: i64,
    pub table_chunks:  i64,
    pub page_count:    i64,
    pub job_log:       Option<String>,
    pub replaces:      Vec<String>,   // active documents that approval will archive
}

// ══════════════════════════════════════════════════════════════════
//  QUERIES
// ══════════════════════════════════════════════════════════════════

pub async fn create_source(pool: &PgPool, s: &NewSource, link: &ParsedLink, user_id: Option<i64>) -> Result<i64, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let row = sqlx::query(
        "INSERT INTO sources (kind, url, external_id, title, category, subcategory, version_label,
                              valid_from, supersedes_id, auto_check, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8::date,$9,$10,$11)
         RETURNING id",
    )
    .bind(link.kind).bind(&link.url).bind(&link.external_id)
    .bind(s.title.trim()).bind(s.category.trim()).bind(s.subcategory.trim()).bind(s.version_label.trim())
    .bind(s.valid_from.as_deref().filter(|v| !v.trim().is_empty()))
    .bind(s.supersedes_id).bind(s.auto_check).bind(user_id)
    .fetch_one(&mut *tx).await
    .map_err(|e| if e.to_string().contains("sources_url_key") {
        "Link ini sudah terdaftar sebagai sumber".to_string()
    } else { e.to_string() })?;
    let id: i64 = row.get("id");
    sqlx::query("INSERT INTO ingest_jobs (source_id, log) VALUES ($1, 'Dibuat dari form admin')")
        .bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(id)
}

pub async fn list_sources(pool: &PgPool) -> Result<Vec<SourceRow>, String> {
    let rows = sqlx::query(
        "SELECT s.id, s.kind, s.url, s.title, s.category, s.subcategory, s.version_label,
                s.valid_from::text AS valid_from, s.supersedes_id, s.auto_check,
                to_json(s.last_checked)#>>'{}' AS last_checked, to_json(s.created_at)#>>'{}' AS created_at,
                (SELECT COUNT(*) FROM source_files f WHERE f.source_id = s.id) AS file_count,
                (SELECT COUNT(DISTINCT d.document_id) FROM documents d JOIN source_files f ON f.id = d.source_file_id
                  WHERE f.source_id = s.id AND d.status = 'active') AS active_files,
                (SELECT COUNT(DISTINCT d.document_id) FROM documents d JOIN source_files f ON f.id = d.source_file_id
                  WHERE f.source_id = s.id AND d.status = 'draft') AS draft_files,
                j.state AS last_job_state, j.log AS last_job_log
         FROM sources s
         LEFT JOIN LATERAL (
            SELECT state, log FROM ingest_jobs WHERE source_id = s.id ORDER BY id DESC LIMIT 1
         ) j ON TRUE
         ORDER BY s.created_at DESC",
    )
    .fetch_all(pool).await.map_err(|e| e.to_string())?;

    Ok(rows.iter().map(|r| SourceRow {
        id: r.get("id"),
        kind: r.get("kind"),
        url: r.get("url"),
        title: r.get::<Option<String>, _>("title").unwrap_or_default(),
        category: r.get::<Option<String>, _>("category").unwrap_or_default(),
        subcategory: r.get::<Option<String>, _>("subcategory").unwrap_or_default(),
        version_label: r.get::<Option<String>, _>("version_label").unwrap_or_default(),
        valid_from: r.get("valid_from"),
        supersedes_id: r.get("supersedes_id"),
        auto_check: r.get("auto_check"),
        last_checked: r.get("last_checked"),
        created_at: r.get::<Option<String>, _>("created_at").unwrap_or_default(),
        file_count: r.get("file_count"),
        active_files: r.get("active_files"),
        draft_files: r.get("draft_files"),
        last_job_state: r.get("last_job_state"),
        last_job_log: r.get("last_job_log"),
    }).collect())
}

/// Queue a (re)check of a source. Returns false if a job is already waiting or running.
pub async fn queue_job(pool: &PgPool, source_id: i64, note: &str) -> Result<bool, String> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM sources WHERE id = $1")
        .bind(source_id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err("Sumber tidak ditemukan".into());
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ingest_jobs WHERE source_id = $1 AND state IN ('queued','running')",
    )
    .bind(source_id).fetch_one(pool).await.map_err(|e| e.to_string())?;
    if pending > 0 {
        return Ok(false);
    }
    sqlx::query("INSERT INTO ingest_jobs (source_id, log) VALUES ($1, $2)")
        .bind(source_id).bind(note).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(true)
}

pub async fn list_jobs(pool: &PgPool, source_id: i64) -> Result<Vec<JobRow>, String> {
    let rows = sqlx::query(
        "SELECT j.id, j.source_id, f.name AS file_name, j.state, j.log, j.stats,
                to_json(j.created_at)#>>'{}' AS created_at, to_json(j.finished_at)#>>'{}' AS finished_at
         FROM ingest_jobs j LEFT JOIN source_files f ON f.id = j.source_file_id
         WHERE j.source_id = $1 ORDER BY j.id DESC LIMIT 50",
    )
    .bind(source_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| JobRow {
        id: r.get("id"),
        source_id: r.get("source_id"),
        file_name: r.get("file_name"),
        state: r.get("state"),
        log: r.get("log"),
        stats: r.get("stats"),
        created_at: r.get::<Option<String>, _>("created_at").unwrap_or_default(),
        finished_at: r.get("finished_at"),
    }).collect())
}

/// Delete a source with its files and jobs. Draft chunks are deleted; active chunks are
/// archived (kept for the record, no longer searchable).
pub async fn delete_source(pool: &PgPool, source_id: i64) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        "DELETE FROM documents WHERE status = 'draft'
           AND source_file_id IN (SELECT id FROM source_files WHERE source_id = $1)",
    ).bind(source_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE documents SET status = 'archived', updated_at = NOW()
         WHERE status = 'active' AND source_file_id IN (SELECT id FROM source_files WHERE source_id = $1)",
    ).bind(source_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM sources WHERE id = $1")
        .bind(source_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}

pub async fn list_drafts(pool: &PgPool) -> Result<Vec<DraftDocument>, String> {
    let rows = sqlx::query(
        "SELECT d.document_id,
                COALESCE(MIN(d.title), '') AS title,
                MIN(d.source_file_id) AS source_file_id,
                COALESCE(MIN(d.source_url), '') AS source_url,
                COUNT(*) AS chunk_count,
                COUNT(*) FILTER (WHERE d.chunk_kind = 'figure') AS figure_chunks,
                COUNT(*) FILTER (WHERE d.chunk_kind = 'table')  AS table_chunks,
                COUNT(DISTINCT d.page_number) AS page_count
         FROM documents d
         WHERE d.status = 'draft'
         GROUP BY d.document_id
         ORDER BY MIN(d.created_at) DESC",
    )
    .fetch_all(pool).await.map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let document_id: String = r.get("document_id");
        let file_id: Option<i64> = r.get("source_file_id");
        let (mut source_id, mut source_title, mut version_label, mut citation_url, mut job_log) =
            (None, None, None, r.get::<String, _>("source_url"), None);
        if let Some(fid) = file_id {
            if let Ok(Some(m)) = sqlx::query(
                "SELECT s.id, s.title, s.version_label, f.citation_url,
                        (SELECT log FROM ingest_jobs WHERE source_file_id = f.id ORDER BY id DESC LIMIT 1) AS job_log
                 FROM source_files f JOIN sources s ON s.id = f.source_id WHERE f.id = $1",
            ).bind(fid).fetch_optional(pool).await {
                source_id = Some(m.get::<i64, _>("id"));
                source_title = m.get("title");
                version_label = m.get("version_label");
                citation_url = m.get("citation_url");
                job_log = m.get("job_log");
            }
        }
        let replaces = documents_replaced_by(pool, &document_id).await.unwrap_or_default();
        out.push(DraftDocument {
            document_id,
            title: r.get("title"),
            source_id, source_title, version_label, citation_url,
            chunk_count: r.get("chunk_count"),
            figure_chunks: r.get("figure_chunks"),
            table_chunks: r.get("table_chunks"),
            page_count: r.get("page_count"),
            job_log,
            replaces,
        });
    }
    Ok(out)
}

/// Active documents that approving `draft_doc_id` will archive: earlier versions of the same
/// source file, plus all documents of the source this one supersedes.
const REPLACED_SQL: &str =
    "WITH draft AS (
        SELECT DISTINCT d.source_file_id, f.source_id
        FROM documents d LEFT JOIN source_files f ON f.id = d.source_file_id
        WHERE d.document_id = $1 AND d.status = 'draft'
     )
     SELECT DISTINCT d.document_id FROM documents d
     WHERE d.status = 'active' AND d.document_id <> $1 AND (
        d.source_file_id IN (SELECT source_file_id FROM draft WHERE source_file_id IS NOT NULL)
        OR d.source_file_id IN (
            SELECT f.id FROM source_files f JOIN sources s ON s.supersedes_id = f.source_id
            WHERE s.id IN (SELECT source_id FROM draft)
        )
     )";

pub async fn documents_replaced_by(pool: &PgPool, draft_doc_id: &str) -> Result<Vec<String>, String> {
    let rows = sqlx::query(REPLACED_SQL).bind(draft_doc_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| r.get("document_id")).collect())
}

/// Approve a draft: its chunks become searchable, replaced documents are archived.
pub async fn approve_draft(pool: &PgPool, document_id: &str) -> Result<(u64, Vec<String>), String> {
    let replaced = documents_replaced_by(pool, document_id).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    if !replaced.is_empty() {
        sqlx::query("UPDATE documents SET status = 'archived', updated_at = NOW()
                     WHERE status = 'active' AND document_id = ANY($1::text[])")
            .bind(&replaced).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        sqlx::query("UPDATE source_files SET status = 'archived', updated_at = NOW()
                     WHERE document_id = ANY($1::text[])")
            .bind(&replaced).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    let res = sqlx::query("UPDATE documents SET status = 'active', updated_at = NOW()
                           WHERE status = 'draft' AND document_id = $1")
        .bind(document_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        return Err("Draft tidak ditemukan".into());
    }
    sqlx::query("UPDATE source_files SET status = 'active', updated_at = NOW() WHERE document_id = $1")
        .bind(document_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok((res.rows_affected(), replaced))
}

/// Reject a draft: delete its chunks; the file is re-processed only when it changes again
/// or the admin queues a new check.
pub async fn reject_draft(pool: &PgPool, document_id: &str) -> Result<u64, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let res = sqlx::query("DELETE FROM documents WHERE status = 'draft' AND document_id = $1")
        .bind(document_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE source_files SET status = 'rejected', updated_at = NOW()
                 WHERE document_id = $1 AND status = 'draft'")
        .bind(document_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(res.rows_affected())
}

pub async fn draft_chunks(pool: &PgPool, document_id: &str) -> Result<Vec<serde_json::Value>, String> {
    let rows = sqlx::query(
        "SELECT id::bigint AS id, chunk_index, page_number, chunk_kind, content
         FROM documents WHERE document_id = $1 AND status = 'draft' ORDER BY chunk_index",
    ).bind(document_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| serde_json::json!({
        "id": r.get::<i64, _>("id"),
        "chunk_index": r.get::<Option<i32>, _>("chunk_index"),
        "page_number": r.get::<Option<i32>, _>("page_number"),
        "chunk_kind":  r.get::<String, _>("chunk_kind"),
        "content":     r.get::<Option<String>, _>("content").unwrap_or_default(),
    })).collect())
}

// ══════════════════════════════════════════════════════════════════
//  TESTS
// ══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_folder_link() {
        let p = parse_source_link("https://drive.google.com/drive/folders/1AbCdEfGhIjKlMnOp?usp=sharing").unwrap();
        assert_eq!(p.kind, "drive_folder");
        assert_eq!(p.external_id, "1AbCdEfGhIjKlMnOp");
        assert_eq!(p.url, "https://drive.google.com/drive/folders/1AbCdEfGhIjKlMnOp");
    }

    #[test]
    fn drive_file_links() {
        for url in [
            "https://drive.google.com/file/d/1ZyXwVuTsRqPoNm/view?usp=drive_link",
            "https://drive.google.com/open?id=1ZyXwVuTsRqPoNm",
            "https://drive.google.com/uc?export=download&id=1ZyXwVuTsRqPoNm",
            "https://docs.google.com/document/d/1ZyXwVuTsRqPoNm/edit",
        ] {
            let p = parse_source_link(url).unwrap();
            assert_eq!(p.kind, "drive_file", "{url}");
            assert_eq!(p.external_id, "1ZyXwVuTsRqPoNm", "{url}");
        }
    }

    #[test]
    fn web_pdf_link_and_errors() {
        let p = parse_source_link(" https://jteti.ugm.ac.id/wp-content/uploads/panduan.pdf#page=3 ").unwrap();
        assert_eq!(p.kind, "web_pdf");
        assert_eq!(p.url, "https://jteti.ugm.ac.id/wp-content/uploads/panduan.pdf");
        assert!(parse_source_link("ftp://x/y.pdf").is_err());
        assert!(parse_source_link("https://drive.google.com/drive/my-drive").is_err());
    }

    #[test]
    fn citation_links() {
        assert_eq!(citation_link("https://x.ac.id/a.pdf", &[24, 25]), "https://x.ac.id/a.pdf#page=24");
        assert_eq!(citation_link("https://x.ac.id/A.PDF?v=2", &[3]), "https://x.ac.id/A.PDF?v=2#page=3");
        assert_eq!(citation_link("https://drive.google.com/file/d/abc/view", &[3]), "https://drive.google.com/file/d/abc/view");
        assert_eq!(citation_link("https://x.ac.id/page.html", &[3]), "https://x.ac.id/page.html");
        assert_eq!(citation_link("https://x.ac.id/a.pdf", &[]), "https://x.ac.id/a.pdf");
    }
}
