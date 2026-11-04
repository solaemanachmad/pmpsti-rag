//! Abbreviations students type ("sempro", "TA", "tendik") — Admin → Abbreviations.
//!
//! - Candidates: short or upper-case words from query_logs that are not in any dictionary,
//!   most used (and most often unanswered) first, so the admin can explain them.
//! - Dictionary: the admin's entries (table `abbreviations`, no redeploy). Entries marked
//!   "ignored" are words the admin said are not abbreviations; they leave the candidate list.
//!   The built-in list `src/acronyms.json` is used at ingest only (enrich.rs): documents
//!   already carry "Satuan Kredit Semester (SKS)", so expanding SKS/PMPSTI/DTETI in questions
//!   only adds common words to the keyword search — measured as a loss (E11 in
//!   research/experiments.md: hybrid doc hit@5 98% → 93%).
//! - Query expansion: with Settings → "Expand abbreviations" on, a question is searched as
//!   "sempro (seminar proposal)". Only the search sees this; the log and the answer prompt keep
//!   the question as asked. Mirrored by scripts/eval/expand.py.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::handlers::{require_auth, AppState};
use crate::models::{ApiError, ApiSuccess, JwtClaims};

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id:         i64,
    /// as typed, lower case
    pub short:      String,
    pub expansion:  String,
    /// active | ignored
    pub status:     String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub short:        String,
    /// questions that use it
    pub times:        i64,
    /// of those, answered "not in the documents" or without sources
    pub unanswered:   i64,
    /// written in capitals at least once ("TA")
    pub upper:        bool,
    /// the word appears in an active document
    pub in_documents: bool,
    /// up to 3 recent questions that use it
    pub examples:     Vec<String>,
}

// ── Expansion ─────────────────────────────────────────────────────

/// "kapan sempro?" → "kapan sempro (seminar proposal)?". The first use of each of the admin's
/// short forms (any case, whole word) gets its meaning in brackets, unless the meaning is
/// already in the question. Built-in acronyms are not expanded here (see the module docs).
pub fn expand_query(text: &str, entries: &[(String, String)]) -> String {
    let lower_text = text.to_lowercase();
    let mut done: HashSet<String> = HashSet::new();
    let mut out = String::with_capacity(text.len() + 32);
    let mut word = String::new();

    let flush = |word: &mut String, out: &mut String, done: &mut HashSet<String>| {
        if word.is_empty() { return; }
        out.push_str(word);
        let key = word.to_lowercase();
        if !done.contains(&key) {
            if let Some((_, meaning)) = entries.iter().find(|(short, _)| short.to_lowercase() == key) {
                if !lower_text.contains(&meaning.to_lowercase()) {
                    out.push_str(" (");
                    out.push_str(meaning);
                    out.push(')');
                }
                done.insert(key);
            }
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut out, &mut done);
            out.push(c);
        }
    }
    flush(&mut word, &mut out, &mut done);
    out
}

// ── Dictionary in the database (cached for the search) ────────────

fn cache() -> &'static RwLock<Option<(Instant, Arc<Vec<(String, String)>>)>> {
    static C: RwLock<Option<(Instant, Arc<Vec<(String, String)>>)>> = RwLock::new(None);
    &C
}

fn invalidate() {
    if let Ok(mut c) = cache().write() { *c = None; }
}

/// The admin's active entries, read again at most once a minute (or after a change).
pub async fn active_entries(pool: &PgPool) -> Arc<Vec<(String, String)>> {
    if let Ok(c) = cache().read() {
        if let Some((at, list)) = c.as_ref() {
            if at.elapsed() < Duration::from_secs(60) { return list.clone(); }
        }
    }
    let list: Vec<(String, String)> = sqlx::query_as(
        "SELECT short, expansion FROM abbreviations WHERE status = 'active' ORDER BY short")
        .fetch_all(pool).await.unwrap_or_default();
    let list = Arc::new(list);
    if let Ok(mut c) = cache().write() { *c = Some((Instant::now(), list.clone())); }
    list
}

async fn all_entries(pool: &PgPool) -> Result<Vec<Entry>, String> {
    let rows = sqlx::query(
        "SELECT id, short, expansion, status, updated_at::text AS updated_at
         FROM abbreviations ORDER BY status, short")
        .fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| Entry {
        id: r.get("id"), short: r.get("short"), expansion: r.get("expansion"),
        status: r.get("status"), updated_at: r.get("updated_at"),
    }).collect())
}

/// Words from the questions of the last `days` days that look like abbreviations and are in no
/// dictionary: written in capitals, or short (2–6 letters) and not found in the documents.
async fn candidates(pool: &PgPool, days: i64) -> Result<Vec<Candidate>, String> {
    let rows = sqlx::query(
        r#"WITH words AS (
               SELECT q.id, q.outcome, q.query_text, w
               FROM query_logs q, regexp_split_to_table(q.query_text, '[^[:alnum:]]+') AS w
               WHERE q.created_at >= NOW() - ($1::bigint * INTERVAL '1 day')
                 AND length(w) BETWEEN 2 AND 8 AND w !~ '^[0-9]+$'
           ),
           doc_words AS (
               SELECT DISTINCT lower(w) AS w
               FROM documents d, regexp_split_to_table(d.content, '[^[:alnum:]]+') AS w
               WHERE d.status = 'active' AND length(w) BETWEEN 2 AND 8
           ),
           grouped AS (
               SELECT lower(w) AS short,
                      COUNT(DISTINCT id) AS times,
                      COUNT(DISTINCT id) FILTER (WHERE outcome IN ('not_found', 'no_sources')) AS unanswered,
                      bool_or(w ~ '^[A-Z][A-Z0-9]+$') AS upper,
                      (array_agg(DISTINCT query_text))[1:3] AS examples
               FROM words
               WHERE lower(w) NOT IN (SELECT short FROM abbreviations)
               GROUP BY lower(w)
           )
           SELECT g.*, EXISTS (SELECT 1 FROM doc_words dw WHERE dw.w = g.short) AS in_documents
           FROM grouped g
           ORDER BY g.unanswered DESC, g.times DESC
           LIMIT 400"#)
        .bind(days)
        .fetch_all(pool).await.map_err(|e| e.to_string())?;

    let builtin: HashSet<String> = crate::enrich::acronym_list().iter().map(|(s, _)| s.to_lowercase()).collect();
    Ok(rows.iter().map(|r| Candidate {
        short: r.get("short"), times: r.get("times"), unanswered: r.get("unanswered"),
        upper: r.get::<Option<bool>, _>("upper").unwrap_or(false),
        in_documents: r.get::<Option<bool>, _>("in_documents").unwrap_or(false),
        examples: r.get::<Option<Vec<String>>, _>("examples").unwrap_or_default(),
    })
    .filter(|c| !builtin.contains(&c.short) && !crate::db::is_keyword_stopword(&c.short))
    // capitals always; otherwise short words the documents do not contain
    .filter(|c| c.upper || (c.short.chars().count() <= 6 && !c.in_documents))
    .take(100)
    .collect())
}

// ── API: /api/admin/abbreviations ─────────────────────────────────

fn require_admin(req: &HttpRequest, state: &AppState) -> Result<JwtClaims, HttpResponse> {
    let claims = require_auth(req, &state.jwt_secret)?;
    if claims.role != "admin" {
        return Err(HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")));
    }
    Ok(claims)
}

/// GET /api/admin/abbreviations — the admin's entries, the built-in list and whether expansion is on
pub async fn list(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match all_entries(state.db.pool()).await {
        Ok(entries) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "entries": entries,
            "builtin": crate::enrich::acronym_list().iter()
                .map(|(s, e)| serde_json::json!({ "short": s, "expansion": e })).collect::<Vec<_>>(),
            "expand_enabled": crate::settings::get().expand_abbreviations,
        }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// GET /api/admin/abbreviations/candidates?days=90
pub async fn list_candidates(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let q = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
        .map(|q| q.into_inner()).unwrap_or_default();
    let days: i64 = q.get("days").and_then(|v| v.parse().ok()).unwrap_or(90).clamp(1, 3650);
    match candidates(state.db.pool(), days).await {
        Ok(c) => HttpResponse::Ok().json(ApiSuccess::new(c)),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

#[derive(Deserialize)]
pub struct SaveBody {
    pub short:     String,
    #[serde(default)]
    pub expansion: String,
    /// active (default) | ignored
    #[serde(default)]
    pub status:    String,
}

/// POST /api/admin/abbreviations {short, expansion} — add or change an entry;
/// {short, status: "ignored"} — not an abbreviation (hide it from the candidates)
pub async fn save(req: HttpRequest, state: web::Data<AppState>, body: web::Json<SaveBody>) -> HttpResponse {
    let claims = match require_admin(&req, &state) { Ok(c) => c, Err(r) => return r };
    let short = body.short.trim().to_lowercase();
    let expansion = body.expansion.split_whitespace().collect::<Vec<_>>().join(" ");
    let status = if body.status == "ignored" { "ignored" } else { "active" };
    if short.is_empty() || short.chars().count() > 16 || !short.chars().all(|c| c.is_alphanumeric()) {
        return HttpResponse::BadRequest().json(ApiError::new(400, "Short form: 1–16 letters or digits, no spaces"));
    }
    if status == "active" && (expansion.is_empty() || expansion.chars().count() > 120) {
        return HttpResponse::BadRequest().json(ApiError::new(400, "Expansion: 1–120 characters"));
    }
    let res = sqlx::query(
        "INSERT INTO abbreviations (short, expansion, status, updated_by)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (short) DO UPDATE SET expansion = EXCLUDED.expansion, status = EXCLUDED.status,
                                           updated_by = EXCLUDED.updated_by, updated_at = NOW()
         RETURNING id")
        .bind(&short).bind(&expansion).bind(status).bind(claims.sub.parse::<i64>().ok())
        .fetch_one(state.db.pool()).await;
    invalidate();
    match res {
        Ok(r) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "id": r.get::<i64, _>("id") }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e.to_string())),
    }
}

/// DELETE /api/admin/abbreviations/{id} — remove an entry (an ignored word becomes a candidate again)
pub async fn delete(req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let res = sqlx::query("DELETE FROM abbreviations WHERE id = $1").bind(path.into_inner())
        .execute(state.db.pool()).await;
    invalidate();
    match res {
        Ok(r) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "deleted": r.rows_affected() }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::expand_query;

    fn custom() -> Vec<(String, String)> {
        vec![("sempro".into(), "seminar proposal".into()), ("ta".into(), "tugas akhir".into())]
    }

    // The same cases are in scripts/eval/test_expand_query.py — keep them in step.
    #[test]
    fn expands_the_admins_short_forms_once_in_any_case() {
        assert_eq!(expand_query("kapan sempro? sempro lagi", &custom()), "kapan sempro (seminar proposal)? sempro lagi");
        assert_eq!(expand_query("syarat TA", &custom()), "syarat TA (tugas akhir)");
        assert_eq!(expand_query("data", &custom()), "data");
    }

    #[test]
    fn built_in_acronyms_are_not_expanded() {
        assert_eq!(expand_query("Berapa SKS tesis PMPSTI?", &[]), "Berapa SKS tesis PMPSTI?");
        assert_eq!(expand_query("jumlah tendik", &custom()), "jumlah tendik");
    }

    #[test]
    fn leaves_explained_and_unknown_words_alone() {
        assert_eq!(expand_query("seminar proposal (sempro) kapan", &custom()), "seminar proposal (sempro) kapan");
        assert_eq!(expand_query("halo", &custom()), "halo");
        assert_eq!(expand_query("", &custom()), "");
    }
}
