//! Background runner for web sources (kinds `web_site`, `web_page`).
//!
//! PDFs are processed by the Python worker (scripts/worker); web pages are read here with the
//! same reader as Add from URL (`handlers::read_and_chunk_page`). Jobs come from the shared
//! `ingest_jobs` queue: created by the admin (add source / add pages / Check now) or by the
//! scheduled re-check. Every changed page becomes a DRAFT document for review.

use std::sync::OnceLock;
use std::time::Duration;

use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tokio::sync::Notify;

use crate::site;
use crate::sources::WEB_KINDS;

const ADMIN_RECHECK_NOTE: &str = "Dicek ulang oleh admin";
const MAX_DISCOVERED_PAGES: usize = 150;
const PAGE_DELAY: Duration = Duration::from_millis(800);

static WAKE: OnceLock<Notify> = OnceLock::new();

fn wake_signal() -> &'static Notify {
    WAKE.get_or_init(Notify::new)
}

/// Process queued web jobs now instead of at the next poll.
pub fn wake() {
    wake_signal().notify_one();
}

/// Start the runner. Polls every minute and on `wake()`.
pub fn spawn(pool: PgPool) {
    actix_web::rt::spawn(async move {
        loop {
            requeue_stale(&pool).await;
            match claim(&pool).await {
                Ok(Some(job)) => {
                    let id = job.job_id;
                    // Own task, so a panic in the reader fails this job instead of stopping the runner
                    let task_pool = pool.clone();
                    let keep_awake = keep_awake();
                    let outcome = tokio::spawn(async move { run(&task_pool, job).await }).await;
                    keep_awake.abort();
                    let failure = match outcome {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e),
                        Err(join) => Some(format!("runner crashed: {join}")),
                    };
                    if let Some(e) = failure {
                        log::warn!("web job {id} failed: {e}");
                        for _ in 0..3 {
                            if finish(&pool, id, "failed", &format!("GAGAL: {e}"), serde_json::json!({})).await.is_ok() {
                                break;
                            }
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
                Ok(None) => {
                    let _ = tokio::time::timeout(Duration::from_secs(60), wake_signal().notified()).await;
                }
                Err(e) => {
                    log::warn!("web job queue unavailable: {e}");
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }
        }
    });
}

/// Jobs 'running' for over 30 minutes were cut off (Render restart or sleep): queue them again.
/// The age limit keeps a second instance (during a deploy) from taking over a live job.
async fn requeue_stale(pool: &PgPool) {
    let _ = sqlx::query(
        "UPDATE ingest_jobs j SET state = 'queued', log = j.log || E'\\n[runner] dikembalikan ke antrian'
         FROM sources s
         WHERE s.id = j.source_id AND s.kind = ANY($1) AND j.state = 'running'
           AND j.started_at < NOW() - INTERVAL '30 minutes'",
    ).bind(&WEB_KINDS[..]).execute(pool).await;
}

/// Render free sleeps after ~15 minutes without inbound requests, even mid-job. While a job runs,
/// request our own public /health every 5 minutes (RENDER_EXTERNAL_URL is set by Render).
fn keep_awake() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async {
        let Ok(base) = std::env::var("RENDER_EXTERNAL_URL") else { return };
        let http = reqwest::Client::new();
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let _ = http.get(format!("{}/health", base.trim_end_matches('/'))).send().await;
        }
    })
}

struct Job {
    job_id:      i64,
    source_id:   i64,
    kind:        String,
    root:        String,
    discover:    String,
    title:       String,
    category:    String,
    subcategory: String,
    version:     String,
    force:       bool,
}

async fn claim(pool: &PgPool) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query(
        "UPDATE ingest_jobs SET state = 'running', started_at = NOW()
         WHERE id = (SELECT j.id FROM ingest_jobs j JOIN sources s ON s.id = j.source_id
                     WHERE j.state = 'queued' AND s.kind = ANY($1)
                     ORDER BY j.id FOR UPDATE OF j SKIP LOCKED LIMIT 1)
         RETURNING id, source_id, log",
    ).bind(&WEB_KINDS[..]).fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    let source_id: i64 = row.get("source_id");
    let src = sqlx::query(
        "SELECT kind, url, discover, COALESCE(title,'') AS title, COALESCE(category,'') AS category,
                COALESCE(subcategory,'') AS subcategory, COALESCE(version_label,'') AS version_label
         FROM sources WHERE id = $1",
    ).bind(source_id).fetch_one(pool).await?;
    Ok(Some(Job {
        job_id: row.get("id"),
        source_id,
        kind: src.get("kind"),
        root: src.get("url"),
        discover: src.get("discover"),
        title: src.get("title"),
        category: src.get("category"),
        subcategory: src.get("subcategory"),
        version: src.get("version_label"),
        force: row.get::<String, _>("log").starts_with(ADMIN_RECHECK_NOTE),
    }))
}

async fn finish(pool: &PgPool, job_id: i64, state: &str, log: &str, stats: serde_json::Value) -> Result<(), sqlx::Error> {
    let row = sqlx::query(
        "UPDATE ingest_jobs SET state = $1, log = $2, stats = $3, finished_at = NOW() WHERE id = $4 RETURNING source_id",
    ).bind(state).bind(log).bind(stats).bind(job_id).fetch_optional(pool).await?;
    if let Some(r) = row {
        sqlx::query("UPDATE sources SET last_checked = NOW() WHERE id = $1")
            .bind(r.get::<i64, _>("source_id")).execute(pool).await?;
    }
    Ok(())
}

struct PageFile {
    id:          i64,
    url:         String,
    sha256:      String,
    status:      String,
    document_id: String,
}

async fn files_of(pool: &PgPool, source_id: i64) -> Result<Vec<PageFile>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, external_id, COALESCE(sha256,'') AS sha256, status, COALESCE(document_id,'') AS document_id
         FROM source_files WHERE source_id = $1 ORDER BY external_id",
    ).bind(source_id).fetch_all(pool).await?;
    Ok(rows.iter().map(|r| PageFile {
        id: r.get("id"), url: r.get("external_id"), sha256: r.get("sha256"),
        status: r.get("status"), document_id: r.get("document_id"),
    }).collect())
}

async fn run(pool: &PgPool, job: Job) -> Result<(), String> {
    let mut log: Vec<String> = Vec::new();
    let mut stats = serde_json::json!({ "pages": [] });

    if job.kind == "web_page" {
        sqlx::query("INSERT INTO source_files (source_id, external_id, name, citation_url) VALUES ($1,$2,'',$2)
                     ON CONFLICT (source_id, external_id) DO NOTHING")
            .bind(job.source_id).bind(&job.root).execute(pool).await.map_err(|e| e.to_string())?;
    }
    if job.kind == "web_site" && job.discover == "auto" {
        let known: Vec<String> = files_of(pool, job.source_id).await.map_err(|e| e.to_string())?
            .into_iter().map(|f| f.url).collect();
        let found = discover(&job.root, &known).await;
        let mut new = 0;
        for url in &found {
            let res = sqlx::query("INSERT INTO source_files (source_id, external_id, name, citation_url) VALUES ($1,$2,'',$2)
                                   ON CONFLICT (source_id, external_id) DO NOTHING")
                .bind(job.source_id).bind(url).execute(pool).await.map_err(|e| e.to_string())?;
            new += res.rows_affected();
        }
        log.push(format!("{} halaman ditemukan di bawah {}{}", found.len(), job.root,
            if new > 0 { format!(" ({new} halaman baru)") } else { String::new() }));
    }

    let files = files_of(pool, job.source_id).await.map_err(|e| e.to_string())?;
    let (mut drafts, mut unchanged, mut failed) = (0, 0, 0);
    for (i, f) in files.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(PAGE_DELAY).await;
        }
        let short = page_tag(&f.url, &job.root, &job.kind);
        match process_page(pool, &job, f).await {
            Ok(PageResult::Draft(n, warnings)) => {
                drafts += 1;
                log.push(format!("[{short}] {n} chunk draft → menunggu review admin"));
                log.extend(warnings.into_iter().map(|w| format!("[{short}] {w}")));
            }
            Ok(PageResult::Unchanged(note)) => {
                unchanged += 1;
                if job.kind == "web_page" || !note.is_empty() {
                    log.push(format!("[{short}] tidak berubah{note}"));
                }
            }
            Ok(PageResult::Note(msg)) => log.push(format!("[{short}] {msg}")),
            Err(e) => {
                failed += 1;
                log.push(format!("[{short}] GAGAL: {e}"));
            }
        }
        if let Some(arr) = stats["pages"].as_array_mut() {
            arr.push(serde_json::json!({ "url": f.url }));
        }
    }
    // Pages added by the admin while this job ran are picked up by a follow-up job
    let added_meanwhile: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM source_files WHERE source_id = $1 AND status = 'pending' AND NOT (id = ANY($2))",
    ).bind(job.source_id).bind(files.iter().map(|f| f.id).collect::<Vec<_>>())
        .fetch_one(pool).await.unwrap_or(0);
    if added_meanwhile > 0 {
        let _ = crate::sources::queue_job(pool, job.source_id, "Halaman baru ditambahkan saat proses berjalan").await;
    }
    log.insert(0, format!("{} halaman: {drafts} draft baru, {unchanged} tidak berubah, {failed} gagal",
        files.len()));
    let state = if failed > 0 && failed == files.len() { "failed" } else { "done" };
    finish(pool, job.job_id, state, &log.join("\n"), stats).await.map_err(|e| e.to_string())
}

/// Label of a page in the job log; the review screen matches it against the page's link.
fn page_tag(url: &str, root: &str, kind: &str) -> String {
    if kind == "web_site" {
        match url.strip_prefix(root) {
            Some("") => return "halaman utama".into(),
            Some(rest) => return rest.to_string(),
            None => {}
        }
    }
    let path = url.split("://").nth(1).and_then(|r| r.split_once('/')).map(|(_, p)| p).unwrap_or("");
    if path.is_empty() { url.to_string() } else { path.to_string() }
}

enum PageResult {
    Draft(usize, Vec<String>),
    Unchanged(String),
    Note(String),
}

fn page_document_id(url: &str, sha: &str) -> String {
    let path = url.split("://").nth(1).unwrap_or(url);
    let slug: String = path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let slug: String = slug.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
    format!("web-{}-{}", slug.trim_matches('-'), &sha[..8])
}

async fn delete_file_drafts(pool: &PgPool, file_id: i64) -> Result<(), String> {
    sqlx::query("DELETE FROM documents WHERE status = 'draft' AND source_file_id = $1")
        .bind(file_id).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn set_file(pool: &PgPool, id: i64, status: &str, sha: &str, document_id: &str, name: &str) -> Result<(), String> {
    sqlx::query("UPDATE source_files SET status = $2,
                    sha256 = CASE WHEN $3 = '' THEN sha256 ELSE $3 END,
                    document_id = CASE WHEN $4 = '' THEN document_id ELSE $4 END,
                    name = CASE WHEN $5 = '' THEN name ELSE $5 END,
                    updated_at = NOW()
                 WHERE id = $1")
        .bind(id).bind(status).bind(sha).bind(document_id).bind(name)
        .execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn process_page(pool: &PgPool, job: &Job, f: &PageFile) -> Result<PageResult, String> {
    let page = match crate::handlers::read_and_chunk_page(&f.url).await {
        Ok(p) => p,
        Err(e) if e.contains("HTTP 404") || e.contains("HTTP 410") => {
            set_file(pool, f.id, "missing", "", "", "").await?;
            return Ok(PageResult::Note("halaman tidak ditemukan (404) — versi aktif tetap dipakai sampai dihapus admin".into()));
        }
        Err(e) => return Err(e),
    };
    let sha = format!("{:x}", Sha256::digest(page.chunks.join("\n\u{1e}").as_bytes()));

    // Is the stored document still there? (chunks can be deleted by hand)
    let doc_present: bool = !f.document_id.is_empty() && sqlx::query("SELECT 1 FROM documents WHERE document_id = $1 LIMIT 1")
        .bind(&f.document_id).fetch_optional(pool).await.map_err(|e| e.to_string())?.is_some();
    // An active/draft page whose chunks were deleted by hand must be read again; a rejected page
    // has no chunks on purpose.
    let removed_by_hand = matches!(f.status.as_str(), "active" | "draft") && !doc_present;
    let settled: &[&str] = if job.force { &["active", "draft"] } else { &["active", "draft", "rejected"] };
    if sha == f.sha256 && !removed_by_hand && settled.contains(&f.status.as_str()) {
        return Ok(PageResult::Unchanged(if f.status == "rejected" { " (pernah ditolak admin)".into() } else { String::new() }));
    }

    let document_id = page_document_id(&f.url, &sha);
    let active: Option<String> = sqlx::query_scalar("SELECT status FROM documents WHERE document_id = $1 AND status = 'active' LIMIT 1")
        .bind(&document_id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
    if active.is_some() {
        // e.g. the page went back to the approved content: a pending draft of the other content
        // must not stay in review (approving it would archive the current version)
        delete_file_drafts(pool, f.id).await?;
        set_file(pool, f.id, "active", &sha, &document_id, &page.title).await?;
        return Ok(PageResult::Unchanged(" (sama dengan versi aktif)".into()));
    }

    // Same content as another page (e.g. an old copy of a chapter under a different URL)
    let dup: Option<(String, String)> = sqlx::query(
        "SELECT external_id, document_id FROM source_files
         WHERE sha256 = $1 AND id <> $2 AND status IN ('active','draft') LIMIT 1",
    ).bind(&sha).bind(f.id).fetch_optional(pool).await.map_err(|e| e.to_string())?
        .map(|r| (r.get("external_id"), r.get("document_id")));
    if let Some((other, doc)) = dup {
        delete_file_drafts(pool, f.id).await?;
        set_file(pool, f.id, "duplicate", &sha, &doc, &page.title).await?;
        return Ok(PageResult::Note(format!("isi sama dengan {other}; tidak disimpan dua kali")));
    }

    let title = if job.version.is_empty() { page.title.clone() } else { format!("{} ({})", page.title, job.version) };
    let embeddings = crate::handlers::embed_page_chunks(&page.title, &page.chunks).await?;
    let missing_vectors = embeddings.iter().filter(|v| v.is_empty()).count();

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM documents WHERE status = 'draft' AND (source_file_id = $1 OR document_id = $2)")
        .bind(f.id).bind(&document_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    // An archived copy with the same id keeps its history under a new id (unique index)
    sqlx::query("UPDATE documents SET document_id = document_id || '@archived-' || EXTRACT(EPOCH FROM NOW())::bigint
                 WHERE status = 'archived' AND document_id = $1")
        .bind(&document_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    for (i, (content, emb)) in page.chunks.iter().zip(embeddings.iter()).enumerate() {
        let vec = (!emb.is_empty()).then(|| pgvector::Vector::from(emb.clone()));
        let kind = if content.contains(" | ") && content.lines().count() > 2 { "table" } else { "text" };
        sqlx::query(
            "INSERT INTO documents (document_id, title, content, document_type, source_url, category, subcategory,
                                    file_hash, embedding, chunk_index, status, source_file_id, chunk_kind,
                                    created_at, updated_at)
             VALUES ($1,$2,$3,'url',$4,$5,$6,$7,$8,$9,'draft',$10,$11,NOW(),NOW())",
        )
        .bind(&document_id).bind(&title).bind(content).bind(&f.url)
        .bind(&job.category).bind(&job.subcategory).bind(&sha).bind(vec)
        .bind(i as i32).bind(f.id).bind(kind)
        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    set_file(pool, f.id, "draft", &sha, &document_id, &page.title).await?;

    let mut warnings = page.warnings;
    if missing_vectors > 0 {
        warnings.push(format!("embedding gagal untuk {missing_vectors} chunk (kuota?) — hanya ditemukan lewat kata kunci"));
    }
    if job.title.is_empty() && job.kind == "web_site" {
        let _ = sqlx::query("UPDATE sources SET title = $2 WHERE id = $1 AND COALESCE(title,'') = ''")
            .bind(job.source_id).bind(site_title(&page.title)).execute(pool).await;
    }
    Ok(PageResult::Draft(page.chunks.len(), warnings))
}

/// "1 Pendahuluan – Buku Panduan Akademik PMPSTI" → "Buku Panduan Akademik PMPSTI"
fn site_title(page_title: &str) -> String {
    page_title.rsplit_once(" – ").map(|(_, s)| s.trim().to_string()).unwrap_or_else(|| page_title.to_string())
}

/// Pages under `root`: from sitemap.xml if the site has one, else by following links from the
/// root and the known pages (breadth-first, same prefix only, at most MAX_DISCOVERED_PAGES).
async fn discover(root: &str, known: &[String]) -> Vec<String> {
    let http = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(Duration::from_secs(20))
        .build() { Ok(c) => c, Err(_) => return known.to_vec() };

    if let Ok(resp) = http.get(format!("{root}sitemap.xml")).send().await {
        if resp.status().is_success() {
            if let Ok(xml) = resp.text().await {
                let pages = site::sitemap_pages(&xml, root);
                if !pages.is_empty() {
                    let mut all = pages;
                    for k in known { if !all.contains(k) { all.push(k.clone()); } }
                    all.truncate(MAX_DISCOVERED_PAGES.max(known.len()));
                    return all;
                }
            }
        }
    }

    let mut queue: std::collections::VecDeque<String> = std::iter::once(root.to_string())
        .chain(known.iter().cloned()).collect();
    let mut seen: Vec<String> = Vec::new();
    let mut pages: Vec<String> = Vec::new();
    while let Some(url) = queue.pop_front() {
        if seen.contains(&url) || pages.len() >= MAX_DISCOVERED_PAGES {
            continue;
        }
        seen.push(url.clone());
        let Ok(resp) = http.get(&url).send().await else { continue };
        let html_page = resp.status().is_success() && resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).map(|v| v.contains("html")).unwrap_or(true);
        if !html_page { continue; }
        let Ok(html) = resp.text().await else { continue };
        pages.push(url.clone());
        for link in site::page_links(&html, &url, root) {
            if !seen.contains(&link) && !queue.contains(&link) {
                queue.push_back(link);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    for k in known { if !pages.contains(k) { pages.push(k.clone()); } }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_ids_are_stable_and_short() {
        let id = page_document_id("https://mtiugm.github.io/panduan_pmpsti/chapter1/introduction.html", "abcdef0123456789");
        assert_eq!(id, "web-mtiugm-github-io-panduan-pmpsti-chapter1-introduction-html-abcdef01");
        assert!(id.len() <= 80);
        assert_eq!(site_title("1 Pendahuluan – Buku Panduan Akademik PMPSTI"), "Buku Panduan Akademik PMPSTI");
        let root = "https://mtiugm.github.io/panduan_pmpsti/";
        assert_eq!(page_tag(root, root, "web_site"), "halaman utama");
        assert_eq!(page_tag(&format!("{root}front/b_preface.html"), root, "web_site"), "front/b_preface.html");
        assert_eq!(page_tag("https://jteti.ugm.ac.id/tenaga-pendidik/", "https://jteti.ugm.ac.id/tenaga-pendidik/", "web_page"), "tenaga-pendidik/");
    }
}
