use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::sleep;
use uuid::Uuid;

use crate::auth::{extract_bearer, generate_api_key, hash_password, sign_jwt, verify_jwt, verify_password};
use crate::db::{ChatMessage, ChatSession, ChatSource, Database, DocumentChunk};
use crate::email::{send_verification_email, send_password_reset_email, send_welcome_email};
use crate::models::*;
use crate::rag::RagEngine;

// ── Gemini Embed helper with retry ─────────────────────────────────────────
/// Embed `text` using Gemini gemini-embedding-001 with exponential backoff retry.
/// `task_type`: "RETRIEVAL_DOCUMENT" untuk chunk yang disimpan,
/// "SEMANTIC_SIMILARITY" untuk perbandingan antar kalimat (semantic chunking).
/// Query pencarian memakai "RETRIEVAL_QUERY" (lihat search.rs).
/// Retries up to 3 times on 429/503 (rate-limit / overload).
/// Returns the 768-dim embedding or an error string.
const EMBED_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent";

async fn embed_with_retry(
    http:       &reqwest::Client,
    embed_url:  &str,
    gemini_key: &str,
    text:       &str,
    task_type:  &str,
) -> Result<Vec<f32>, String> {
    #[derive(serde::Serialize)]
    struct EmbedReq {
        content: EmbedContent,
        #[serde(rename="taskType")] task_type: String,
        #[serde(rename="outputDimensionality")] output_dimensionality: u32,
    }
    #[derive(serde::Serialize)]
    struct EmbedContent { parts: Vec<EmbedPart> }
    #[derive(serde::Serialize)]
    struct EmbedPart { text: String }
    #[derive(serde::Deserialize)]
    struct EmbedResp { embedding: EmbedVals }
    #[derive(serde::Deserialize)]
    struct EmbedVals { values: Vec<f32> }

    // Proactive throttle: jeda antar setiap embed call untuk hindari 429.
    // gemini-embedding-001 free tier = 100 RPM → aman di 700ms (≈85 req/menit).
    // Tunable via env EMBED_DELAY_MS: 0=off (jika pakai tier berbayar), 700=default.
    let delay_ms: u64 = std::env::var("EMBED_DELAY_MS")
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or(700);
    if delay_ms > 0 {
        sleep(std::time::Duration::from_millis(delay_ms)).await;
    }

    let body = EmbedReq {
        content: EmbedContent { parts: vec![EmbedPart { text: text.to_string() }] },
        task_type: task_type.to_string(),
        output_dimensionality: 768,
    };

    let max_retries = 4u32;
    for attempt in 0..=max_retries {
        let resp = match http.post(embed_url)
            .header("x-goog-api-key", gemini_key)
            .json(&body)
            .send().await
        {
            Ok(r)  => r,
            Err(e) => {
                if attempt < max_retries {
                    let delay = std::time::Duration::from_millis(1000 * (1 << attempt));
                    sleep(delay).await;
                    continue;
                }
                return Err(format!("Network error: {e}"));
            }
        };

        let status = resp.status();

        // Retry-able transient errors: 429 (rate limit) dan 503 (overload)
        if status.as_u16() == 429 || status.as_u16() == 503 {
            if attempt < max_retries {
                // Exponential backoff: 2s, 4s, 8s, 16s
                let backoff = std::time::Duration::from_millis(2000 * (1 << attempt));
                eprintln!("[embed_retry] Gemini embed {status}, retry {}/{max_retries} after {}ms",
                    attempt + 1, backoff.as_millis());
                sleep(backoff).await;
                continue;
            }
            let msg = resp.text().await.unwrap_or_default();
            return Err(format!("Gemini embed {}: {msg}", status.as_u16()));
        }

        if !status.is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(format!("Gemini embed {}: {msg}", status.as_u16()));
        }

        return match resp.json::<EmbedResp>().await {
            Ok(d)  => Ok(d.embedding.values),
            Err(e) => Err(format!("Parse embed response: {e}")),
        };
    }

    Err("Embed failed after max retries".to_string())
}

// ── Document embeddings: one recipe, batched ───────────────────────────────
const BATCH_EMBED_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:batchEmbedContents";

/// The text embedded for a stored chunk: the document title, a blank line, the content.
/// The SAME recipe as `document_text` in scripts/gemini_embed.py (used by ingest.py and
/// regen_embeddings.py), so re-embedding a database reproduces what ingest stored.
/// The title is also sent as Gemini's `title` field (RETRIEVAL_DOCUMENT only).
fn document_embed_text(title: &str, content: &str) -> String {
    if title.is_empty() { content.to_string() } else { format!("{title}\n\n{content}") }
}

/// Embed a document's chunks with `batchEmbedContents`, `EMBED_BATCH_SIZE` (default 50) per
/// request — one call instead of one per chunk, which kept long pages past the proxy's
/// 60 s limit. Retries 429/503 with backoff. A batch that still fails gets empty vectors:
/// its chunks are saved without embeddings (still found by keyword search) and can be
/// filled later with `scripts/regen_embeddings.py`, which uses the same recipe.
async fn embed_documents(http: &reqwest::Client, gemini_key: &str, title: &str, chunks: &[String]) -> Vec<Vec<f32>> {
    #[derive(serde::Deserialize)]
    struct BatchResp { embeddings: Vec<Vals> }
    #[derive(serde::Deserialize)]
    struct Vals { values: Vec<f32> }

    let batch_size: usize = std::env::var("EMBED_BATCH_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(50).max(1);
    let batch_delay_ms: u64 = std::env::var("EMBED_BATCH_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
    let mut out: Vec<Vec<f32>> = Vec::with_capacity(chunks.len());

    for (b, batch) in chunks.chunks(batch_size).enumerate() {
        if b > 0 && batch_delay_ms > 0 {
            sleep(std::time::Duration::from_millis(batch_delay_ms)).await;
        }
        let requests: Vec<serde_json::Value> = batch.iter().map(|c| {
            let mut r = serde_json::json!({
                "model": "models/gemini-embedding-001",
                "content": { "parts": [{ "text": document_embed_text(title, c) }] },
                "taskType": "RETRIEVAL_DOCUMENT",
                "outputDimensionality": 768,
            });
            if !title.is_empty() {
                r["title"] = serde_json::json!(title);
            }
            r
        }).collect();
        let body = serde_json::json!({ "requests": requests });

        let mut vectors: Option<Vec<Vec<f32>>> = None;
        for attempt in 0..=4u32 {
            let resp = http.post(BATCH_EMBED_URL).header("x-goog-api-key", gemini_key).json(&body).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    match r.json::<BatchResp>().await {
                        Ok(d) if d.embeddings.len() == batch.len() => {
                            vectors = Some(d.embeddings.into_iter().map(|e| e.values).collect());
                        }
                        Ok(d) => eprintln!("[embed] batch {b}: {} vectors for {} chunks", d.embeddings.len(), batch.len()),
                        Err(e) => eprintln!("[embed] batch {b}: bad response: {e}"),
                    }
                    break;
                }
                Ok(r) if matches!(r.status().as_u16(), 429 | 503) && attempt < 4 => {
                    let backoff = 2000u64 << attempt; // 2, 4, 8, 16 s
                    eprintln!("[embed] batch {b}: Gemini {}, retry {}/4 after {backoff} ms", r.status(), attempt + 1);
                    sleep(std::time::Duration::from_millis(backoff)).await;
                }
                Ok(r) => {
                    let status = r.status();
                    eprintln!("[embed] batch {b}: Gemini {status}: {}", r.text().await.unwrap_or_default().chars().take(300).collect::<String>());
                    break;
                }
                Err(e) if attempt < 4 => {
                    eprintln!("[embed] batch {b}: network error {e}, retrying");
                    sleep(std::time::Duration::from_millis(1000 << attempt)).await;
                }
                Err(e) => {
                    eprintln!("[embed] batch {b}: network error {e}");
                    break;
                }
            }
        }
        match vectors {
            Some(v) => out.extend(v),
            None => {
                eprintln!("[embed] batch {b}: saving {} chunks without embeddings", batch.len());
                out.extend(std::iter::repeat_with(Vec::new).take(batch.len()));
            }
        }
    }
    out
}

// ── Chunking config (tune via env or change constants here) ────────────────
/// Similarity threshold: kalimat dengan cosine similarity < nilai ini → split baru.
/// Turunkan (e.g. 0.65) untuk chunk lebih kecil/granular,
/// naikkan (e.g. 0.85) untuk chunk lebih besar/kontekstual.
fn chunk_similarity_threshold() -> f32 {
    std::env::var("CHUNK_SIMILARITY_THRESHOLD")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(0.75)
}
/// Minimum ukuran chunk (chars). Chunk lebih kecil dari ini di-merge ke sebelumnya.
fn chunk_min_chars() -> usize {
    std::env::var("CHUNK_MIN_CHARS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(150)
}
/// Maximum ukuran chunk (chars). Chunk lebih besar dari ini di-force-split.
fn chunk_max_chars() -> usize {
    std::env::var("CHUNK_MAX_CHARS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(1200)
}

// ── Semantic Chunking ───────────────────────────────────────────────────────
/// Split `text` menjadi chunk-chunk yang semantically coherent.
///
/// Algoritma:
/// 1. Split teks menjadi kalimat/baris
/// 2. Embed tiap kalimat via Gemini
/// 3. Hitung cosine similarity antar kalimat berurutan
/// 4. Split saat similarity < threshold (default 0.75)
/// 5. Guard: merge chunk < min_chars, force-split chunk > max_chars
///
/// Returns list of chunk strings, atau error.
async fn semantic_chunk(
    http:        &reqwest::Client,
    embed_url:   &str,
    gemini_key:  &str,
    text:        &str,
    threshold:   f32,   // similarity threshold untuk split (0.0–1.0)
    min_chars:   usize, // chunk terlalu kecil → merge ke sebelumnya
    max_chars:   usize, // chunk terlalu besar → force split di tengah
) -> Result<Vec<String>, String> {
    // ── 1. Split jadi kalimat ───────────────────────────────────────
    // Split di ". ", ".\n", "\n\n", atau "\n" (untuk baris tabel)
    let sentences: Vec<String> = split_sentences(text);
    if sentences.is_empty() {
        return Ok(vec![text.to_string()]);
    }
    // Dokumen sangat pendek → langsung 1 chunk
    if sentences.len() <= 2 || text.len() < min_chars * 2 {
        return Ok(vec![text.to_string()]);
    }

    // ── 2. Embed tiap kalimat ────────────────────────────────────────
    let mut sent_embeddings: Vec<Vec<f32>> = Vec::with_capacity(sentences.len());
    for sentence in &sentences {
        let emb = embed_with_retry(http, embed_url, gemini_key, sentence, "SEMANTIC_SIMILARITY").await?;
        sent_embeddings.push(emb);
    }

    // ── 3. Hitung cosine similarity antar kalimat berurutan ──────────
    let mut similarities: Vec<f32> = Vec::with_capacity(sentences.len().saturating_sub(1));
    for i in 0..sent_embeddings.len().saturating_sub(1) {
        let sim = cosine_similarity(&sent_embeddings[i], &sent_embeddings[i + 1]);
        similarities.push(sim);
    }

    // ── 4. Split saat similarity drop ───────────────────────────────
    let mut segments: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = vec![sentences[0].clone()];

    for (i, sim) in similarities.iter().enumerate() {
        if *sim < threshold {
            // similarity drop → mulai segment baru
            segments.push(current.clone());
            current = vec![sentences[i + 1].clone()];
        } else {
            current.push(sentences[i + 1].clone());
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }

    // ── 5. Guard: merge terlalu kecil, split terlalu besar ──────────
    let mut chunks: Vec<String> = Vec::new();
    let mut pending = String::new();

    for seg in segments {
        let seg_text = seg.join(" ");
        pending.push_str(if pending.is_empty() { "" } else { " " });
        pending.push_str(&seg_text);

        if pending.len() >= min_chars {
            // Force-split kalau pending terlalu besar
            if pending.len() > max_chars {
                let chars: Vec<char> = pending.chars().collect();
                let mut start = 0;
                while start < chars.len() {
                    let end = (start + max_chars).min(chars.len());
                    chunks.push(chars[start..end].iter().collect());
                    if end >= chars.len() { break; }
                    start += max_chars;
                }
            } else {
                chunks.push(pending.clone());
            }
            pending = String::new();
        }
    }
    // Sisa pending → gabung ke chunk terakhir atau buat baru
    if !pending.is_empty() {
        if let Some(last) = chunks.last_mut() {
            if last.len() + pending.len() <= max_chars {
                last.push(' ');
                last.push_str(&pending);
            } else {
                chunks.push(pending);
            }
        } else {
            chunks.push(pending);
        }
    }

    if chunks.is_empty() {
        return Ok(vec![text.to_string()]);
    }
    Ok(chunks)
}

/// Split teks menjadi kalimat-kalimat.
/// Menangani: ". ", ".\n", "\n\n" (paragraf), "\n" (baris tabel).
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();

    // Deteksi teks tabel: banyak baris pendek (< 200 char rata-rata)
    let lines: Vec<&str> = text.lines().collect();
    let avg_line_len = if lines.is_empty() { 0 }
        else { lines.iter().map(|l| l.len()).sum::<usize>() / lines.len() };

    if avg_line_len < 150 && lines.len() > 3 {
        // Mode tabel: tiap baris = satu "kalimat"
        for line in lines {
            let s = line.trim().to_string();
            if !s.is_empty() { sentences.push(s); }
        }
        return sentences;
    }

    // Mode naratif: split di ". ", "! ", "? ", atau "\n\n"
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        current.push(c);

        let is_sentence_end = matches!(c, '.' | '!' | '?')
            && i + 1 < chars.len()
            && (chars[i + 1] == ' ' || chars[i + 1] == '\n');

        let is_para_break = c == '\n'
            && i + 1 < chars.len()
            && chars[i + 1] == '\n';

        if (is_sentence_end || is_para_break) && current.trim().len() > 20 {
            sentences.push(current.trim().to_string());
            current = String::new();
            if is_para_break { i += 1; } // skip extra newline
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }
    sentences
}

// ══════════════════════════════════════════════════════════════════
//  NON-SEMANTIC CHUNKING STRATEGIES (tidak butuh Gemini quota)
// ══════════════════════════════════════════════════════════════════

/// **sentence** — Split di batas kalimat dan paragraf, gabung sampai min_chars.
/// Tidak butuh embedding. Cocok untuk teks naratif panjang.
fn sentence_chunk(text: &str, min_chars: usize, max_chars: usize) -> Vec<String> {
    let sentences = split_sentences(text);
    merge_into_chunks(sentences, min_chars, max_chars)
}

/// **structural** — Tiap baris adalah unit. Baris yang diawali angka/bullet/header
/// dijadikan batas chunk baru. Sangat cocok untuk daftar dosen, tabel, SOP.
fn structural_chunk(text: &str, min_chars: usize, max_chars: usize) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Baris kosong = potensi batas blok
            if !current.trim().is_empty() && current.len() >= min_chars / 2 {
                // Jika cukup besar, simpan sebagai blok tersendiri
                // tapi biarkan terus bergabung dulu — cek di akhir
            }
            if !current.is_empty() { current.push('\n'); }
            continue;
        }

        // Deteksi header/entry baru: baris yang diawali digit+titik/paren,
        // atau "##", atau "---", atau huruf besar semua (≥4 huruf)
        let is_new_entry = {
            let is_numbered = trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                && (trimmed.contains(". ") || trimmed.contains(") "));
            let is_header = trimmed.starts_with("##") || trimmed.starts_with("---")
                || trimmed.starts_with("===");
            let is_allcaps = trimmed.len() >= 4
                && trimmed.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase())
                && trimmed.chars().filter(|c| c.is_alphabetic()).count() >= 4;
            is_numbered || is_header || is_allcaps
        };

        if is_new_entry && current.trim().len() >= min_chars {
            blocks.push(current.trim().to_string());
            current = String::new();
        }

        if !current.is_empty() { current.push('\n'); }
        current.push_str(trimmed);
    }
    if !current.trim().is_empty() {
        blocks.push(current.trim().to_string());
    }

    // Merge blok kecil, split blok besar
    merge_into_chunks(blocks, min_chars, max_chars)
}

/// **fixed** — Split setiap max_chars karakter, di batas kata jika memungkinkan.
fn fixed_chunk(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let chars: Vec<char> = text.chars().collect();
    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        // Mundur ke batas kata
        let split_at = if end < chars.len() {
            let mut s = end;
            while s > start && !chars[s].is_whitespace() { s -= 1; }
            if s == start { end } else { s }
        } else { end };
        chunks.push(chars[start..split_at].iter().collect::<String>().trim().to_string());
        start = split_at;
        // skip whitespace
        while start < chars.len() && chars[start].is_whitespace() { start += 1; }
    }
    chunks.into_iter().filter(|c| !c.is_empty()).collect()
}

/// Helper: merge Vec<String> unit menjadi chunks dengan min/max size.
fn merge_into_chunks(units: Vec<String>, min_chars: usize, max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut pending = String::new();

    for unit in units {
        if pending.is_empty() {
            pending = unit;
        } else {
            let sep = if pending.ends_with('\n') { "" } else { "\n" };
            let candidate = format!("{}{}{}", pending, sep, unit);
            if candidate.len() > max_chars && pending.len() >= min_chars {
                // pending sudah cukup besar, simpan dan mulai baru
                chunks.push(pending.clone());
                pending = unit;
            } else {
                pending = candidate;
            }
        }
        // Jika pending sudah terlalu besar, force split
        if pending.len() > max_chars {
            chunks.extend(fixed_chunk(&pending, max_chars));
            pending = String::new();
        }
    }
    if !pending.trim().is_empty() {
        if let Some(last) = chunks.last_mut() {
            if last.len() + pending.len() + 1 <= max_chars {
                last.push('\n');
                last.push_str(&pending);
            } else {
                chunks.push(pending);
            }
        } else {
            chunks.push(pending);
        }
    }
    if chunks.is_empty() { chunks.push(String::new()); }
    chunks.into_iter().filter(|c| c.trim().len() >= 10).collect()
}

/// Cosine similarity antara dua vektor f32.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { return 0.0; }
    dot / (norm_a * norm_b)
}

// ── Rate limiter state: IP -> (fail_count, window_start) ──
#[derive(Clone)]
pub struct LoginAttempt {
    count:      u32,
    window_start: Instant,
}

const MAX_LOGIN_FAILS:   u32 = 5;
const LOGIN_WINDOW_SECS: u64 = 15 * 60; // 15 menit

// ── App state yang di-share ke semua handler ──
pub struct AppState {
    pub db:           Arc<Database>,
    pub rag:          Arc<RagEngine>,
    pub jwt_secret:   String,
    pub resend_key:   String,
    pub app_base_url: String,
    pub login_attempts: Arc<DashMap<String, LoginAttempt>>,
}

// ══════════════════════════════════════════════════════════════════
//  MIDDLEWARE HELPER — ekstrak user dari JWT
// ══════════════════════════════════════════════════════════════════

pub(crate) fn require_auth(req: &HttpRequest, jwt_secret: &str) -> Result<JwtClaims, HttpResponse> {
    // Coba baca dari httpOnly cookie dulu
    let token_from_cookie = req.cookie("auth_token").map(|c| c.value().to_string());

    // Fallback: Authorization header (untuk API key / backward compat)
    let token_from_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| extract_bearer(v))
        .map(|s| s.to_string());

    let token = token_from_cookie
        .or(token_from_header)
        .ok_or_else(|| {
            HttpResponse::Unauthorized().json(ApiError::new(401, "Token tidak ditemukan"))
        })?;

    verify_jwt(&token, jwt_secret).map_err(|_| {
        HttpResponse::Unauthorized().json(ApiError::new(401, "Token tidak valid atau sudah expired"))
    })
}

/// The API key sent with a request: `Authorization: Bearer rag_…` or `X-API-Key: rag_…`.
fn api_key_from(req: &HttpRequest) -> Option<String> {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    header("X-API-Key")
        .or_else(|| header("Authorization").and_then(extract_bearer))
        .filter(|k| k.starts_with("rag_") && k.len() == 68)
        .map(str::to_string)
}

/// User for /api/ask: the signed-in user (cookie / JWT), or the owner of a valid API key
/// (created on the API Keys page). API keys are limited to their `rate_limit` requests per minute.
async fn ask_user(req: &HttpRequest, state: &AppState) -> Result<i64, HttpResponse> {
    let jwt_err = match require_auth(req, &state.jwt_secret) {
        Ok(c) => return Ok(c.sub.parse().unwrap_or(0)),
        Err(r) => r,
    };
    let Some(key) = api_key_from(req) else { return Err(jwt_err) };
    let invalid = || HttpResponse::Unauthorized().json(ApiError::new(401, "API key tidak valid atau sudah dicabut"));
    let Ok(Some((key_id, user_id, hash, _perms, rate_limit))) = state.db.lookup_api_key(&key[..12]).await else {
        return Err(invalid());
    };
    // Per-key limit per minute, counted before the (slow) hash check so wrong keys that share a
    // real prefix are limited too. Shares the in-memory map used for login attempts.
    {
        let now = Instant::now();
        let mut e = state.login_attempts.entry(format!("apikey:{key_id}"))
            .or_insert_with(|| LoginAttempt { count: 0, window_start: now });
        if e.window_start.elapsed().as_secs() >= 60 {
            e.count = 0;
            e.window_start = now;
        }
        if e.count >= rate_limit.max(1) as u32 {
            return Err(HttpResponse::TooManyRequests().json(ApiError::new(
                429, &format!("Batas {rate_limit} permintaan per menit untuk API key ini tercapai"))));
        }
        e.count += 1;
    }
    if !verify_password(&key, &hash).unwrap_or(false) {
        return Err(invalid());
    }
    let _ = state.db.touch_api_key(key_id).await;
    Ok(user_id)
}

// ══════════════════════════════════════════════════════════════════
//  AUTH HANDLERS
// ══════════════════════════════════════════════════════════════════

pub async fn register(
    state: web::Data<AppState>,
    body:  web::Json<RegisterRequest>,
) -> HttpResponse {
    // Validasi dasar
    if body.email.is_empty() || body.password.len() < 8 {
        return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Email tidak valid atau password kurang dari 8 karakter"));
    }

    // Izinkan email @mail.ugm.ac.id (mahasiswa) dan @ugm.ac.id (dosen/staf)
    let email_lower = body.email.to_lowercase();
    if !email_lower.ends_with("@mail.ugm.ac.id") && !email_lower.ends_with("@ugm.ac.id") {
        return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Registrasi hanya untuk email @mail.ugm.ac.id atau @ugm.ac.id"));
    }

    let hash = match tokio::task::spawn_blocking({
        let pw = body.password.clone();
        move || hash_password(&pw)
    })
    .await
    {
        Ok(Ok(h)) => h,
        _ => return HttpResponse::InternalServerError()
                .json(ApiError::new(500, "Gagal memproses password")),
    };

    let display = body.display_name.clone().unwrap_or_default();
    let user = match state.db.create_user(&body.email, &hash, &display).await {
        Ok(u)  => u,
        Err(e) => return HttpResponse::Conflict().json(ApiError::new(409, e)),
    };

    // Buat token verifikasi dan kirim email
    let token = Uuid::new_v4().to_string();
    if let Err(e) = state.db.create_verification_token(user.id, &token).await {
        log::error!("Gagal menyimpan verification token: {e}");
        return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal membuat token verifikasi"));
    }

    let verify_url = format!("{}/verify-email?token={}", state.app_base_url, token);
    if let Err(e) = send_verification_email(
        &state.resend_key,
        &user.email,
        &verify_url,
        &state.app_base_url,
    )
    .await
    {
        log::error!("Gagal mengirim email verifikasi: {e}");
        return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal mengirim email verifikasi"));
    }

    HttpResponse::Created().json(ApiSuccess::new(serde_json::json!({
        "message": "Registrasi berhasil. Silakan cek email Anda untuk verifikasi akun."
    })))
}

// ── Verifikasi email via token ──
pub async fn verify_email(
    state: web::Data<AppState>,
    path:  web::Path<String>,
) -> HttpResponse {
    let token = path.into_inner();
    match state.db.verify_email_token(&token).await {
        Ok(Some(_)) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "message": "Email berhasil diverifikasi. Silakan login."
        }))),
        Ok(None) => HttpResponse::BadRequest().json(ApiError::new(
            400,
            "Token verifikasi tidak valid atau sudah kedaluwarsa",
        )),
        Err(e) => {
            log::error!("verify_email_token error: {e}");
            HttpResponse::InternalServerError()
                .json(ApiError::new(500, "Gagal memverifikasi email"))
        }
    }
}

pub async fn login(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<LoginRequest>,
) -> HttpResponse {
    // ── Rate limiting: max 5 gagal per 15 menit per IP ──
    let ip = req
        .connection_info()
        .peer_addr()
        .unwrap_or("unknown")
        .to_string();

    {
        let now = Instant::now();
        let mut entry = state.login_attempts.entry(ip.clone()).or_insert_with(|| LoginAttempt {
            count: 0,
            window_start: now,
        });
        if entry.window_start.elapsed().as_secs() >= LOGIN_WINDOW_SECS {
            entry.count = 0;
            entry.window_start = now;
        }
        if entry.count >= MAX_LOGIN_FAILS {
            let remaining = LOGIN_WINDOW_SECS
                .saturating_sub(entry.window_start.elapsed().as_secs());
            return HttpResponse::TooManyRequests().json(ApiError::new(
                429,
                &format!(
                    "Terlalu banyak percobaan login. Coba lagi dalam {} menit.",
                    (remaining + 59) / 60
                ),
            ));
        }
    }

    let user = match state.db.get_user_by_email(&body.email).await {
        Ok(Some(u)) => u,
        _ => {
            // Increment fail counter agar tidak bisa brute-force enumeration
            if let Some(mut entry) = state.login_attempts.get_mut(&ip) {
                entry.count += 1;
            }
            return HttpResponse::Unauthorized()
                .json(ApiError::new(401, "Email atau password salah"));
        }
    };

    // Cek apakah email sudah diverifikasi
    if !user.email_verified {
        return HttpResponse::Forbidden().json(ApiError::new(
            403,
            "Akun belum diverifikasi. Silakan cek email Anda untuk link verifikasi.",
        ));
    }

    if !user.is_active {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Akun tidak aktif"));
    }

    let hash = user.password_hash.clone();
    let pw   = body.password.clone();
    let ok   = tokio::task::spawn_blocking(move || verify_password(&pw, &hash))
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);

    if !ok {
        if let Some(mut entry) = state.login_attempts.get_mut(&ip) {
            entry.count += 1;
        }
        return HttpResponse::Unauthorized()
            .json(ApiError::new(401, "Email atau password salah"));
    }

    // Login berhasil — reset counter
    state.login_attempts.remove(&ip);

    let (token, expires_in) =
        match sign_jwt(user.id, &user.email, &user.role, &state.jwt_secret) {
            Ok(t) => t,
            Err(_) => return HttpResponse::InternalServerError()
                .json(ApiError::new(500, "Gagal membuat token")),
        };

    let user_public = UserPublic {
        id:           user.id,
        email:        user.email,
        display_name: user.display_name,
        role:         user.role,
        created_at:   user.created_at,
    };

    // httpOnly cookie — not readable from JavaScript
    let cookie = auth_cookie(&req, &token, expires_in);

    HttpResponse::Ok()
        .append_header(("Set-Cookie", cookie))
        .json(ApiSuccess::new(AuthResponse {
            token: String::new(),   // kosong — token ada di cookie
            token_type: "Bearer".to_string(),
            expires_in,
            user: user_public,
        }))
}

/// Build the `auth_token` cookie. Production (https) needs `Secure; SameSite=None`
/// because the frontend (Vercel) and backend (Render) are on different domains.
/// Local development over plain `http://localhost` gets `SameSite=Lax` without
/// `Secure`: Safari drops `Secure` cookies on http://localhost, so login looked
/// successful but every next request failed with "Token tidak ditemukan".
fn auth_cookie(req: &HttpRequest, value: &str, max_age: u64) -> String {
    let info = req.connection_info();
    let attributes = cookie_attributes(info.scheme(), info.host());
    format!("auth_token={value}; HttpOnly; {attributes}; Path=/; Max-Age={max_age}")
}

/// `Secure; SameSite=None` unless the request is plain http to localhost / 127.0.0.1.
fn cookie_attributes(scheme: &str, host: &str) -> &'static str {
    let hostname = host.rsplit_once(':').map_or(host, |(name, _port)| name);
    let local = matches!(hostname, "localhost" | "127.0.0.1" | "[::1]");
    if scheme == "http" && local { "SameSite=Lax" } else { "Secure; SameSite=None" }
}

pub async fn logout_handler(req: HttpRequest) -> HttpResponse {
    // Clear the httpOnly cookie (same attributes as when it was set)
    let clear = auth_cookie(&req, "", 0);
    HttpResponse::Ok()
        .append_header(("Set-Cookie", clear))
        .json(serde_json::json!({"data": {"message": "Logout berhasil"}}))
}

pub async fn get_me(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.get_user_by_id(uid).await {
        Ok(Some(u)) => HttpResponse::Ok().json(ApiSuccess::new(UserPublic {
            id:           u.id,
            email:        u.email,
            display_name: u.display_name,
            role:         u.role,
            created_at:   u.created_at,
        })),
        _ => HttpResponse::NotFound().json(ApiError::new(404, "User tidak ditemukan")),
    }
}

pub async fn update_profile(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<UpdateProfileRequest>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.update_user_profile(
        uid,
        body.display_name.as_deref(),
        body.email.as_deref(),
    ).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("Profil berhasil diperbarui")),
        Err(e) => HttpResponse::BadRequest().json(ApiError::new(400, e)),
    }
}

pub async fn update_password(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<UpdatePasswordRequest>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uid: i64 = claims.sub.parse().unwrap_or(0);

    // Ambil user untuk verifikasi password lama
    let user = match state.db.get_user_by_id(uid).await {
        Ok(Some(u)) => u,
        _ => return HttpResponse::NotFound().json(ApiError::new(404, "User tidak ditemukan")),
    };

    let hash = user.password_hash.clone();
    let pw   = body.current_password.clone();
    let ok   = tokio::task::spawn_blocking(move || verify_password(&pw, &hash))
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);

    if !ok {
        return HttpResponse::Unauthorized()
            .json(ApiError::new(401, "Password saat ini tidak sesuai"));
    }

    if body.new_password.len() < 8 {
        return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Password baru minimal 8 karakter"));
    }

    let new_hash = match tokio::task::spawn_blocking({
        let pw = body.new_password.clone();
        move || hash_password(&pw)
    }).await {
        Ok(Ok(h)) => h,
        _ => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal memproses password")),
    };

    match state.db.update_user_password(uid, &new_hash).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("Password berhasil diperbarui")),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

// ══════════════════════════════════════════════════════════════════
//  RAG / CHAT HANDLERS
// ══════════════════════════════════════════════════════════════════


// ══════════════════════════════════════════════════════════════════
//  FILTER SOURCES — dynamic relevance filtering
// ══════════════════════════════════════════════════════════════════

/// Pick the search hits that are good enough to use as context (still per chunk;
/// `RagEngine::build_context` then merges them per document = per source card).
///
/// 1. Score threshold — drop chunks with score < min_score
/// 2. Score gap       — stop when the score falls by more than gap_ratio from the previous chunk
///                      (skipped for list/procedural questions: they need more parts)
/// 3. Max documents   — every hit from at most `max_docs` top documents
fn filter_sources<'a>(
    chunks:    &'a [crate::db::SearchResult],
    query:     &str,
    min_score: f64,
    gap_ratio: f64,
    max_docs:  usize,
) -> Vec<&'a crate::db::SearchResult> {
    use crate::rag::{RagEngine, QueryType};

    let Some(best) = chunks.first() else { return vec![] };
    let is_expand_query = matches!(
        RagEngine::classify_query_patterns(query),
        QueryType::List | QueryType::Procedural
    );

    let mut kept: Vec<&crate::db::SearchResult> = Vec::new();
    for c in chunks.iter().filter(|c| c.score >= min_score) {
        if let Some(prev) = kept.last() {
            if !is_expand_query && prev.score > 0.0 && (prev.score - c.score) / prev.score > gap_ratio {
                break;
            }
        }
        kept.push(c);
    }
    if kept.is_empty() {
        return vec![best]; // still give the LLM the single best context
    }

    let mut docs: Vec<&str> = Vec::new();
    kept.into_iter()
        .filter(|c| {
            if docs.contains(&c.document_id.as_str()) {
                return true;
            }
            if docs.len() >= max_docs {
                return false;
            }
            docs.push(&c.document_id);
            true
        })
        .collect()
}

pub async fn ask(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<AskRequest>,
) -> HttpResponse {
    let uid = match ask_user(&req, &state).await {
        Ok(u) => u,
        Err(r) => return r,
    };

    if body.query.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Query tidak boleh kosong"));
    }

    // Ambil atau buat session
    let session_id = body.session_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let session = state.db
        .get_session(&session_id, uid)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| ChatSession {
            id:         session_id.clone(),
            user_id:    uid,
            title:      body.query.chars().take(50).collect(),
            messages:   vec![],
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        });

    // Konversi history ke format Message untuk RAG
    let history: Vec<crate::rag::Message> = session.messages.iter().map(|m| {
        crate::rag::Message { role: m.role.clone(), content: m.content.clone() }
    }).collect();

    let tuning = crate::settings::get(); // Admin → Settings
    let start = Instant::now();
    let found = match retrieve(
        &state, &body.query, body.category_filter.as_deref(), body.search_mode.as_deref(),
        body.top_k.unwrap_or(tuning.candidates), tuning.max_sources,
    ).await {
        Ok(f) => f,
        Err(r) => return r,
    };

    // After the answer: save the turn to the session and log the question
    let finish = {
        let state = state.clone();
        let query = body.query.clone();
        let category = body.category_filter.clone().unwrap_or_default();
        let session_id = session_id.clone();
        move |answer: String, shown: Vec<crate::db::SearchResult>, n_contexts: usize, top_score: f32| async move {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let mut session = session;
            let now = Utc::now().to_rfc3339();
            session.messages.push(ChatMessage {
                role: "user".to_string(), content: query.clone(), created_at: now.clone(), sources: vec![],
            });
            session.messages.push(ChatMessage {
                role:       "assistant".to_string(),
                content:    answer.clone(),
                created_at: now.clone(),
                sources:    shown.iter().map(|c| ChatSource {
                    title:      clean_source_title(&c.title, &c.subcategory),
                    snippet:    c.snippet.clone(),
                    source_url: crate::sources::citation_link(&c.source_url, &c.pages),
                    category:   c.category.clone(),
                    subcategory: c.subcategory.clone(),
                    score:      c.score,
                    pages:      c.pages.clone(),
                }).collect(),
            });
            session.updated_at = now;
            let _ = state.db.save_session(&session).await;
            let _ = state.db.log_query(
                &query, "id", &category, n_contexts as i32, top_score, elapsed_ms as i64,
                Some(&session_id), Some(uid), answer_outcome(&answer, n_contexts),
            ).await;
            serde_json::json!({ "session_id": session_id, "search_time_ms": elapsed_ms })
        }
    };

    if body.stream.unwrap_or(false) {
        return stream_answer(state, body.query.clone(), history, found, finish);
    }

    // Call the LLM with exactly the same contexts as the source cards
    let refs: Vec<&crate::db::SearchResult> = found.contexts.iter().collect();
    let answer = match state.rag.answer_with_chunks(&body.query, &history, &refs, &found.query_type).await {
        Ok(r)  => r,
        Err(e) => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, format!("Gagal mendapat jawaban: {}", e))),
    };
    // Show only the sources the answer cites, renumbered to match the text
    let (answer, cited) = cited_sources(&answer, refs.len());
    let shown: Vec<crate::db::SearchResult> = cited.iter().map(|&i| found.contexts[i].clone()).collect();
    let sources = source_refs(&shown);
    let meta = finish(answer.clone(), shown, refs.len(), found.top_score).await;

    HttpResponse::Ok().json(ApiSuccess::new(AskResponse {
        answer,
        session_id,
        sources,
        search_time_ms: meta["search_time_ms"].as_u64().unwrap_or(0),
    }))
}

/// Search results ready for the LLM: one context per document (= source [n]).
pub(crate) struct Retrieved {
    pub contexts:   Vec<crate::db::SearchResult>,
    pub query_type: crate::rag::QueryType,
    pub top_score:  f32,
}

/// Search → filter → merge per document with surrounding context. The question type
/// (one LLM call) is worked out at the same time as the search and used twice: for how much
/// context to take and for the answer's format.
async fn retrieve(
    state:       &AppState,
    query:       &str,
    category:    Option<&str>,
    search_mode: Option<&str>,
    top_k:       i64,
    max_sources: usize,
) -> Result<Retrieved, HttpResponse> {
    // hybrid scored best in scripts/eval_retrieval.py (see PR #5); rrf was worse
    let search_mode = search_mode.unwrap_or("hybrid");
    let (all_chunks, query_type) = tokio::join!(
        state.rag.search_chunks(query, category, search_mode, top_k),
        state.rag.classify_query(query),
    );
    let all_chunks = all_chunks.map_err(|e| HttpResponse::InternalServerError()
        .json(ApiError::new(500, format!("Gagal mencari dokumen: {}", e))))?;
    let tuning = crate::settings::get();
    let hits = filter_sources(&all_chunks, query, tuning.min_score, 0.25, max_sources);
    let contexts = state.rag.build_context(&query_type, &hits).await;
    let top_score = all_chunks.first().map(|c| c.score as f32).unwrap_or(0.0);
    Ok(Retrieved { contexts, query_type, top_score })
}

fn source_refs(shown: &[crate::db::SearchResult]) -> Vec<SourceRef> {
    shown.iter().map(|c| SourceRef {
        title:      clean_source_title(&c.title, &c.subcategory),
        snippet:    c.snippet.clone(),
        source_url: crate::sources::citation_link(&c.source_url, &c.pages),
        category:   c.category.clone(),
        subcategory: c.subcategory.clone(),
        score:      c.score,
        pages:      c.pages.clone(),
    }).collect()
}

/// Stream an answer as NDJSON, one JSON object per line:
/// `{"type":"delta","text":…}` while the LLM writes (citations already renumbered), then
/// `{"type":"done","answer":…,"sources":[…], …fields from finish}` or `{"type":"error","error":…}`.
/// `finish(answer, cited contexts, number of contexts, top score)` saves/logs and returns extra
/// fields for the done line.
fn stream_answer<F, Fut>(
    state:   web::Data<AppState>,
    query:   String,
    history: Vec<crate::rag::Message>,
    found:   Retrieved,
    finish:  F,
) -> HttpResponse
where
    F: FnOnce(String, Vec<crate::db::SearchResult>, usize, f32) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = serde_json::Value> + Send + 'static,
{
    use tokio::io::AsyncWriteExt;
    let (mut writer, reader) = tokio::io::duplex(16 * 1024);

    actix_web::rt::spawn(async move {
        let refs: Vec<&crate::db::SearchResult> = found.contexts.iter().collect();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut cites = CitationRenumber::new(refs.len());
        let mut answer = String::new();

        async fn line(w: &mut tokio::io::DuplexStream, v: serde_json::Value) {
            let mut s = v.to_string();
            s.push('\n');
            let _ = w.write_all(s.as_bytes()).await; // the client may have gone; keep going
            let _ = w.flush().await;
        }

        let generate = async {
            let r = state.rag.answer_stream(&query, &history, &refs, &found.query_type, &tx).await;
            drop(tx);
            r
        };
        let forward = async {
            while let Some(delta) = rx.recv().await {
                let text = cites.push(&delta);
                if !text.is_empty() {
                    answer.push_str(&text);
                    line(&mut writer, serde_json::json!({ "type": "delta", "text": text })).await;
                }
            }
        };
        let (result, ()) = tokio::join!(generate, forward);

        let rest = cites.finish();
        if !rest.is_empty() {
            answer.push_str(&rest);
            line(&mut writer, serde_json::json!({ "type": "delta", "text": rest })).await;
        }
        if let Err(e) = result {
            log::warn!("[ask] streaming answer failed: {e}");
            line(&mut writer, serde_json::json!({ "type": "error", "error": format!("Gagal mendapat jawaban: {e}") })).await;
            return;
        }

        let shown: Vec<crate::db::SearchResult> = cites.order().iter().map(|&i| found.contexts[i].clone()).collect();
        let sources = source_refs(&shown);
        let mut done = finish(answer.clone(), shown, refs.len(), found.top_score).await;
        if let Some(obj) = done.as_object_mut() {
            obj.insert("type".into(), "done".into());
            obj.insert("answer".into(), answer.into());
            obj.insert("sources".into(), serde_json::to_value(sources).unwrap_or_default());
        }
        line(&mut writer, done).await;
        let _ = writer.shutdown().await;
    });

    HttpResponse::Ok()
        .content_type("application/x-ndjson; charset=utf-8")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(tokio_util::io::ReaderStream::new(reader))
}


// ══════════════════════════════════════════════════════════════════
//  PUBLIC ASK — tanpa auth, maks GUEST_QUOTA pertanyaan per token
// ══════════════════════════════════════════════════════════════════

const GUEST_QUOTA: i32 = 5;

pub async fn ask_public(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<AskRequest>,
) -> HttpResponse {
    // Guest token dari header X-Guest-Token
    let guest_token = req
        .headers()
        .get("X-Guest-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if guest_token.is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiError::new(400, "X-Guest-Token header diperlukan"));
    }

    // Cek quota
    let count = state.db.guest_ask_count(&guest_token).await;
    if count >= GUEST_QUOTA {
        return HttpResponse::TooManyRequests()
            .json(ApiError::new(429, format!(
                "Batas {} pertanyaan untuk tamu telah habis. Silakan daftar atau masuk.",
                GUEST_QUOTA
            )));
    }

    if body.query.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Query tidak boleh kosong"));
    }

    let tuning = crate::settings::get(); // Admin → Settings; guests: at most 5 candidates and sources
    let start = Instant::now();
    let found = match retrieve(
        &state, &body.query, None, body.search_mode.as_deref(),
        body.top_k.unwrap_or(tuning.candidates.min(5)), tuning.max_sources.min(5),
    ).await {
        Ok(f) => f,
        Err(r) => return r,
    };

    // After the answer: count the question and log it (no user, no session) so unanswered
    // ones show up for the admin
    let finish = {
        let state = state.clone();
        let query = body.query.clone();
        move |answer: String, _shown: Vec<crate::db::SearchResult>, n_contexts: usize, top_score: f32| async move {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let new_count = state.db.guest_increment(&guest_token).await;
            let _ = state.db.log_query(
                &query, "id", "", n_contexts as i32, top_score, elapsed_ms as i64,
                None, None, answer_outcome(&answer, n_contexts),
            ).await;
            serde_json::json!({
                "search_time_ms": elapsed_ms,
                "questions_used": new_count,
                "questions_left": (GUEST_QUOTA - new_count).max(0),
            })
        }
    };

    if body.stream.unwrap_or(false) {
        return stream_answer(state, body.query.clone(), vec![], found, finish);
    }

    let refs: Vec<&crate::db::SearchResult> = found.contexts.iter().collect();
    let answer = match state.rag.answer_with_chunks(&body.query, &[], &refs, &found.query_type).await {
        Ok(r)  => r,
        Err(e) => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, format!("Gagal mendapat jawaban: {}", e))),
    };
    // Show only the sources the answer cites, renumbered to match the text
    let (answer, cited) = cited_sources(&answer, refs.len());
    let shown: Vec<crate::db::SearchResult> = cited.iter().map(|&i| found.contexts[i].clone()).collect();
    let sources = source_refs(&shown);
    let mut res = finish(answer.clone(), shown, refs.len(), found.top_score).await;
    if let Some(obj) = res.as_object_mut() {
        obj.insert("answer".into(), answer.into());
        obj.insert("sources".into(), serde_json::to_value(sources).unwrap_or_default());
    }
    HttpResponse::Ok().json(ApiSuccess::new(res))
}

// ── Session management ──

pub async fn list_sessions(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.get_user_sessions(uid).await {
        Ok(sessions) => HttpResponse::Ok().json(ApiSuccess::new(sessions)),
        Err(e)       => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn get_session_handler(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.get_session(&path.into_inner(), uid).await {
        Ok(Some(s)) => HttpResponse::Ok().json(ApiSuccess::new(s)),
        Ok(None)    => HttpResponse::NotFound().json(ApiError::new(404, "Session tidak ditemukan")),
        Err(e)      => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn delete_session_handler(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.delete_session(&path.into_inner(), uid).await {
        Ok(true)  => HttpResponse::Ok().json(ApiSuccess::new("Session dihapus")),
        Ok(false) => HttpResponse::NotFound().json(ApiError::new(404, "Session tidak ditemukan")),
        Err(e)    => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn rename_session_handler(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
    body:  web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("");
    match state.db.rename_session(&path.into_inner(), uid, title).await {
        Ok(true)  => HttpResponse::Ok().json(ApiSuccess::new("Session diubah")),
        Ok(false) => HttpResponse::NotFound().json(ApiError::new(404, "Session tidak ditemukan")),
        Err(e)    => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

// ══════════════════════════════════════════════════════════════════
//  API KEY HANDLERS
// ══════════════════════════════════════════════════════════════════

pub async fn create_api_key_handler(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<CreateApiKeyRequest>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);

    let (full_key, prefix, key_hash) = match generate_api_key() {
        Ok(k)  => k,
        Err(_) => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal generate API key")),
    };

    let default_perms = vec!["search", "read"];
    let perms: Vec<&str> = body.permissions
        .as_ref()
        .map(|p| p.iter().map(|s| s.as_str()).collect())
        .unwrap_or(default_perms);

    let rate_limit = body.rate_limit.unwrap_or(30);

    match state.db.create_api_key(
        uid,
        &prefix,
        &key_hash,
        &body.name,
        &perms,
        rate_limit,
        body.expires_at.as_deref(),
    ).await {
        Ok(id) => HttpResponse::Created().json(ApiSuccess::new(CreateApiKeyResponse {
            id,
            key: full_key,   // ditampilkan SEKALI saja
            key_prefix: prefix,
            name: body.name.clone(),
        })),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn list_api_keys(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.get_api_keys(uid).await {
        Ok(keys) => HttpResponse::Ok().json(ApiSuccess::new(keys)),
        Err(e)   => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn revoke_api_key_handler(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i64>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    let uid: i64 = claims.sub.parse().unwrap_or(0);
    match state.db.revoke_api_key(path.into_inner(), uid).await {
        Ok(true)  => HttpResponse::Ok().json(ApiSuccess::new("API key dinonaktifkan")),
        Ok(false) => HttpResponse::NotFound().json(ApiError::new(404, "API key tidak ditemukan")),
        Err(e)    => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

// ══════════════════════════════════════════════════════════════════
//  ADMIN / STATS HANDLERS
// ══════════════════════════════════════════════════════════════════

pub async fn query_stats(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin yang bisa akses"));
    }
    match state.db.get_query_log_stats().await {
        Ok(stats) => HttpResponse::Ok().json(ApiSuccess::new(stats)),
        Err(e)    => HttpResponse::InternalServerError()
            .json(ApiError::new(500, format!("{}", e))),
    }
}

pub async fn list_categories_handler(
    state: web::Data<AppState>,
) -> HttpResponse {
    match state.db.list_categories().await {
        Ok(cats) => HttpResponse::Ok().json(ApiSuccess::new(cats)),
        Err(e)   => HttpResponse::InternalServerError()
            .json(ApiError::new(500, format!("{}", e))),
    }
}

pub async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "service": "pmpsti-rag-api"
    }))
}

// ══════════════════════════════════════════════════════════════════
//  ADMIN — USER MANAGEMENT
// ══════════════════════════════════════════════════════════════════

pub async fn admin_list_users(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_list_users().await {
        Ok(users) => HttpResponse::Ok().json(ApiSuccess::new(users)),
        Err(e)    => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_create_user(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let email = match body.get("email").and_then(|v| v.as_str()) {
        Some(e) if !e.is_empty() => e.to_string(),
        _ => return HttpResponse::BadRequest().json(ApiError::new(400, "Email wajib diisi")),
    };
    let display_name = body.get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let role = body.get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user")
        .to_string();

    // Generate random password — user can reset later
    use rand::RngExt;
    let tmp_pw: String = rand::rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    let hash = match tokio::task::spawn_blocking({
        let pw = tmp_pw.clone();
        move || hash_password(&pw)
    }).await {
        Ok(Ok(h)) => h,
        _ => return HttpResponse::InternalServerError()
                .json(ApiError::new(500, "Gagal hash password")),
    };

    let user = match state.db.create_user_with_role(&email, &hash, &display_name, &role).await {
        Ok(u)  => u,
        Err(e) => return HttpResponse::Conflict().json(ApiError::new(409, e)),
    };

    // Buat password reset token sebagai "set password" link
    let reset_token = Uuid::new_v4().to_string();
    let _ = state.db.create_password_reset_token(user.id, &reset_token).await;
    let reset_url = format!("{}/reset-password?token={}", state.app_base_url, reset_token);

    // Kirim welcome email (non-blocking — jangan gagalkan create user)
    let _ = send_welcome_email(&state.resend_key, &user.email, &reset_url, &state.app_base_url).await;

    HttpResponse::Created().json(ApiSuccess::new(serde_json::json!({
        "id": user.id,
        "email": user.email,
        "display_name": user.display_name,
        "role": role,
        "is_active": true,
        "created_at": user.created_at,
    })))
}

// ── Forgot Password ──
pub async fn forgot_password(
    state: web::Data<AppState>,
    body:  web::Json<serde_json::Value>,
) -> HttpResponse {
    let email = match body.get("email").and_then(|v| v.as_str()) {
        Some(e) if !e.is_empty() => e.to_string(),
        _ => return HttpResponse::BadRequest().json(ApiError::new(400, "Email wajib diisi")),
    };

    // Selalu return OK agar tidak bocorkan apakah email terdaftar
    let user = match state.db.get_user_by_email(&email).await {
        Ok(Some(u)) => u,
        _ => return HttpResponse::Ok().json(ApiSuccess::new(
            serde_json::json!({"message": "Jika email terdaftar, link reset akan dikirim."})
        )),
    };

    if !user.is_active {
        return HttpResponse::Ok().json(ApiSuccess::new(
            serde_json::json!({"message": "Jika email terdaftar, link reset akan dikirim."})
        ));
    }

    let token = Uuid::new_v4().to_string();
    if let Err(e) = state.db.create_password_reset_token(user.id, &token).await {
        log::error!("Gagal buat reset token: {e}");
        return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal membuat token reset"));
    }

    let reset_url = format!("{}/reset-password?token={}", state.app_base_url, token);
    if let Err(e) = send_password_reset_email(
        &state.resend_key, &email, &reset_url, &state.app_base_url
    ).await {
        log::error!("Gagal kirim reset email: {e}");
        return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal mengirim email reset"));
    }

    HttpResponse::Ok().json(ApiSuccess::new(
        serde_json::json!({"message": "Jika email terdaftar, link reset akan dikirim."})
    ))
}

// ── Reset Password ──
pub async fn reset_password(
    state: web::Data<AppState>,
    body:  web::Json<serde_json::Value>,
) -> HttpResponse {
    let token = match body.get("token").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return HttpResponse::BadRequest().json(ApiError::new(400, "Token wajib diisi")),
    };
    let new_password = match body.get("password").and_then(|v| v.as_str()) {
        Some(p) if p.len() >= 8 => p.to_string(),
        _ => return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Password minimal 8 karakter")),
    };

    let user_id = match state.db.consume_password_reset_token(&token).await {
        Ok(Some(id)) => id,
        Ok(None) => return HttpResponse::BadRequest()
            .json(ApiError::new(400, "Token tidak valid atau sudah kadaluarsa")),
        Err(e) => {
            log::error!("consume_password_reset_token error: {e}");
            return HttpResponse::InternalServerError()
                .json(ApiError::new(500, "Gagal memverifikasi token"));
        }
    };

    let hash = match tokio::task::spawn_blocking({
        let pw = new_password.clone();
        move || hash_password(&pw)
    }).await {
        Ok(Ok(h)) => h,
        _ => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal hash password")),
    };

    if let Err(e) = state.db.update_user_password(user_id, &hash).await {
        log::error!("update_user_password error: {e}");
        return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "Gagal update password"));
    }

    // Aktifkan + verifikasi email jika belum (user baru dari admin)
    let _ = sqlx::query(
        "UPDATE users SET email_verified = true, is_active = true WHERE id = $1"
    )
    .bind(user_id)
    .execute(&state.db.pool)
    .await;

    HttpResponse::Ok().json(ApiSuccess::new(
        serde_json::json!({"message": "Password berhasil diubah. Silakan login."})
    ))
}

pub async fn admin_set_user_role(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i64>,
    body:  web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let role = body.get("role").and_then(|v| v.as_str()).unwrap_or("user");
    match state.db.admin_set_role(path.into_inner(), role).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("Role diperbarui")),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_toggle_user(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i64>,
    body:  web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let active = body.get("is_active").and_then(|v| v.as_bool()).unwrap_or(true);
    match state.db.admin_set_active(path.into_inner(), active).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("Status user diperbarui")),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// Admin approve/verify user secara manual — bypass email verification.
pub async fn admin_verify_user(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i64>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_verify_user(path.into_inner()).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("User diverifikasi dan diaktifkan")),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

// ══════════════════════════════════════════════════════════════════
//  ADMIN — CHAT SESSIONS
// ══════════════════════════════════════════════════════════════════

pub async fn admin_delete_user(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i64>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let user_id = path.into_inner();
    // Prevent self-deletion
    if claims.sub.parse::<i64>().ok() == Some(user_id) {
        return HttpResponse::BadRequest().json(ApiError::new(400, "Tidak dapat menghapus akun sendiri"));
    }
    match state.db.admin_delete_user(user_id).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("User dihapus")),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_list_sessions(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_list_sessions().await {
        Ok(sessions) => HttpResponse::Ok().json(ApiSuccess::new(sessions)),
        Err(e)       => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_delete_session(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_delete_session(&path.into_inner()).await {
        Ok(true)  => HttpResponse::Ok().json(ApiSuccess::new("Session dihapus")),
        Ok(false) => HttpResponse::NotFound().json(ApiError::new(404, "Session tidak ditemukan")),
        Err(e)    => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

// ══════════════════════════════════════════════════════════════════
//  ADMIN — DOCUMENTS
// ══════════════════════════════════════════════════════════════════

pub async fn admin_list_documents(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_list_documents().await {
        Ok(docs) => HttpResponse::Ok().json(ApiSuccess::new(docs)),
        Err(e)   => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_delete_document(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_delete_document(&path.into_inner()).await {
        Ok(n)  => HttpResponse::Ok().json(ApiSuccess::new(
            serde_json::json!({ "deleted_chunks": n })
        )),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateDocumentBody {
    pub title:       String,
    pub category:    String,
    pub subcategory: String,
    /// Sent together; missing = keep the stored version.
    #[serde(default)]
    pub version_label: Option<String>,
    #[serde(default)]
    pub valid_from:    Option<String>,
}

pub async fn admin_update_document(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
    body:  web::Json<UpdateDocumentBody>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let doc_id = path.into_inner();
    let version = body.version_label.as_deref().map(|l| (l, body.valid_from.as_deref()));
    if let Some(d) = body.valid_from.as_deref().filter(|d| !d.trim().is_empty()) {
        if chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").is_err() {
            return HttpResponse::BadRequest().json(ApiError::new(400, "valid_from must be YYYY-MM-DD"));
        }
    }
    match state.db.admin_update_document(&doc_id, &body.title, &body.category, &body.subcategory, version).await {
        Ok(n)  => HttpResponse::Ok().json(ApiSuccess::new(
            serde_json::json!({ "updated_chunks": n })
        )),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// Phrases the model uses when the documents do not cover a question (system prompt rule 2).
const NOT_FOUND_PHRASES: &[&str] = &[
    "tidak tersedia dalam dokumen", "tidak tersedia di dokumen", "tidak terdapat dalam dokumen",
    "tidak ditemukan dalam dokumen", "tidak ditemukan di dokumen", "tidak ada dalam dokumen",
    "tidak ada informasi", "belum tersedia dalam dokumen", "tidak tercantum dalam dokumen",
    "informasi tersebut tidak tersedia", "tidak dapat menemukan informasi",
    "not available in the indexed documents", "not found in the documents", "no information about",
];

/// How a question went: "not_found" (the model said the documents do not cover it),
/// "no_sources" (nothing relevant was found; greetings land here too) or "answered".
pub(crate) fn answer_outcome(answer: &str, n_sources: usize) -> &'static str {
    let lower = answer.to_lowercase();
    if NOT_FOUND_PHRASES.iter().any(|p| lower.contains(p)) {
        "not_found"
    } else if n_sources == 0 {
        "no_sources"
    } else {
        "answered"
    }
}

/// Keep only the sources the answer cites. Citations are renumbered 1..k in order of first
/// use ("[3] … [1]" → "[1] … [2]"), so the list under the answer matches the numbers in it.
/// Returns the rewritten answer and the 0-based indexes of the cited sources, in that order.
/// An answer without citations (a greeting, "not in the documents") gets no sources;
/// numbers outside 1..=n_sources are removed from the text.
pub(crate) fn cited_sources(answer: &str, n_sources: usize) -> (String, Vec<usize>) {
    let mut c = CitationRenumber::new(n_sources);
    let mut out = c.push(answer);
    out.push_str(&c.finish());
    (out, c.order)
}

/// `cited_sources` for text that arrives in pieces: `push` returns what can be shown so far
/// (holding back a possibly unfinished "[1" at the end), `finish` returns the rest.
pub(crate) struct CitationRenumber {
    n_sources: usize,
    order:     Vec<usize>, // 0-based source index, by first citation
    pending:   String,
    out:       String,     // everything emitted, to avoid repeating "[1][1]"
}

impl CitationRenumber {
    pub fn new(n_sources: usize) -> Self {
        Self { n_sources, order: Vec::new(), pending: String::new(), out: String::new() }
    }

    pub fn order(&self) -> &[usize] { &self.order }

    pub fn push(&mut self, piece: &str) -> String {
        self.pending.push_str(piece);
        // Keep back a "[" whose citation may continue in the next piece
        let keep_from = match self.pending.rfind('[') {
            Some(open) => {
                let tail = &self.pending[open..];
                match tail.find(']') {
                    None if tail.len() <= 25 => open,
                    Some(close) if close + 1 == tail.len() => open, // "[2]" — is "(" next? (a link)
                    _ => self.pending.len(),
                }
            }
            None => self.pending.len(),
        };
        // ...with the space before it (or a trailing space: a citation may follow), which goes
        // if the citation is dropped
        let keep_from = if self.pending[..keep_from].ends_with(' ') {
            keep_from - 1
        } else {
            keep_from
        };
        let ready: String = self.pending.drain(..keep_from).collect();
        self.rewrite(&ready)
    }

    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.pending);
        self.rewrite(&rest)
    }

    fn rewrite(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(open) = rest.find('[') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some((nums, close)) = after.find(']').filter(|&close| close <= 24)
                .and_then(|close| parse_citation(&after[..close]).map(|n| (n, close)))
                .filter(|&(_, close)| !after[close + 1..].starts_with('(')) // a Markdown link, not a citation
            else {
                out.push('[');
                rest = after;
                continue;
            };
            let before = out.len();
            for n in nums.into_iter().filter(|&n| n >= 1 && n <= self.n_sources) {
                let pos = match self.order.iter().position(|&i| i == n - 1) {
                    Some(p) => p,
                    None => { self.order.push(n - 1); self.order.len() - 1 }
                };
                let mark = format!("[{}]", pos + 1);
                let repeated = if out.is_empty() { self.out.ends_with(&mark) } else { out.ends_with(&mark) };
                if !repeated { out.push_str(&mark); }
            }
            if out.len() == before && out.ends_with(' ') {
                out.pop(); // the whole citation was dropped: no "word ." left behind
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        self.out.push_str(&out);
        out
    }
}

/// "1", "1, 3", "1;2", "2-4" → the numbers; None when it is not a citation.
fn parse_citation(inner: &str) -> Option<Vec<usize>> {
    let mut nums = Vec::new();
    for part in inner.split([',', ';']) {
        let part = part.trim();
        let (a, b) = match part.split_once(['-', '–']) {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let (a, b): (usize, usize) = (a.parse().ok()?, b.parse().ok()?);
        if b < a || b - a > 10 { return None; }
        nums.extend(a..=b);
    }
    (!nums.is_empty()).then_some(nums)
}

/// Bersihkan judul sumber referensi untuk tampilan di frontend.
/// Prioritaskan subcategory (misal "1.2 Latar Belakang") daripada path title
/// (misal "chapter1/introduction"). Jika subcategory kosong, ambil bagian terakhir
/// dari title (setelah `/`), atau title itu sendiri.
fn clean_source_title(title: &str, subcategory: &str) -> String {
    // Subcategory biasanya format "1.2Latar Belakang" atau "1.2 Latar Belakang"
    // Tambah spasi antara angka dan huruf jika belum ada
    if !subcategory.trim().is_empty() {
        let sub = subcategory.trim();
        // Insert spasi setelah digit+titik pattern: "1.2Latar" -> "1.2 Latar"
        let re_result = {
            let mut out = String::with_capacity(sub.len() + 4);
            let chars: Vec<char> = sub.chars().collect();
            for (i, &c) in chars.iter().enumerate() {
                out.push(c);
                if i + 1 < chars.len() {
                    let next = chars[i + 1];
                    // After digit or dot, if next is uppercase/letter but no space
                    if (c.is_ascii_digit() || c == '.') && next.is_alphabetic() && next != ' ' {
                        // Check we haven't already got a space
                        out.push(' ');
                    }
                }
            }
            out
        };
        return re_result;
    }
    // Fallback: ambil bagian terakhir setelah '/'
    if let Some(last) = title.rsplit('/').next() {
        let last = last.trim();
        if !last.is_empty() {
            return last.to_string();
        }
    }
    title.to_string()
}



// ── Admin: chunk-level operations ─────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct ListChunksQuery {
    pub document_id: String,
}

pub async fn admin_list_chunks(
    req:   HttpRequest,
    state: web::Data<AppState>,
    query: web::Query<ListChunksQuery>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_list_chunks(&query.document_id).await {
        Ok(chunks) => HttpResponse::Ok().json(ApiSuccess::new(chunks)),
        Err(e)     => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_delete_chunk(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i32>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    match state.db.admin_delete_chunk(path.into_inner()).await {
        Ok(n)  => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "deleted": n }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateChunkBody {
    pub content: String,
}

pub async fn admin_update_chunk(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i32>,
    body:  web::Json<UpdateChunkBody>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    if body.content.trim().is_empty() {
        return HttpResponse::BadRequest().json(ApiError::new(400, "Konten tidak boleh kosong"));
    }
    match state.db.admin_update_chunk(path.into_inner(), &body.content).await {
        Ok(()) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "updated": true }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

// ── Admin: query logs ──────────────────────────────────────────────
// ══════════════════════════════════════════════════════════════════
//  FAQ — public + admin
// ══════════════════════════════════════════════════════════════════

/// GET /api/faq — publik, kembalikan daftar pertanyaan FAQ
pub async fn faq_public(state: web::Data<AppState>) -> HttpResponse {
    let limit = 8i64;
    match state.db.faq_public(limit).await {
        Ok(qs) => HttpResponse::Ok().json(ApiSuccess::new(qs)),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// GET /api/admin/faq/pins — list pinned questions
pub async fn admin_faq_pins(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" { return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")); }
    match state.db.faq_list_pins().await {
        Ok(pins) => HttpResponse::Ok().json(ApiSuccess::new(pins)),
        Err(e)   => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// GET /api/admin/faq/top — top queries dari logs (untuk admin pilih yang di-pin)
pub async fn admin_faq_top(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" { return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")); }
    let limit = 30i64;
    match state.db.faq_top_queries(limit).await {
        Ok(qs) => HttpResponse::Ok().json(ApiSuccess::new(qs)),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

#[derive(serde::Deserialize)]
pub struct FaqPinBody { pub question: String }

/// POST /api/admin/faq/pins — pin sebuah pertanyaan
pub async fn admin_faq_pin(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<FaqPinBody>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" { return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")); }
    let admin_id: i64 = claims.sub.parse().unwrap_or(0);
    if body.question.trim().is_empty() {
        return HttpResponse::BadRequest().json(ApiError::new(400, "Question kosong"));
    }
    match state.db.faq_pin(body.question.trim(), admin_id).await {
        Ok(id) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "id": id }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

/// DELETE /api/admin/faq/pins/{id} — unpin
pub async fn admin_faq_unpin(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<i64>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" { return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")); }
    match state.db.faq_unpin(path.into_inner()).await {
        Ok(_)  => HttpResponse::Ok().json(ApiSuccess::new("Unpinned")),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, e)),
    }
}

pub async fn admin_query_logs(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let query = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
        .unwrap_or_else(|_| web::Query(std::collections::HashMap::new()));
    let limit:  i64 = query.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50).min(200);
    let offset: i64 = query.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0).max(0);

    match state.db.admin_list_query_logs(limit, offset).await {
        Ok((logs, total)) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "logs": logs,
            "total": total,
            "limit": limit,
            "offset": offset,
        }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, format!("{e}"))),
    }
}

/// GET /api/admin/unanswered?days=30&include_no_sources=false
/// Questions the documents could not answer, grouped and counted — a to-do list for new sources.
pub async fn admin_unanswered(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    let query = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
        .unwrap_or_else(|_| web::Query(std::collections::HashMap::new()));
    let days: i64 = query.get("days").and_then(|v| v.parse().ok()).unwrap_or(30).clamp(1, 365);
    let no_sources = query.get("include_no_sources").map(|v| v == "true").unwrap_or(false);
    match state.db.unanswered_questions(days, no_sources).await {
        Ok(rows) => HttpResponse::Ok().json(ApiSuccess::new(rows)),
        Err(e)   => HttpResponse::InternalServerError().json(ApiError::new(500, format!("{e}"))),
    }
}

#[derive(serde::Deserialize)]
pub struct ResolveBody { pub ids: Vec<i64> }

/// POST /api/admin/unanswered/resolve {"ids":[…]} — mark questions as handled.
pub async fn admin_resolve_unanswered(req: HttpRequest, state: web::Data<AppState>, body: web::Json<ResolveBody>) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }
    if body.ids.is_empty() || body.ids.len() > 1000 {
        return HttpResponse::BadRequest().json(ApiError::new(400, "ids: 1–1000 items"));
    }
    match state.db.resolve_questions(&body.ids).await {
        Ok(n)  => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "resolved": n }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiError::new(500, format!("{e}"))),
    }
}


// ══════════════════════════════════════════════════════════════════
//  ADMIN: INGEST DOCUMENT FROM URL
// ══════════════════════════════════════════════════════════════════

/// Scrape URL, bagi jadi chunks, embed via Gemini, simpan ke DB
// ─── Fetch HTML ────────────────────────────────────────────────────────────────
/// Download a page's HTML. JavaScript is not run (no headless browser: it does not fit in
/// the free Render instance's memory). Tables whose rows are filled in by JavaScript are
/// detected in `extract_page` and reported as a warning in the draft review instead.
async fn fetch_html(url: &str) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = http.get(url).send().await
        .map_err(|e| format!("Gagal fetch URL: {e}"))?;
    // Refuse error pages: otherwise a "404 File not found" page is ingested as a document.
    if !resp.status().is_success() {
        return Err(format!("URL returned HTTP {} — page not ingested", resp.status().as_u16()));
    }
    resp.text().await.map_err(|e| format!("Gagal baca konten: {e}"))
}

/// Where the main content usually lives, most specific first. Tried in this order.
const CONTENT_SELECTORS: &[&str] = &[
    "main#quarto-document-content", // Quarto sites (the PMPSTI guidebook)
    "article .post-content",        // DTETI WordPress theme
    ".entry-content",               // most other WordPress themes
    "main",
    "article",
    "#content",
    ".content",
    "body",
];

/// Turn downloaded HTML into title, main text and tables (stages 2–3 of URL ingest).
/// Pure function — no network — so it can be tested on saved pages in tests/fixtures/.
fn extract_page(html: &str, url: &str, title_override: Option<String>) -> crate::reader::PageContent {
    use crate::reader;
    let document = scraper::Html::parse_document(html);

    // Title: the one given by the admin, else <title>, else the URL
    let page_title = scraper::Selector::parse("title").ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|e| e.text().collect::<String>().replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let title = title_override
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| if page_title.is_empty() { url.to_string() } else { page_title });

    // Take the first content area that exists, in OUR priority order. A single
    // "main, article, …, body" selector would return the first match in page order,
    // and <body> always comes first — so menus, sidebars and footers leaked in.
    let root = CONTENT_SELECTORS.iter()
        .filter_map(|css| scraper::Selector::parse(css).ok())
        .find_map(|sel| document.select(&sel).next())
        .unwrap_or_else(|| document.root_element());

    // Tables: JSON widgets anywhere on the page (they hold every row) + HTML tables in the content
    let mut tables = reader::json_tables(&document);
    // Rows loaded by JavaScript are not in the HTML; JSON widgets above already hold every row
    let js_tables = tables.is_empty() && reader::js_filled_tables(&document);
    tables.extend(reader::html_tables(root));

    // The main text is the sections' text, so the length check and the preview warnings
    // look at exactly what gets chunked.
    let sections = reader::sections(root);
    let text = sections.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
    reader::PageContent { title, text, sections, tables, js_tables }
}

/// Fetch a URL and turn it into a title and plain text (stages 1–3 of URL ingest).
/// Downloading happens here; the parsing lives in `extract_page`.
/// Shared by `admin_ingest_url` and `admin_preview_url`, so a preview shows exactly
/// what would be saved. Errors are returned as ready-made HTTP responses.
async fn read_url(url: &str, title_override: Option<String>) -> Result<crate::reader::PageContent, HttpResponse> {
    let url = url.trim().to_string();
    if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
        return Err(HttpResponse::BadRequest().json(ApiError::new(400, "URL tidak valid")));
    }

    // 1. Fetch HTML (JavaScript is not run; see fetch_html)
    let html = match fetch_html(&url).await {
        Ok(h) => h,
        Err(e) => return Err(HttpResponse::BadGateway().json(ApiError::new(502, e))),
    };

    let page = extract_page(&html, &url, title_override);

    if page.text.len() < 50 && page.tables.is_empty() {
        return Err(HttpResponse::UnprocessableEntity()
            .json(ApiError::new(422, "Konten terlalu pendek atau tidak bisa di-parse")));
    }

    Ok(page)
}

/// All chunks for a page: each section's text, with acronyms expanded, chunked with the
/// requested strategy and prefixed with its heading path, then every table chunked by
/// `reader::table_chunks` (summary + rows with column names).
async fn page_chunks(
    page:       &crate::reader::PageContent,
    strategy:   &crate::models::ChunkStrategy,
    http:       &reqwest::Client,
    gemini_key: &str,
) -> Result<(crate::models::ChunkStrategy, Vec<String>), HttpResponse> {
    let mut used = strategy.clone();
    let mut chunks = Vec::new();
    if page.text.len() >= 50 {
        for (i, section) in page.sections.iter().enumerate() {
            // Expand acronyms per section (first 2 uses each), so every section explains its own
            let text = crate::enrich::expand_acronyms(&section.text, 2);
            let (section_used, section_chunks) = chunk_text(&text, strategy, http, gemini_key).await?;
            if i == 0 {
                used = section_used; // reported in the preview; `auto` may differ per section
            }
            chunks.extend(with_heading_path(&section.path, section_chunks));
        }
    }
    for table in &page.tables {
        chunks.extend(crate::reader::table_chunks(&page.title, table, chunk_max_chars()));
    }
    Ok((used, chunks))
}

/// Put the heading path (e.g. "1 Pendahuluan > 1.1 Identitas Prodi") as the first line of
/// every chunk of a section, so search, embedding and the LLM all see where the text is from.
fn with_heading_path(path: &[String], chunks: Vec<String>) -> Vec<String> {
    if path.is_empty() {
        return chunks;
    }
    let heading = path.join(" > ");
    chunks.into_iter().map(|c| format!("{heading}\n{c}")).collect()
}

/// Split text into chunks (stage 4), resolving `auto` to a concrete strategy first.
/// Returns the strategy actually used. Only `semantic` calls Gemini (one call per sentence).
async fn chunk_text(
    raw_text:   &str,
    strategy:   &crate::models::ChunkStrategy,
    http:       &reqwest::Client,
    gemini_key: &str,
) -> Result<(crate::models::ChunkStrategy, Vec<String>), HttpResponse> {
    // Strategy from the request (default: auto-detect)
    let min_chars = chunk_min_chars();
    let max_chars = chunk_max_chars();
    // Resolve Auto → detect from the content
    let resolved_strategy = if *strategy == crate::models::ChunkStrategy::Auto {
        crate::models::ChunkStrategy::detect(raw_text)
    } else {
        strategy.clone()
    };

    let chunks_text: Vec<String> = match &resolved_strategy {
        crate::models::ChunkStrategy::Semantic | crate::models::ChunkStrategy::Auto => {
            match semantic_chunk(&http, EMBED_URL, &gemini_key, raw_text,
                chunk_similarity_threshold(), min_chars, max_chars).await {
                Ok(c)  => c,
                Err(e) => return Err(HttpResponse::BadGateway()
                    .json(ApiError::new(502, format!("Semantic chunk error: {e}")))),
            }
        }
        crate::models::ChunkStrategy::Sentence   => sentence_chunk(raw_text, min_chars, max_chars),
        crate::models::ChunkStrategy::Structural => structural_chunk(raw_text, min_chars, max_chars),
        crate::models::ChunkStrategy::Fixed      => fixed_chunk(raw_text, max_chars),
    };

    Ok((resolved_strategy, chunks_text))
}

/// One web page read and chunked for a source job (src/web_jobs.rs): the same reader,
/// sections, acronyms and table chunks as Add from URL, with the `auto` strategy (which
/// resolves to sentence/structural — no Gemini calls while chunking).
pub(crate) struct WebPageChunks {
    pub title:    String,
    pub chunks:   Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) async fn read_and_chunk_page(url: &str) -> Result<WebPageChunks, String> {
    let html = fetch_html(url).await?;
    let page = extract_page(&html, url, None);
    if page.text.len() < 50 && page.tables.is_empty() {
        return Err("Konten terlalu pendek atau tidak bisa di-parse".into());
    }
    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let (_, chunks) = page_chunks(&page, &crate::models::ChunkStrategy::Auto, &http, "").await
        .map_err(|_| "Chunking gagal".to_string())?;
    let warnings = crate::reader::quality_warnings(&page, &chunks, chunk_max_chars());
    Ok(WebPageChunks { title: page.title, chunks, warnings })
}

/// Embeddings for a page's chunks with the shared recipe (title + content, RETRIEVAL_DOCUMENT).
/// Failed batches come back as empty vectors (stored as NULL, still found by keyword search).
pub(crate) async fn embed_page_chunks(title: &str, chunks: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    if key.is_empty() {
        return Err("GEMINI_API_KEY tidak dikonfigurasi".into());
    }
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(embed_documents(&http, &key, title, chunks).await)
}

/// Preview a URL ingest: read and chunk the page exactly like `admin_ingest_url`,
/// but embed and save nothing. Lets an admin (or a developer) check the extraction first.
pub async fn admin_preview_url(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<IngestUrlRequest>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }

    let page = match read_url(&body.url, body.title.clone()).await {
        Ok(page) => page,
        Err(resp) => return resp,
    };
    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    // Only the semantic strategy needs Gemini here; the others work without a key.
    let gemini_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    let (strategy, chunks) = match page_chunks(&page, &body.chunk_strategy, &http, &gemini_key).await {
        Ok(result) => result,
        Err(resp) => return resp,
    };

    HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
        "url":         body.url.trim(),
        "title":       page.title,
        "strategy":    format!("{strategy:?}").to_lowercase(),
        "text_chars":  page.text.chars().count(),
        "tables":      page.tables.iter().map(|t| serde_json::json!({
            "caption": t.caption, "headers": t.headers, "rows": t.rows.len(),
        })).collect::<Vec<_>>(),
        "sections":    page.sections.len(),
        "warnings":    crate::reader::quality_warnings(&page, &chunks, chunk_max_chars()),
        "chunk_count": chunks.len(),
        "chunks":      chunks.iter().enumerate().map(|(i, c)| serde_json::json!({
            "index": i, "chars": c.chars().count(), "text": c,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn admin_ingest_url(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<IngestUrlRequest>,
) -> HttpResponse {
    // Auth check
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }

    let page = match read_url(&body.url, body.title.clone()).await {
        Ok(page) => page,
        Err(resp) => return resp,
    };
    let title = page.title.clone();
    let url = body.url.trim().to_string();

    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    // 3. Gemini key (always needed to embed chunks, even when chunking isn't semantic)
    let gemini_key = match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "GEMINI_API_KEY tidak dikonfigurasi")),
    };

    let (_, chunks_text) = match page_chunks(&page, &body.chunk_strategy, &http, &gemini_key).await {
        Ok(result) => result,
        Err(resp) => return resp,
    };

    let category    = body.category.clone().unwrap_or_else(|| "Umum".to_string());
    let subcategory = body.subcategory.clone().unwrap_or_else(|| "—".to_string());
    let document_id = format!("url-{}", uuid::Uuid::new_v4());

    // 4. Embed all chunks (batched; failed batches are saved without embeddings)
    let embeddings = embed_documents(&http, &gemini_key, &title, &chunks_text).await;
    let mut doc_chunks: Vec<DocumentChunk> = Vec::new();

    for (i, (chunk_text, embedding)) in chunks_text.iter().zip(embeddings).enumerate() {
        doc_chunks.push(DocumentChunk {
            document_id:   document_id.clone(),
            title:         title.clone(),
            content:       chunk_text.clone(),
            source_url:    url.clone(),
            category:      category.clone(),
            subcategory:   subcategory.clone(),
            document_type: "url".to_string(),
            chunk_index:   i as i32,
            embedding,
        });
    }

    // 5. Simpan ke DB
    match state.db.insert_document_chunks(&doc_chunks).await {
        Ok(n) => HttpResponse::Ok().json(ApiSuccess::new(IngestResponse {
            document_id,
            chunks: n,
            title,
        })),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiError::new(500, format!("DB error: {e}"))),
    }
}

/// Ingest plain text content directly (no URL fetching).
pub async fn admin_ingest_text(
    req:   HttpRequest,
    state: web::Data<AppState>,
    body:  web::Json<crate::models::IngestTextRequest>,
) -> HttpResponse {
    let claims = match require_auth(&req, &state.jwt_secret) { Ok(c) => c, Err(r) => return r };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin"));
    }

    let raw_text = body.content.trim().to_string();
    if raw_text.len() < 10 {
        return HttpResponse::BadRequest().json(ApiError::new(400, "Content terlalu pendek"));
    }

    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; PMPSTI-Bot/1.0)")
        .timeout(std::time::Duration::from_secs(30))
        .build().unwrap();

    // Gemini key — selalu diperlukan untuk embed chunk
    let gemini_key = match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return HttpResponse::InternalServerError()
            .json(ApiError::new(500, "GEMINI_API_KEY tidak dikonfigurasi")),
    };

    let (_, chunks_text) = match chunk_text(&raw_text, &body.chunk_strategy, &http, &gemini_key).await {
        Ok(result) => result,
        Err(resp) => return resp,
    };

    let title      = body.title.clone();
    let source_url = body.source_url.clone().unwrap_or_else(|| "manual".to_string());
    let category   = body.category.clone().unwrap_or_else(|| "Umum".to_string());
    let subcategory= body.subcategory.clone().unwrap_or_else(|| "—".to_string());
    let document_id= format!("text-{}", uuid::Uuid::new_v4());

    // Embed all chunks (batched, same recipe as URL ingest)
    let embeddings = embed_documents(&http, &gemini_key, &title, &chunks_text).await;
    let mut doc_chunks: Vec<DocumentChunk> = Vec::new();

    for (i, (chunk_text, embedding)) in chunks_text.iter().zip(embeddings).enumerate() {
        doc_chunks.push(DocumentChunk {
            document_id:   document_id.clone(),
            title:         title.clone(),
            content:       chunk_text.clone(),
            source_url:    source_url.clone(),
            category:      category.clone(),
            subcategory:   subcategory.clone(),
            document_type: "text".to_string(),
            chunk_index:   i as i32,
            embedding,
        });
    }

    match state.db.insert_document_chunks(&doc_chunks).await {
        Ok(n) => HttpResponse::Ok().json(ApiSuccess::new(IngestResponse {
            document_id,
            chunks: n,
            title,
        })),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiError::new(500, format!("DB error: {e}"))),
    }
}


#[cfg(test)]
mod tests {
    use super::{cookie_attributes, extract_page, with_heading_path};

    #[test]
    fn cookie_is_secure_except_on_local_http() {
        assert_eq!(cookie_attributes("https", "pmpsti-rag.onrender.com"), "Secure; SameSite=None");
        assert_eq!(cookie_attributes("http", "pmpsti-rag.onrender.com"), "Secure; SameSite=None");
        assert_eq!(cookie_attributes("http", "localhost:5173"), "SameSite=Lax");
        assert_eq!(cookie_attributes("http", "127.0.0.1:8080"), "SameSite=Lax");
        assert_eq!(cookie_attributes("https", "localhost:5173"), "Secure; SameSite=None");
    }

    // Real pages saved on 2026-10-18, so parsing can be tested without the network.
    const TENAGA_PENDIDIK: &str = include_str!("../tests/fixtures/tenaga_pendidik.html");
    const D_TENDIK: &str = include_str!("../tests/fixtures/d_tendik.html");
    const INTRODUCTION: &str = include_str!("../tests/fixtures/introduction.html");

    #[test]
    fn wordpress_page_reads_article_not_sidebar() {
        let page = extract_page(TENAGA_PENDIDIK, "https://jteti.ugm.ac.id/tenaga-pendidik/", None);
        let text = format!("{}\n{:?}", page.text, page.tables);
        // Staff list (inside <article>) is kept…
        assert!(text.contains("Insap Santosa"));
        assert!(text.contains("Nailil Husna"));
        // …the news sidebar (outside <article>) is not.
        assert!(!text.contains("Taekwondo"), "sidebar news leaked into the text");
        assert!(!text.contains("Tumiran, Pakar Energi"), "sidebar news leaked into the text");
    }

    #[test]
    fn quarto_page_uses_page_title() {
        let page = extract_page(D_TENDIK, "https://mtiugm.github.io/panduan_pmpsti/appendix/d_tendik.html", None);
        assert!(page.title.starts_with("Lampiran D"), "unexpected title: {}", page.title);
        // &nbsp; in <title> becomes a normal space ("1&nbsp; Pendahuluan" was stored with U+00A0)
        let intro = extract_page(INTRODUCTION, "https://example.org/", None);
        assert!(intro.title.starts_with("1 Pendahuluan"), "unexpected title: {:?}", intro.title);
        assert!(page.text.contains("Tabel D.1"));
        assert_eq!(page.tables.len(), 1);
        assert_eq!(page.tables[0].rows.len(), 40);
    }

    /// Chunks of a saved page as URL ingest makes them, without the network: sections →
    /// acronyms → auto strategy (never semantic) → heading path, then table chunks.
    fn offline_chunks(page: &crate::reader::PageContent) -> Vec<String> {
        use crate::models::ChunkStrategy;
        let (min, max) = (super::chunk_min_chars(), super::chunk_max_chars());
        let mut chunks = Vec::new();
        for section in &page.sections {
            let text = crate::enrich::expand_acronyms(&section.text, 2);
            let parts = match ChunkStrategy::detect(&text) {
                ChunkStrategy::Structural => super::structural_chunk(&text, min, max),
                _ => super::sentence_chunk(&text, min, max),
            };
            chunks.extend(with_heading_path(&section.path, parts));
        }
        for table in &page.tables {
            chunks.extend(crate::reader::table_chunks(&page.title, table, max));
        }
        chunks
    }

    /// Prints the first chunks and the preview warnings of each saved page. Run with:
    /// `cargo test show_guidebook_chunks -- --ignored --nocapture`
    #[test]
    #[ignore = "prints example output; run explicitly"]
    fn show_guidebook_chunks() {
        for (name, html, url) in [
            ("introduction", INTRODUCTION, "https://mtiugm.github.io/panduan_pmpsti/chapter1/introduction.html"),
            ("d_tendik", D_TENDIK, "https://mtiugm.github.io/panduan_pmpsti/appendix/d_tendik.html"),
            ("c_ewmp", include_str!("../tests/fixtures/c_ewmp.html"), "https://mtiugm.github.io/panduan_pmpsti/appendix/c_ewmp.html"),
            ("tenaga_pendidik", TENAGA_PENDIDIK, "https://jteti.ugm.ac.id/tenaga-pendidik/"),
        ] {
            let page = extract_page(html, url, None);
            let chunks = offline_chunks(&page);
            println!("\n================ {name}: {} sections, {} chunks ================", page.sections.len(), chunks.len());
            for (i, c) in chunks.iter().take(3).enumerate() {
                println!("--- chunk {} ({} chars)\n{}\n", i + 1, c.len(), c.chars().take(300).collect::<String>());
            }
            for c in chunks.iter().filter(|c| c.chars().count() < 80 || c.chars().count() > 1500) {
                println!("  flagged ({} chars): {}", c.chars().count(), c.chars().take(160).collect::<String>().replace('\n', " / "));
            }
            println!("warnings: {:?}", crate::reader::quality_warnings(&page, &chunks, super::chunk_max_chars()));
        }
    }

    #[test]
    fn saved_pages_have_no_quality_warnings() {
        // The jteti staff page (WordPress) has its rows in the HTML: no JavaScript needed
        assert!(!extract_page(TENAGA_PENDIDIK, "https://jteti.ugm.ac.id/tenaga-pendidik/", None).js_tables);
        for (name, html) in [("introduction", INTRODUCTION), ("d_tendik", D_TENDIK)] {
            let page = extract_page(html, "https://example.org/", None);
            assert!(!page.js_tables, "{name}");
            let warnings = crate::reader::quality_warnings(&page, &offline_chunks(&page), super::chunk_max_chars());
            assert!(warnings.is_empty(), "{name}: {warnings:?}");
        }
    }

    #[test]
    fn embed_text_is_title_blank_line_content() {
        // Must match document_text() in scripts/gemini_embed.py
        assert_eq!(super::document_embed_text("Beban Dosen", "Nama: X"), "Beban Dosen\n\nNama: X");
        assert_eq!(super::document_embed_text("", "Nama: X"), "Nama: X");
    }

    #[test]
    fn chunks_start_with_heading_path() {
        let path = vec!["1 Pendahuluan".to_string(), "1.1 Identitas Prodi".to_string()];
        let chunks = with_heading_path(&path, vec!["Nama ...".into(), "Visi ...".into()]);
        assert_eq!(chunks, ["1 Pendahuluan > 1.1 Identitas Prodi\nNama ...", "1 Pendahuluan > 1.1 Identitas Prodi\nVisi ..."]);
        assert_eq!(with_heading_path(&[], vec!["text".into()]), ["text"]);
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::answer_outcome;

    #[test]
    fn classifies_answers() {
        assert_eq!(answer_outcome("Maaf, informasi tersebut tidak tersedia dalam dokumen yang diindeks.", 3), "not_found");
        assert_eq!(answer_outcome("Halo! Saya asisten akademik PMPSTI.", 0), "no_sources");
        assert_eq!(answer_outcome("Tesis bernilai **6 SKS** [1].", 2), "answered");
        assert_eq!(answer_outcome("Tidak ada informasi mengenai beasiswa LPDP.", 0), "not_found");
    }
}

#[cfg(test)]
mod citation_tests {
    use super::cited_sources;

    #[test]
    fn keeps_only_cited_sources_renumbered() {
        let (text, keep) = cited_sources("Tesis 6 SKS [3]. Total 36 SKS [1][3].", 3);
        assert_eq!(text, "Tesis 6 SKS [1]. Total 36 SKS [2][1].");
        assert_eq!(keep, vec![2, 0]);
    }

    #[test]
    fn lists_ranges_and_bad_numbers() {
        let (text, keep) = cited_sources("A [1, 2]. B [2-3]. C [9].", 3);
        assert_eq!(text, "A [1][2]. B [2][3]. C.");
        assert_eq!(keep, vec![0, 1, 2]);
    }

    #[test]
    fn no_citation_no_sources() {
        let (text, keep) = cited_sources("Maaf, informasi tersebut tidak tersedia dalam dokumen.", 3);
        assert_eq!(text, "Maaf, informasi tersebut tidak tersedia dalam dokumen.");
        assert!(keep.is_empty());
    }

    #[test]
    fn streaming_pieces_give_the_same_text() {
        let full = "Tesis 6 SKS [3]. Total [1, 3] dan [panduan](https://x.id) [2](https://y.id). Lain [9].";
        let (whole, order) = super::cited_sources(full, 3);
        for size in [1, 2, 3, 5, 7] {
            let mut c = super::CitationRenumber::new(3);
            let chars: Vec<char> = full.chars().collect();
            let mut out = String::new();
            for piece in chars.chunks(size) {
                out.push_str(&c.push(&piece.iter().collect::<String>()));
            }
            out.push_str(&c.finish());
            assert_eq!(out, whole, "piece size {size}");
            assert_eq!(c.order(), order.as_slice());
        }
    }

    #[test]
    fn leaves_other_brackets_alone() {
        let (text, keep) = cited_sources("Lihat [panduan](https://x.id) dan [2](https://y.id) [ ] [1a] [1].", 2);
        assert_eq!(text, "Lihat [panduan](https://x.id) dan [2](https://y.id) [ ] [1a] [1].");
        assert_eq!(keep, vec![0]);
    }
}
