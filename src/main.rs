mod auth;
mod db;
mod draft_check;
mod email;
mod enrich;
mod handlers;
mod models;
mod rag;
mod reader;
mod search;
mod site;
mod settings;
mod settings_api;
mod sources;
mod sources_api;
mod web_jobs;

use actix_web::{middleware, web, App, HttpServer, HttpResponse};
use dashmap::DashMap;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

use db::Database;
use handlers::AppState;
use db::seed_admin_user;
use rag::{LlmConfig, RagEngine};
use search::SearchEngine;

#[derive(Debug, Clone)]
struct Config {
    database_url:   String,
    jwt_secret:     String,
    host:           String,
    port:           u16,
    llm_backend:    String,
    llm_model:      String,
    llm_api_key:    String,
    llm_api_url:    String,
    gemini_api_key: String,
    gemini_model:   String,
    resend_api_key: String,
    app_base_url:   String,
    frontend_url:   String,
}

impl Config {
    fn from_env() -> Self {
        let _ = dotenvy::dotenv();
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgresql://admin:admin@localhost:5432/pmpsti_rag".to_string()),
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "ganti-ini-dengan-secret-yang-aman-minimal-32-karakter".to_string()),
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7860),
            llm_backend:    std::env::var("LLM_BACKEND").unwrap_or_else(|_| "ollama".to_string()),
            llm_model:      std::env::var("LLM_MODEL").unwrap_or_else(|_| "llama3.2".to_string()),
            llm_api_key:    std::env::var("LLM_API_KEY").unwrap_or_default(),
            llm_api_url:    std::env::var("LLM_API_URL").unwrap_or_default(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            gemini_model:   std::env::var("GEMINI_MODEL")
                .unwrap_or_else(|_| "gemini-3.5-flash-lite".to_string()),
            resend_api_key: std::env::var("RESEND_API_KEY").unwrap_or_default(),
            app_base_url:   std::env::var("APP_BASE_URL")
                .unwrap_or_else(|_| "https://pmpsti-rag.vercel.app".to_string()),
            frontend_url:   std::env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "https://pmpsti-rag.vercel.app".to_string()),
        }
    }
}

/// Versi DATABASE_URL yang aman untuk log: password diganti `****`,
/// query string (mis. `?sslmode=require`) dibuang.
fn mask_database_url(url: &str) -> String {
    let strip_query = |s: &str| s.split(['?', '#']).next().unwrap_or("").to_string();
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, url),
    };
    // Pisah userinfo dulu (pakai '@' terakhir) supaya '?'/'#' di password
    // tidak ikut terpotong dan bocor sebagian.
    let rest = match rest.rsplit_once('@') {
        Some((userinfo, host)) => {
            let host = strip_query(host);
            match userinfo.split_once(':') {
                Some((user, _password)) => format!("{user}:****@{host}"),
                None => format!("{userinfo}@{host}"),
            }
        }
        None => strip_query(rest),
    };
    match scheme {
        Some(s) => format!("{s}://{rest}"),
        None => rest,
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();

    let cfg = Config::from_env();

    log::info!("═══════════════════════════════════════");
    log::info!("  PMPSTI API");
    log::info!("  Database : {}", mask_database_url(&cfg.database_url));
    log::info!("  LLM      : {} ({})", if cfg.llm_backend == "gemini" { &cfg.gemini_model } else { &cfg.llm_model }, cfg.llm_backend);
    log::info!("  Listen   : {}:{}", cfg.host, cfg.port);
    log::info!("═══════════════════════════════════════");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&cfg.database_url)
        .await
        .expect("Gagal konek ke PostgreSQL");

    let db = Arc::new(Database::new(pool));
    db.setup().await.expect("Gagal setup database schema");
    seed_admin_user(&db).await;

    let search = SearchEngine::new(db.clone()).expect("Gagal load embedding model");

    let llm_config = match cfg.llm_backend.as_str() {
        "ollama" => LlmConfig::ollama(&cfg.llm_model),
        "gemini" => {
            if cfg.gemini_api_key.is_empty() {
                panic!("GEMINI_API_KEY wajib diisi di .env ketika LLM_BACKEND=gemini");
            }
            LlmConfig::gemini(&cfg.gemini_api_key, &cfg.gemini_model)
        }
        _ => {
            let url = if cfg.llm_api_url.is_empty() {
                "https://api.openai.com/v1/chat/completions".to_string()
            } else {
                cfg.llm_api_url.clone()
            };
            LlmConfig::openai_compatible(&url, &cfg.llm_api_key, &cfg.llm_model)
        }
    };

    for sql in draft_check::SCHEMA {
        if let Err(e) = sqlx::query(sql).execute(db.pool()).await { log::warn!("draft_checks schema skipped: {e}"); }
    }
    // Answer settings (Admin → Settings); defaults = the model from the environment
    settings::init(db.pool(), settings::Settings::from_env(&llm_config.model)).await;
    let rag   = Arc::new(RagEngine::new(search, llm_config));
    let login_attempts = Arc::new(DashMap::new());
    let state = web::Data::new(AppState {
        db:             db.clone(),
        rag:            rag.clone(),
        jwt_secret:     cfg.jwt_secret.clone(),
        resend_key:     cfg.resend_api_key.clone(),
        app_base_url:   cfg.app_base_url.clone(),
        login_attempts: login_attempts.clone(),
    });

    let addr = format!("{}:{}", cfg.host, cfg.port);
    log::info!("Server siap di http://{}", addr);

    // Background processing of web sources (Admin → Sources); PDFs go to the GitHub worker
    web_jobs::spawn(db.pool().clone());

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(
                web::JsonConfig::default().error_handler(|err, _| {
                    let msg = format!("JSON parse error: {}", err);
                    actix_web::error::InternalError::from_response(
                        err,
                        HttpResponse::BadRequest().json(models::ApiError::new(400, msg)),
                    ).into()
                })
            )
            .wrap(middleware::Logger::default())
            .wrap(
                actix_web::middleware::DefaultHeaders::new()
                    .add(("X-Content-Type-Options", "nosniff"))
                    .add(("X-Frame-Options", "DENY"))
                    .add(("X-XSS-Protection", "1; mode=block"))
                    .add(("Referrer-Policy", "strict-origin-when-cross-origin"))
                    .add(("Permissions-Policy", "camera=(), microphone=(), geolocation=()"))
                    .add(("Content-Security-Policy",
                        "default-src 'self'; script-src 'self'; object-src 'none'"))
            )
            .wrap({
                let origin = cfg.frontend_url.clone();
                actix_cors::Cors::default()
                    .allowed_origin(&origin)
                    .allowed_origin("http://localhost:5173")
                    .allowed_origin("http://localhost:4173")
                    .allow_any_method()
                    .allow_any_header()
                    .supports_credentials()
                    .max_age(3600)
            })
            .route("/health", web::get().to(handlers::health))
            .service(
                web::scope("/api/auth")
                    .route("/register",        web::post().to(handlers::register))
                    .route("/login",           web::post().to(handlers::login))
                    .route("/verify/{token}",  web::get().to(handlers::verify_email))
                    .route("/me",              web::get().to(handlers::get_me))
                    .route("/me",              web::patch().to(handlers::update_profile))
                    .route("/me/password",     web::patch().to(handlers::update_password))
                    .route("/logout",           web::post().to(handlers::logout_handler))
                    .route("/forgot-password",  web::post().to(handlers::forgot_password))
                    .route("/reset-password",   web::post().to(handlers::reset_password))
            )
            .service(
                web::scope("/api")
                    .route("/ask",        web::post().to(handlers::ask))
                    .route("/ask_public", web::post().to(handlers::ask_public))
                    .route("/faq",        web::get().to(handlers::faq_public))
                    .route("/categories", web::get().to(handlers::list_categories_handler))
                    .route("/sessions",             web::get().to(handlers::list_sessions))
                    .route("/sessions/{id}",        web::get().to(handlers::get_session_handler))
                    .route("/sessions/{id}",        web::delete().to(handlers::delete_session_handler))
                    .route("/sessions/{id}/title",  web::patch().to(handlers::rename_session_handler))
                    .route("/keys",       web::get().to(handlers::list_api_keys))
                    .route("/keys",       web::post().to(handlers::create_api_key_handler))
                    .route("/keys/{id}",  web::delete().to(handlers::revoke_api_key_handler))
                    .route("/admin/stats",              web::get().to(handlers::query_stats))
                    .route("/admin/users",              web::get().to(handlers::admin_list_users))
                    .route("/admin/users",              web::post().to(handlers::admin_create_user))
                    .route("/admin/users/{id}/role",    web::patch().to(handlers::admin_set_user_role))
                    .route("/admin/users/{id}/active",  web::patch().to(handlers::admin_toggle_user))
                    .route("/admin/users/{id}/verify",  web::patch().to(handlers::admin_verify_user))
                    .route("/admin/users/{id}",         web::delete().to(handlers::admin_delete_user))
                    .route("/admin/sessions",           web::get().to(handlers::admin_list_sessions))
                    .route("/admin/sessions/{id}",      web::delete().to(handlers::admin_delete_session))
                    .route("/admin/documents",                web::get().to(handlers::admin_list_documents))
                    .route("/admin/documents/ingest-url",     web::post().to(handlers::admin_ingest_url))
                    .route("/admin/documents/preview-url",    web::post().to(handlers::admin_preview_url))
                    .route("/admin/documents/ingest-text",    web::post().to(handlers::admin_ingest_text))
                    .route("/admin/chunks",                   web::get().to(handlers::admin_list_chunks))
                    .route("/admin/documents/{id}",           web::delete().to(handlers::admin_delete_document))
                    .route("/admin/documents/{id}",           web::patch().to(handlers::admin_update_document))
                    .route("/admin/chunks/{id}",              web::delete().to(handlers::admin_delete_chunk))
                    .route("/admin/chunks/{id}",              web::put().to(handlers::admin_update_chunk))
                    .route("/admin/logs",                web::get().to(handlers::admin_query_logs))
                    .route("/admin/settings",            web::get().to(settings_api::get_settings))
                    .route("/admin/settings",            web::put().to(settings_api::put_settings))
                    .route("/admin/settings/reset",      web::post().to(settings_api::reset_settings))
                    .route("/admin/settings/test",       web::post().to(settings_api::test_settings))
                    .route("/admin/unanswered",          web::get().to(handlers::admin_unanswered))
                    .route("/admin/unanswered/resolve",  web::post().to(handlers::admin_resolve_unanswered))
                    .route("/admin/faq/pins",            web::get().to(handlers::admin_faq_pins))
                    .route("/admin/faq/pins",            web::post().to(handlers::admin_faq_pin))
                    .route("/admin/faq/pins/{id}",       web::delete().to(handlers::admin_faq_unpin))
                    .route("/admin/faq/top",             web::get().to(handlers::admin_faq_top))
                    // Link-based sources (Drive / website PDFs) and draft review
                    .route("/admin/sources",                web::get().to(sources_api::list_sources))
                    .route("/admin/sources",                web::post().to(sources_api::create_source))
                    .route("/admin/sources/resolve",        web::post().to(sources_api::resolve_link))
                    .route("/admin/sources/{id}",           web::delete().to(sources_api::delete_source))
                    .route("/admin/sources/{id}",           web::patch().to(sources_api::update_source))
                    .route("/admin/sources/{id}/pages",     web::post().to(sources_api::add_pages))
                    .route("/admin/sources/{id}/files",     web::get().to(sources_api::list_files))
                    .route("/admin/sources/{id}/approve-all", web::post().to(sources_api::approve_all))
                    .route("/admin/sources/{id}/check",     web::post().to(sources_api::check_source))
                    .route("/admin/sources/{id}/jobs",      web::get().to(sources_api::list_jobs))
                    .route("/admin/drafts",                 web::get().to(sources_api::list_drafts))
                    .route("/admin/drafts/{id}/chunks",     web::get().to(sources_api::draft_chunks))
                    .route("/admin/drafts/{id}/approve",    web::post().to(sources_api::approve_draft))
                    .route("/admin/drafts/{id}/reject",     web::post().to(sources_api::reject_draft))
                    .route("/admin/drafts/{id}/test-questions", web::post().to(sources_api::test_questions))
            )
    })
    .bind(&addr)?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::mask_database_url;

    #[test]
    fn masks_password_and_strips_query() {
        assert_eq!(
            mask_database_url("postgresql://neondb_owner:s3cr3t@ep-x.neon.tech/neondb?sslmode=require&channel_binding=require"),
            "postgresql://neondb_owner:****@ep-x.neon.tech/neondb"
        );
    }

    #[test]
    fn masks_password_with_at_sign() {
        assert_eq!(
            mask_database_url("postgres://u:p@ss@localhost:5432/db"),
            "postgres://u:****@localhost:5432/db"
        );
        assert_eq!(
            mask_database_url("postgres://u:p?ss#x@localhost/db?sslmode=require"),
            "postgres://u:****@localhost/db"
        );
    }

    #[test]
    fn keeps_url_without_password() {
        assert_eq!(
            mask_database_url("postgres://user@localhost/db?sslmode=disable"),
            "postgres://user@localhost/db"
        );
        assert_eq!(mask_database_url("postgres://localhost/db"), "postgres://localhost/db");
    }
}
