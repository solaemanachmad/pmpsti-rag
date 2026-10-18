//! /api/admin/settings — Admin → Settings (see src/settings.rs).

use actix_web::{web, HttpRequest, HttpResponse};

use crate::handlers::{require_auth, AppState};
use crate::models::{ApiError, ApiSuccess, JwtClaims};
use crate::settings::{self, Settings};

fn require_admin(req: &HttpRequest, state: &AppState) -> Result<JwtClaims, HttpResponse> {
    let claims = require_auth(req, &state.jwt_secret)?;
    if claims.role != "admin" {
        return Err(HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")));
    }
    Ok(claims)
}

async fn snapshot(state: &AppState) -> serde_json::Value {
    let (provider, env_model, key_tail) = state.rag.provider_info();
    let history = settings::history(state.db.pool(), 20).await.unwrap_or_default();
    serde_json::json!({
        "settings": &*settings::get(),
        "defaults": settings::defaults(),
        "provider": provider,
        "env_model": env_model,
        // never the key itself: only whether it is set and its last 4 characters
        "api_key": { "set": key_tail.is_some(), "last4": key_tail },
        "history": history,
    })
}

/// GET /api/admin/settings
pub async fn get_settings(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    HttpResponse::Ok().json(ApiSuccess::new(snapshot(&state).await))
}

/// PUT /api/admin/settings
pub async fn put_settings(req: HttpRequest, state: web::Data<AppState>, body: web::Json<Settings>) -> HttpResponse {
    let claims = match require_admin(&req, &state) { Ok(c) => c, Err(r) => return r };
    match settings::save(state.db.pool(), body.into_inner(), claims.sub.parse().ok()).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new(snapshot(&state).await)),
        Err(e) if e.contains("must") || e.contains("model") || e.contains("at most") =>
            HttpResponse::BadRequest().json(ApiError::new(400, e)),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// POST /api/admin/settings/reset — back to the environment defaults
pub async fn reset_settings(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_admin(&req, &state) { Ok(c) => c, Err(r) => return r };
    match settings::reset(state.db.pool(), claims.sub.parse().ok()).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new(snapshot(&state).await)),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

#[derive(serde::Deserialize)]
pub struct TestBody { pub model: String }

/// POST /api/admin/settings/test {"model": "…"} — one tiny request to that model (uses 1 call of quota)
pub async fn test_settings(req: HttpRequest, state: web::Data<AppState>, body: web::Json<TestBody>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let model = body.model.trim().to_string();
    if let Err(e) = (Settings { model: model.clone(), ..Settings::default() }).validated() {
        return HttpResponse::BadRequest().json(ApiError::new(400, e));
    }
    let start = std::time::Instant::now();
    let result = state.rag.test_model(&model).await;
    let ms = start.elapsed().as_millis() as u64;
    HttpResponse::Ok().json(ApiSuccess::new(match result {
        Ok(reply) => serde_json::json!({ "ok": true, "model": model, "ms": ms, "reply": reply.chars().take(80).collect::<String>() }),
        Err(e) => {
            // keep the provider's message short and without the request URL
            let msg: String = e.to_string().chars().take(300).collect();
            serde_json::json!({ "ok": false, "model": model, "ms": ms, "error": msg })
        }
    }))
}
