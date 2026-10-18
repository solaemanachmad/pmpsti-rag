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
    let link = match sources::parse_source_link(&body.url) {
        Ok(l) => l,
        Err(e) => return HttpResponse::BadRequest().json(ApiError::new(400, e)),
    };
    if let Some(v) = body.valid_from.as_deref().filter(|v| !v.trim().is_empty()) {
        let ok = v.len() == 10 && v.as_bytes()[4] == b'-' && v.as_bytes()[7] == b'-'
            && v.chars().filter(|c| c.is_ascii_digit()).count() == 8;
        if !ok {
            return HttpResponse::BadRequest().json(ApiError::new(400, "valid_from harus berformat YYYY-MM-DD"));
        }
    }
    let user_id = claims.sub.parse::<i64>().ok();
    match sources::create_source(state.db.pool(), &body, &link, user_id).await {
        Ok(id) => HttpResponse::Created().json(ApiSuccess::new(serde_json::json!({
            "id": id, "kind": link.kind, "url": link.url,
            "message": queued_message(trigger_worker()),
        }))),
        Err(e) if e.contains("sudah terdaftar") => HttpResponse::Conflict().json(ApiError::new(409, e)),
        Err(e) => server_error(e),
    }
}

/// POST /api/admin/sources/{id}/check — queue a re-check now.
pub async fn check_source(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match sources::queue_job(state.db.pool(), path.into_inner(), "Dicek ulang oleh admin").await {
        Ok(true) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "queued": true, "message": queued_message(trigger_worker()),
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
