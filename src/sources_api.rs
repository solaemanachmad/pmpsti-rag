//! Admin HTTP endpoints for link-based sources and draft review.
//! See docs/design/ingest-from-links.md §5.

use actix_web::{web, HttpRequest, HttpResponse};

use crate::handlers::{require_auth, AppState};
use crate::models::{ApiError, ApiSuccess, JwtClaims};
use crate::sources::{self, NewSource};

fn require_admin(req: &HttpRequest, state: &AppState) -> Result<JwtClaims, HttpResponse> {
    let claims = require_auth(req, &state.jwt_secret)?;
    if claims.role != "admin" {
        return Err(HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")));
    }
    Ok(claims)
}

/// Start the GitHub Actions worker right away (optional). Needs GITHUB_WORKER_TOKEN (fine-grained
/// token with "Actions: read and write" on the repo) and GITHUB_REPO ("owner/name"). Without them
/// queued jobs wait for the scheduled run. Returns true if a run was requested.
fn trigger_worker() -> bool {
    let (Ok(token), Ok(repo)) = (std::env::var("GITHUB_WORKER_TOKEN"), std::env::var("GITHUB_REPO")) else {
        return false;
    };
    if token.is_empty() || repo.is_empty() {
        return false;
    }
    let workflow = std::env::var("GITHUB_WORKER_WORKFLOW").unwrap_or_else(|_| "ingest-worker.yml".into());
    let branch = std::env::var("GITHUB_WORKER_REF").unwrap_or_else(|_| "main".into());
    actix_web::rt::spawn(async move {
        let url = format!("https://api.github.com/repos/{repo}/actions/workflows/{workflow}/dispatches");
        let res = reqwest::Client::new()
            .post(&url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "pmpsti-rag-backend")
            .json(&serde_json::json!({ "ref": branch }))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => log::info!("Ingest worker dispatched"),
            Ok(r) => log::warn!("Ingest worker dispatch failed: HTTP {}", r.status()),
            Err(e) => log::warn!("Ingest worker dispatch failed: {e}"),
        }
    });
    true
}

/// Start processing a queued source: web pages in this backend, PDFs in the GitHub worker.
fn start(kind: &str) -> &'static str {
    if sources::WEB_KINDS.contains(&kind) {
        crate::web_jobs::wake();
        "Sedang diproses; halaman yang berubah muncul di 'Waiting for review' dalam beberapa menit."
    } else {
        queued_message(trigger_worker())
    }
}

fn queued_message(dispatched: bool) -> &'static str {
    if dispatched {
        "Worker dijalankan; hasil draft biasanya muncul dalam beberapa menit."
    } else {
        "Masuk antrian; diproses pada jadwal worker berikutnya (atau jalankan workflow 'Ingest worker' secara manual)."
    }
}

fn server_error(e: String) -> HttpResponse {
    HttpResponse::InternalServerError().json(ApiError::new(500, e))
}

/// GET /api/admin/sources
pub async fn list_sources(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::list_sources(state.db.pool()).await {
        Ok(rows) => HttpResponse::Ok().json(ApiSuccess::new(rows)),
        Err(e) => server_error(e),
    }
}

/// POST /api/admin/sources  — register a Drive folder/file or PDF link and queue the first check.
pub async fn create_source(req: HttpRequest, state: web::Data<AppState>, body: web::Json<NewSource>) -> HttpResponse {
    let claims = match require_admin(&req, &state) { Ok(c) => c, Err(r) => return r };
    let mut link = match sources::parse_source_link(&body.url) {
        Ok(l) => l,
        Err(e) => return HttpResponse::BadRequest().json(ApiError::new(400, e)),
    };
    // The admin (after /resolve) may say what a plain web link is
    match (link.kind, body.kind.as_deref()) {
        ("web_page" | "web_pdf", Some("web_site")) => {
            let Some(root) = crate::site::normalize_root(&link.url) else {
                return HttpResponse::BadRequest().json(ApiError::new(400, "Link situs tidak valid"));
            };
            link = sources::ParsedLink { kind: "web_site", external_id: root.clone(), url: root };
        }
        ("web_page", Some("web_pdf")) => link.kind = "web_pdf", // a PDF link without ".pdf"
        _ => {}
    }
    if let Some(v) = body.valid_from.as_deref().filter(|v| !v.trim().is_empty()) {
        let ok = v.len() == 10 && v.as_bytes()[4] == b'-' && v.as_bytes()[7] == b'-'
            && v.chars().filter(|c| c.is_ascii_digit()).count() == 8;
        if !ok {
            return HttpResponse::BadRequest().json(ApiError::new(400, "valid_from harus berformat YYYY-MM-DD"));
        }
    }
    if let Err(e) = sources::valid_interval(body.check_every_days) {
        return HttpResponse::BadRequest().json(ApiError::new(400, e));
    }
    let user_id = claims.sub.parse::<i64>().ok();
    match sources::create_source(state.db.pool(), &body, &link, user_id).await {
        Ok(id) => HttpResponse::Created().json(ApiSuccess::new(serde_json::json!({
            "id": id, "kind": link.kind, "url": link.url,
            "message": start(link.kind),
        }))),
        Err(e) if e.contains("sudah terdaftar") => HttpResponse::Conflict().json(ApiError::new(409, e)),
        Err(e) if e.contains("tidak berada di bawah") || e.contains("tidak valid") || e.contains("minimal satu")
            || e.contains("sudah menjadi bagian") || e.contains("adalah PDF") =>
            HttpResponse::BadRequest().json(ApiError::new(400, e)),
        Err(e) => server_error(e),
    }
}

#[derive(serde::Deserialize)]
pub struct ResolveBody { pub url: String }

/// POST /api/admin/sources/resolve — what is this link, and does it belong to a known site?
/// Used by the Add source form before saving.
pub async fn resolve_link(req: HttpRequest, state: web::Data<AppState>, body: web::Json<ResolveBody>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let link = match sources::parse_source_link(&body.url) {
        Ok(l) => l,
        Err(e) => return HttpResponse::BadRequest().json(ApiError::new(400, e)),
    };
    if link.kind.starts_with("drive_") {
        return HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "kind": link.kind, "url": link.url })));
    }

    // 0. Already a page of some source?
    if link.kind == "web_page" {
        match sources::page_owner(state.db.pool(), &link.url, None).await {
            Ok(Some((id, title))) => return HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
                "kind": "web_page", "url": link.url, "existing": { "id": id, "title": title },
            }))),
            Ok(None) => {}
            Err(e) => return server_error(e),
        }
    }

    // 1. Part of a site that is already a source?
    let roots = match sources::site_roots(state.db.pool()).await { Ok(r) => r, Err(e) => return server_error(e) };
    let pairs: Vec<(i64, String)> = roots.iter().map(|(id, root, _)| (*id, root.clone())).collect();
    if let Some((id, root)) = crate::site::parent_for(&link.url, &pairs) {
        let title = roots.iter().find(|r| r.0 == id).map(|r| r.2.clone()).unwrap_or_default();
        return HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "kind": if link.kind == "web_pdf" { "web_pdf" } else { "web_page" },
            "url": link.url,
            "parent": { "id": id, "root": root, "title": title },
        })));
    }

    // 2. Look at the link itself: PDF, or a page whose site we can guess
    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap();
    // If the page cannot be opened right now, still offer a choice from the address alone;
    // the job reports the error when it runs.
    let offline = |why: String| HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
        "kind": link.kind,
        "url": link.url,
        "warning": why,
        "suggestion": crate::site::guess_site("", &link.url).filter(|g| g.root != link.url),
    })));
    let resp = match http.get(&link.url).send().await {
        Ok(r) => r,
        Err(e) => return offline(format!("Link tidak bisa dibuka sekarang ({e}); pilihan di bawah ditebak dari alamatnya.")),
    };
    if !resp.status().is_success() {
        return offline(format!("Link mengembalikan HTTP {}; periksa alamatnya.", resp.status().as_u16()));
    }
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if ctype.contains("pdf") {
        return HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "kind": "web_pdf", "url": link.url })));
    }
    let html = resp.text().await.unwrap_or_default();
    if html.trim_start().starts_with("%PDF") {
        return HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "kind": "web_pdf", "url": link.url })));
    }
    let guess = crate::site::guess_site(&html, &link.url);
    let page_title = scraper::Html::parse_document(&html)
        .select(&scraper::Selector::parse("title").unwrap()).next()
        .map(|e| e.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
        "kind": "web_page",
        "url": link.url,
        "page_title": page_title,
        // offered to the admin: "this page looks like part of <site_name> (<root>)"
        "suggestion": guess.filter(|g| g.root != link.url),
    })))
}

#[derive(serde::Deserialize)]
pub struct PagesBody { pub urls: Vec<String> }

/// POST /api/admin/sources/{id}/pages — add pages to a site source
pub async fn add_pages(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>, body: web::Json<PagesBody>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::add_pages(state.db.pool(), path.into_inner(), &body.urls).await {
        Ok(n) => {
            let message = if n > 0 { start("web_site") } else { "Semua halaman itu sudah terdaftar." };
            HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "added": n, "message": message })))
        }
        Err(e) if e.contains("tidak ditemukan") => HttpResponse::NotFound().json(ApiError::new(404, e)),
        Err(e) => HttpResponse::BadRequest().json(ApiError::new(400, e)),
    }
}

/// GET /api/admin/sources/{id}/files — pages of a site / files of a Drive folder
pub async fn list_files(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::list_files(state.db.pool(), path.into_inner()).await {
        Ok(rows) => HttpResponse::Ok().json(ApiSuccess::new(rows)),
        Err(e) => server_error(e),
    }
}

/// POST /api/admin/sources/{id}/approve-all — approve every draft of the source
pub async fn approve_all(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::approve_source(state.db.pool(), path.into_inner()).await {
        Ok((chunks, docs, archived)) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "approved_documents": docs, "activated_chunks": chunks, "archived_documents": archived
        }))),
        Err(e) => server_error(e),
    }
}

/// POST /api/admin/sources/{id}/check — queue a re-check now.
pub async fn check_source(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let id = path.into_inner();
    let kind = match sources::source_kind(state.db.pool(), id).await {
        Ok(Some((k, _))) => k,
        Ok(None) => return HttpResponse::NotFound().json(ApiError::new(404, "Sumber tidak ditemukan")),
        Err(e) => return server_error(e),
    };
    match sources::queue_job(state.db.pool(), id, "Dicek ulang oleh admin").await {
        Ok(true) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "queued": true, "message": start(&kind),
        }))),
        Ok(false) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "queued": false, "message": "Sudah ada pekerjaan yang menunggu atau berjalan"
        }))),
        Err(e) if e.contains("tidak ditemukan") => HttpResponse::NotFound().json(ApiError::new(404, e)),
        Err(e) => server_error(e),
    }
}

/// GET /api/admin/sources/{id}/jobs
pub async fn list_jobs(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::list_jobs(state.db.pool(), path.into_inner()).await {
        Ok(rows) => HttpResponse::Ok().json(ApiSuccess::new(rows)),
        Err(e) => server_error(e),
    }
}

/// PATCH /api/admin/sources/{id}
pub async fn update_source(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>, body: web::Json<sources::SourceUpdate>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::update_source(state.db.pool(), path.into_inner(), &body).await {
        Ok(true)  => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "updated": true }))),
        Ok(false) => HttpResponse::NotFound().json(ApiError::new(404, "Source not found")),
        Err(e) if e.starts_with("valid_from") || e.starts_with("check_every_days") => HttpResponse::BadRequest().json(ApiError::new(400, e)),
        Err(e) => server_error(e),
    }
}

/// DELETE /api/admin/sources/{id}
pub async fn delete_source(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::delete_source(state.db.pool(), path.into_inner()).await {
        Ok(()) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "deleted": true }))),
        Err(e) => server_error(e),
    }
}

/// GET /api/admin/drafts
pub async fn list_drafts(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::list_drafts(state.db.pool()).await {
        Ok(rows) => HttpResponse::Ok().json(ApiSuccess::new(rows)),
        Err(e) => server_error(e),
    }
}

/// GET /api/admin/drafts/{document_id}/chunks
pub async fn draft_chunks(req: HttpRequest, state: web::Data<AppState>, path: web::Path<String>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::draft_chunks(state.db.pool(), &path.into_inner()).await {
        Ok(rows) => HttpResponse::Ok().json(ApiSuccess::new(rows)),
        Err(e) => server_error(e),
    }
}

/// POST /api/admin/drafts/{document_id}/approve
pub async fn approve_draft(req: HttpRequest, state: web::Data<AppState>, path: web::Path<String>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::approve_draft(state.db.pool(), &path.into_inner()).await {
        Ok((n, archived)) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "activated_chunks": n, "archived_documents": archived
        }))),
        Err(e) if e.contains("tidak ditemukan") => HttpResponse::NotFound().json(ApiError::new(404, e)),
        Err(e) => server_error(e),
    }
}

/// POST /api/admin/drafts/{document_id}/reject
pub async fn reject_draft(req: HttpRequest, state: web::Data<AppState>, path: web::Path<String>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::reject_draft(state.db.pool(), &path.into_inner()).await {
        Ok(n) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "deleted_chunks": n }))),
        Err(e) => server_error(e),
    }
}
