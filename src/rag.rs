use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::db::SearchResult;
use crate::search::SearchEngine;

// ══════════════════════════════════════════════════════════════════
//  CONFIG
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum LlmProvider {
    Ollama,
    OpenAICompatible,
    Gemini,
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider:   LlmProvider,
    pub api_url:    String,
    pub api_key:    String,
    pub model:      String,
}

impl LlmConfig {
    pub fn ollama(model: &str) -> Self {
        Self {
            provider:   LlmProvider::Ollama,
            api_url:    "http://localhost:11434/api/chat".to_string(),
            api_key:    String::new(),
            model:      model.to_string(),
        }
    }

    pub fn openai_compatible(api_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            provider:   LlmProvider::OpenAICompatible,
            api_url:    api_url.to_string(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
        }
    }

    pub fn gemini(api_key: &str, model: &str) -> Self {
        Self {
            provider:   LlmProvider::Gemini,
            api_url:    String::new(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
        }
    }
}

// ══════════════════════════════════════════════════════════════════
//  MODEL FALLBACK (free-tier daily quotas)
// ══════════════════════════════════════════════════════════════════

/// Models that hit their quota (429) are skipped until this time.
fn cooldowns() -> &'static Mutex<HashMap<String, Instant>> {
    static C: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Primary model followed by the fallback models from Admin → Settings (default: env
/// LLM_FALLBACK_MODELS, or gemini-3.1-flash-lite for Gemini). Models still cooling down after a
/// 429 go last.
pub fn model_chain(primary: &str) -> Vec<String> {
    let fallbacks = crate::settings::get().fallback_models.clone();
    chain_from(primary, &fallbacks, &cooldowns().lock().map(|c| c.clone()).unwrap_or_default(), Instant::now())
}

fn chain_from(primary: &str, fallbacks: &str, cooling: &HashMap<String, Instant>, now: Instant) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for m in std::iter::once(primary).chain(fallbacks.split(',')).map(str::trim).filter(|m| !m.is_empty()) {
        if !all.iter().any(|x| x == m) { all.push(m.to_string()); }
    }
    let (ready, waiting): (Vec<String>, Vec<String>) =
        all.into_iter().partition(|m| cooling.get(m).map_or(true, |until| *until <= now));
    ready.into_iter().chain(waiting).collect()
}

/// After a 429 the model is skipped for a while: an hour when the daily quota is gone,
/// one minute for a per-minute limit.
fn mark_exhausted(model: &str, body: &str) {
    let daily = body.contains("PerDay") || body.to_lowercase().contains("per day");
    let wait = if daily { Duration::from_secs(3600) } else { Duration::from_secs(60) };
    if let Ok(mut c) = cooldowns().lock() { c.insert(model.to_string(), Instant::now() + wait); }
    log::warn!("[llm] {model} rate-limited ({}) — using the next model", if daily { "daily quota" } else { "per minute" });
}

/// Errors worth trying the next model for: quota, overload, or a model name that does not exist.
fn try_next_model(status: u16) -> bool {
    matches!(status, 429 | 404 | 500 | 503)
}

// ══════════════════════════════════════════════════════════════════
//  OPENAI-COMPATIBLE TYPES
// ══════════════════════════════════════════════════════════════════

#[derive(Serialize)]
struct ChatRequest {
    model:       String,
    messages:    Vec<Message>,
    max_tokens:  u32,
    temperature: f32,
    stream:      bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    pub role:    String,
    pub content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

// ── Ollama ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaChatRequest {
    model:    String,
    messages: Vec<Message>,
    stream:   bool,
    options:  OllamaOptions,
}

#[derive(Serialize)]
struct OllamaOptions {
    num_predict: u32,
    temperature: f32,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Message,
}

// ── Gemini ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct GeminiRequest {
    system_instruction: GeminiSystemInstruction,
    contents:           Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config:  GeminiGenerationConfig,
}

#[derive(Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiContent {
    role:  String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
    temperature:       f32,
}

// ══════════════════════════════════════════════════════════════════
//  QUERY TYPE
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum QueryType {
    List,        // sebutkan semua, daftar, berapa jumlah
    Procedural,  // bagaimana cara, langkah, syarat, prosedur
    Comparison,  // bandingkan, perbedaan, vs
    Definition,  // apa itu, jelaskan, pengertian
    General,     // lainnya
}

// ══════════════════════════════════════════════════════════════════
//  RAG ENGINE
// ══════════════════════════════════════════════════════════════════

pub struct RagEngine {
    pub search: SearchEngine,
    llm:        LlmConfig,
    http:       Client,
}

impl RagEngine {
    pub fn new(search: SearchEngine, llm: LlmConfig) -> Self {
        // Answers stream for a while, so no total limit; each chunk must arrive within 60 s
        Self { search, llm, http: crate::limits::http_client(std::time::Duration::from_secs(10), std::time::Duration::from_secs(60), None) }
    }

    /// Answer model: Admin → Settings, else the one from the environment.
    fn model(&self) -> String {
        let m = crate::settings::get().model.clone();
        if m.is_empty() { self.llm.model.clone() } else { m }
    }

    /// Provider name and whether its API key is set (last 4 characters only) — for Admin → Settings.
    pub fn provider_info(&self) -> (String, String, Option<String>) {
        let name = match self.llm.provider {
            LlmProvider::Gemini           => "gemini",
            LlmProvider::OpenAICompatible => "openai-compatible",
            LlmProvider::Ollama           => "ollama",
        };
        let key = self.llm.api_key.trim();
        let tail = (key.len() >= 8).then(|| key[key.len() - 4..].to_string());
        (name.to_string(), self.llm.model.clone(), tail)
    }

    /// A single instruction + text → reply, with the answer model and its fallbacks
    /// (used for admin tools such as draft test questions, not for student answers).
    pub async fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let messages = vec![
            Message { role: "system".into(), content: system.to_string() },
            Message { role: "user".into(), content: user.to_string() },
        ];
        let models = model_chain(&self.model());
        match self.llm.provider {
            // No truncation note here: admin tools (e.g. draft checks) parse the reply
            LlmProvider::Gemini           => self.call_gemini_models(&models, system, &messages, max_tokens).await.map(|t| t.text),
            LlmProvider::OpenAICompatible => self.call_openai_models(&models, &messages, max_tokens).await,
            LlmProvider::Ollama           => self.call_ollama_model(&self.model(), &messages).await,
        }
    }

    /// One tiny request to `model` (no fallback) to check the key and the model name.
    /// Costs one generation call.
    pub async fn test_model(&self, model: &str) -> Result<String> {
        let messages = vec![
            Message { role: "system".into(), content: "Balas hanya dengan kata: OK".into() },
            Message { role: "user".into(), content: "Tes koneksi".into() },
        ];
        match self.llm.provider {
            LlmProvider::Gemini           => self.call_gemini_models(&[model.to_string()], "Balas hanya dengan kata: OK", &messages, 16).await.map(|t| t.text),
            LlmProvider::OpenAICompatible => self.call_openai_models(&[model.to_string()], &messages, 16).await,
            LlmProvider::Ollama           => self.call_ollama_model(model, &messages).await,
        }
    }

    /// Klasifikasi tipe query via Gemini (single-token response).
    /// Fallback ke pattern-based jika LLM tidak tersedia atau gagal.
    /// Dipanggil dengan `&self` karena butuh http client dan API key.
    pub async fn classify_query(&self, query: &str) -> QueryType {
        // Coba LLM classification dulu
        if let Some(qt) = self.classify_query_llm(query).await {
            return qt;
        }
        // Fallback ke pattern-based
        Self::classify_query_patterns(query)
    }

    /// LLM-based classification via Gemini Flash (hemat, cepat).
    /// Mengembalikan None jika API key tidak ada atau request gagal.
    async fn classify_query_llm(&self, query: &str) -> Option<QueryType> {
        let api_key = std::env::var("GEMINI_API_KEY").ok()?;

        // Classifier model: GEMINI_CLASSIFY_MODEL > the answer model (when it is a Gemini model)
        // > GEMINI_MODEL > default flash-lite
        let answer_model = crate::settings::get().model.clone();
        let model = std::env::var("GEMINI_CLASSIFY_MODEL").ok()
            .or_else(|| answer_model.starts_with("gemini").then_some(answer_model))
            .or_else(|| std::env::var("GEMINI_MODEL").ok())
            .unwrap_or_else(|| "gemini-3.5-flash-lite".to_string());

        let prompt = format!(
            "Klasifikasikan query berikut ke salah satu kategori. \
             Jawab HANYA dengan satu kata dari pilihan ini: LIST, PROCEDURAL, COMPARISON, DEFINITION, GENERAL\n\n\
             Panduan:\n\
             - LIST: meminta daftar/enumerasi (semua mitra, daftar dosen, berapa jumlah, siapa saja)\n\
             - PROCEDURAL: meminta langkah/cara/prosedur (cara daftar, syarat masuk, alur pendaftaran)\n\
             - COMPARISON: meminta perbandingan (perbedaan antara, vs, mana yang lebih baik)\n\
             - DEFINITION: meminta penjelasan/definisi (apa itu, jelaskan, pengertian, apakah)\n\
             - GENERAL: pertanyaan faktual spesifik atau lainnya\n\n\
             Query: \"{}\"\n\nJawab:",
            query
        );

        #[derive(serde::Serialize)]
        struct GeminiReq { contents: Vec<GContent> }
        #[derive(serde::Serialize)]
        struct GContent { parts: Vec<GPart> }
        #[derive(serde::Serialize)]
        struct GPart { text: String }
        #[derive(serde::Deserialize)]
        struct GeminiResp { candidates: Vec<GCandidate> }
        #[derive(serde::Deserialize)]
        struct GCandidate { content: GContentResp }
        #[derive(serde::Deserialize)]
        struct GContentResp { parts: Vec<GPartResp> }
        #[derive(serde::Deserialize)]
        struct GPartResp { text: String }

        let body = GeminiReq {
            contents: vec![GContent { parts: vec![GPart { text: prompt }] }],
        };

        let mut data: Option<GeminiResp> = None;
        for model in model_chain(&model) {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
            let resp = self.http.post(&url)
                .header("x-goog-api-key", &api_key)
                .json(&body)
                .send().await.ok()?;
            let status = resp.status().as_u16();
            if resp.status().is_success() {
                data = resp.json().await.ok();
                break;
            }
            let text = resp.text().await.unwrap_or_default();
            if status == 429 { mark_exhausted(&model, &text); }
            if !try_next_model(status) { return None; }
        }
        let data = data?;
        let raw = data.candidates.first()?
            .content.parts.first()?
            .text.trim().to_uppercase();

        // Parse single-word response
        let qt = match raw.as_str() {
            "LIST"        => QueryType::List,
            "PROCEDURAL"  => QueryType::Procedural,
            "COMPARISON"  => QueryType::Comparison,
            "DEFINITION"  => QueryType::Definition,
            "GENERAL"     => QueryType::General,
            // LLM kadang balas dengan kata lebih → cek prefix
            _ if raw.starts_with("LIST")       => QueryType::List,
            _ if raw.starts_with("PROCEDURAL") => QueryType::Procedural,
            _ if raw.starts_with("COMPARISON") => QueryType::Comparison,
            _ if raw.starts_with("DEFINITION") => QueryType::Definition,
            _                                  => QueryType::General,
        };

        Some(qt)
    }

    /// Pattern-based classification — fallback jika LLM tidak tersedia.
    pub fn classify_query_patterns(query: &str) -> QueryType {
        let q = query.to_lowercase();

        let list_patterns = [
            "sebutkan", "semua", "daftar", "list", "berapa jumlah",
            "berapa banyak", "seluruh", "lengkap", "mitra", "semua mitra",
            "apa saja", "siapa saja", "jumlah", "total", "kerjasama",
        ];
        if list_patterns.iter().any(|p| q.contains(p)) { return QueryType::List; }

        let procedural_patterns = [
            "bagaimana cara", "langkah", "prosedur", "syarat", "persyaratan",
            "cara mendaftar", "cara", "tahapan", "alur", "mekanisme",
            "tata cara", "ketentuan", "aturan", "bagaimana proses",
        ];
        if procedural_patterns.iter().any(|p| q.contains(p)) { return QueryType::Procedural; }

        let comparison_patterns = [
            "bandingkan", "perbedaan", "persamaan", "vs", "versus",
            "lebih baik", "mana yang", "dibanding", "beda antara",
        ];
        if comparison_patterns.iter().any(|p| q.contains(p)) { return QueryType::Comparison; }

        let definition_patterns = [
            "apa itu", "definisi", "pengertian", "jelaskan", "apakah",
            "maksud dari", "arti", "apa yang dimaksud",
        ];
        if definition_patterns.iter().any(|p| q.contains(p)) { return QueryType::Definition; }

        let person_patterns = [
            "prof", "profesor", "guru besar", "jabatan", "gelar",
            "siapa", "dosen", "pengajar", "lektor", "tenaga pendidik",
        ];
        if person_patterns.iter().any(|p| q.contains(p)) { return QueryType::List; }

        QueryType::General
    }

    /// Search chunks — ranking only, no expansion.
    /// hybrid (default) / rrf / semantic / keyword depending on search_mode.
    /// Context is expanded AFTER filtering, in `build_context`.
    pub async fn search_chunks(
        &self,
        query:           &str,
        category_filter: Option<&str>,
        search_mode:     &str,
        top_k:           i64,
    ) -> Result<(Vec<SearchResult>, Option<Vec<f32>>)> {
        Ok(match search_mode {
            "semantic" => (self.search.search_semantic(query, top_k, category_filter).await?, None),
            "keyword"  => (self.search.search_keyword(query, top_k, category_filter).await?, None),
            "rrf"      => (self.search.search_rrf(query, top_k, category_filter).await?, None),
            _          => self.search.search_with_embedding(query, top_k, category_filter).await?, // hybrid
        })
    }

    /// Merge the selected chunks into ONE context per document (= one source card;
    /// [n] in the answer = the n-th card), then add the surrounding context:
    /// - List / Procedural: other chunks of the same document (original order), within budget
    /// - otherwise        : the ±1 neighbours of every matched chunk
    /// Matched chunks always come first — expansion can never push them out.
    pub async fn build_context(&self, query_type: &QueryType, hits: &[&SearchResult]) -> Vec<SearchResult> {
        if hits.is_empty() {
            return vec![];
        }
        let expand_whole_doc = matches!(query_type, QueryType::List | QueryType::Procedural);
        let budget_chars: usize = if expand_whole_doc { 14_000 } else { 8_000 };

        // Group per document, ordered by the rank of its best chunk
        let mut groups: Vec<(&SearchResult, Vec<i32>)> = Vec::new();
        for hit in hits {
            let idx = hit.chunk_index.unwrap_or(0);
            match groups.iter_mut().find(|(lead, _)| lead.document_id == hit.document_id) {
                Some((_, idxs)) => idxs.push(idx),
                None => groups.push((hit, vec![idx])),
            }
        }

        let doc_ids: Vec<String> = groups.iter().map(|(lead, _)| lead.document_id.clone()).collect();
        let all_chunks = self.search.db.fetch_all_chunks_for_documents(&doc_ids).await.unwrap_or_default();
        let versions = self.search.db.document_versions(&doc_ids).await.unwrap_or_default();
        let per_doc_budget = budget_chars / groups.len();

        groups.into_iter().map(|(lead, hit_idxs)| {
            let doc_chunks: Vec<&SearchResult> = all_chunks.iter()
                .filter(|c| c.document_id == lead.document_id)
                .collect();
            let content_of = |i: i32| doc_chunks.iter()
                .find(|c| c.chunk_index.unwrap_or(0) == i)
                .map(|c| c.content.as_str());

            // Priority: matched chunks → their neighbours → (List/Procedural) rest of the document
            let mut priority: Vec<i32> = hit_idxs.clone();
            for &i in &hit_idxs {
                priority.extend([i - 1, i + 1]);
            }
            if expand_whole_doc {
                priority.extend(doc_chunks.iter().map(|c| c.chunk_index.unwrap_or(0)));
            }

            let mut chosen: Vec<i32> = Vec::new();
            let mut used = 0usize;
            for i in priority {
                if chosen.contains(&i) {
                    continue;
                }
                let Some(text) = content_of(i) else { continue };
                // Matched chunks always go in; expansion only while the budget allows
                if !hit_idxs.contains(&i) && used + text.len() > per_doc_budget {
                    continue;
                }
                used += text.len();
                chosen.push(i);
            }
            chosen.sort_unstable();

            let mut content = String::new();
            for (n, &i) in chosen.iter().enumerate() {
                if n > 0 {
                    content.push_str(if i == chosen[n - 1] + 1 { "\n\n" } else { "\n\n[…]\n\n" });
                }
                content.push_str(content_of(i).unwrap_or_default());
            }
            if content.is_empty() {
                content = lead.content.clone(); // document not fetched: use the matched chunk itself
            }
            // Pages used (PDF sources) — shown on the source card and in the prompt for citations
            let mut pages: Vec<i32> = chosen.iter()
                .filter_map(|&i| doc_chunks.iter().find(|c| c.chunk_index.unwrap_or(0) == i))
                .filter_map(|c| c.page_number)
                .collect();
            if pages.is_empty() {
                pages.extend(lead.page_number);
            }
            pages.sort_unstable();
            pages.dedup();
            let version = versions.get(&lead.document_id).map(|v| v.describe()).unwrap_or_default();
            SearchResult { content, pages, version, ..lead.clone() }
        }).collect()
    }

    /// Panggil LLM dengan chunks yang sudah difilter oleh handler.
    /// chunks = apa yang akan ditampilkan sebagai kartu sumber → nomor [1]..[N] cocok.
    /// `query_type` comes from one `classify_query` call per question (also used by `build_context`).
    pub async fn answer_with_chunks(
        &self,
        query:      &str,
        history:    &[Message],
        chunks:     &[&SearchResult],
        query_type: &QueryType,
    ) -> Result<String> {
        if chunks.is_empty() {
            return Ok(NO_CONTEXT_ANSWER.to_string());
        }
        let (system_prompt, messages) = self.answer_messages(query, history, chunks, query_type);
        let answer = match self.llm.provider {
            LlmProvider::Ollama           => self.call_ollama(&messages).await?,
            LlmProvider::Gemini           => self.call_gemini(&system_prompt, &messages).await?,
            LlmProvider::OpenAICompatible => self.call_openai_compatible(&messages).await?,
        };
        Ok(answer)
    }

    /// Like `answer_with_chunks`, but sends the text to `tx` while it is generated
    /// (Gemini streams; the other providers send the whole answer at once). Returns the full answer.
    pub async fn answer_stream(
        &self,
        query:      &str,
        history:    &[Message],
        chunks:     &[&SearchResult],
        query_type: &QueryType,
        tx:         &tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<String> {
        if !chunks.is_empty() && matches!(self.llm.provider, LlmProvider::Gemini) {
            let (system_prompt, messages) = self.answer_messages(query, history, chunks, query_type);
            return self.stream_gemini(&system_prompt, &messages, tx).await;
        }
        let answer = self.answer_with_chunks(query, history, chunks, query_type).await?;
        let _ = tx.send(answer.clone());
        Ok(answer)
    }

    /// System prompt and messages (history window + context + question) for an answer.
    fn answer_messages(
        &self,
        query:      &str,
        history:    &[Message],
        chunks:     &[&SearchResult],
        query_type: &QueryType,
    ) -> (String, Vec<Message>) {
        // Format context dari references
        let context = SearchEngine::format_context_refs(chunks);
        let system_prompt = build_system_prompt();

        // Hint format jawaban berdasarkan tipe query
        let format_hint = match query_type {
            QueryType::List => "\n\nINSTRUKSI FORMAT: Query ini meminta daftar lengkap. \
                Tampilkan SEMUA entri dalam format tabel Markdown (| No | Nama | ... |) \
                atau numbered list. Jangan meringkas atau menghilangkan satu pun entri.",
            QueryType::Procedural => "\n\nINSTRUKSI FORMAT: Query ini meminta prosedur/langkah. \
                Tampilkan sebagai numbered list yang urut (1. 2. 3. dst). \
                Jika ada syarat/ketentuan, tampilkan dulu sebelum langkah.",
            QueryType::Comparison => "\n\nINSTRUKSI FORMAT: Query ini meminta perbandingan. \
                Gunakan tabel Markdown untuk membandingkan aspek-aspek kunci secara berdampingan. \
                Akhiri dengan ringkasan perbedaan utama.",
            QueryType::Definition => "\n\nINSTRUKSI FORMAT: Query ini meminta definisi/penjelasan. \
                Mulai dengan definisi singkat (1-2 kalimat), lalu elaborasi dengan poin-poin penting.",
            QueryType::General => "",
        };

        // Suggest klarifikasi jika hasil dari >= 3 kategori berbeda
        let categories: std::collections::HashSet<&str> = chunks.iter()
            .map(|c| c.category.as_str())
            .filter(|c| !c.is_empty())
            .collect();
        let clarification_hint = if categories.len() >= 3 {
            let cat_list: Vec<&str> = categories.into_iter().collect();
            format!(
                "\n\nCATATAN: Hasil mencakup {} kategori: {}. \
                 Jika pertanyaan ambigu, jawab sebaik mungkin LALU tawarkan klarifikasi.",
                cat_list.len(), cat_list.join(", ")
            )
        } else {
            String::new()
        };

        let combined_hint = format!("{}{}", format_hint, clarification_hint);
        let user_message = build_user_message(query, &context, combined_hint.trim());

        let mut messages: Vec<Message> = vec![
            Message { role: "system".to_string(), content: system_prompt.clone() },
        ];
        let history_window = if history.len() > 6 { &history[history.len() - 6..] } else { history };
        messages.extend_from_slice(history_window);
        messages.push(Message { role: "user".to_string(), content: user_message });

        (system_prompt, messages)
    }

    async fn call_ollama(&self, messages: &[Message]) -> Result<String> {
        self.call_ollama_model(&self.model(), messages).await
    }

    async fn call_ollama_model(&self, model: &str, messages: &[Message]) -> Result<String> {
        let req = OllamaChatRequest {
            model:    model.to_string(),
            messages: messages.to_vec(),
            stream:   false,
            options:  OllamaOptions {
                num_predict: crate::settings::get().max_tokens,
                temperature: crate::settings::get().temperature as f32,
            },
        };

        let resp = self.http.post(&self.llm.api_url).json(&req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body   = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Ollama error {}: {}", status, body));
        }

        let data: OllamaChatResponse = resp.json().await?;
        Ok(data.message.content)
    }

    async fn call_openai_compatible(&self, messages: &[Message]) -> Result<String> {
        let max_tokens = crate::settings::get().max_tokens;
        self.call_openai_models(&model_chain(&self.model()), messages, max_tokens).await
    }

    /// Try `models` in order (next one on quota / overload / unknown model).
    async fn call_openai_models(&self, models: &[String], messages: &[Message], max_tokens: u32) -> Result<String> {
        let temperature = crate::settings::get().temperature as f32;
        let mut last_err = anyhow::anyhow!("no LLM model configured");
        let mut ok = None;
        for model in models.iter().cloned() {
            let req = ChatRequest {
                model:       model.clone(),
                messages:    messages.to_vec(),
                max_tokens,
                temperature,
                stream:      false,
            };
            let mut builder = self.http.post(&self.llm.api_url).json(&req);
            if !self.llm.api_key.is_empty() {
                builder = builder.bearer_auth(&self.llm.api_key);
            }
            let resp = builder.send().await?;
            let status = resp.status();
            if status.is_success() {
                ok = Some(resp);
                break;
            }
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 429 { mark_exhausted(&model, &body); }
            last_err = anyhow::anyhow!("LLM API error {} ({}): {}", status, model, body);
            if !try_next_model(status.as_u16()) { break; }
        }
        let Some(resp) = ok else { return Err(last_err) };

        let data: ChatResponse = resp.json().await?;
        data.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("LLM returned empty choices"))
    }

    async fn call_gemini(&self, system_prompt: &str, messages: &[Message]) -> Result<String> {
        let max_tokens = crate::settings::get().max_tokens;
        Ok(self.call_gemini_models(&model_chain(&self.model()), system_prompt, messages, max_tokens).await?.for_student(None))
    }

    /// Stream an answer from Gemini (`streamGenerateContent`, SSE), trying the model chain
    /// until one accepts the request. Text goes to `tx` as it arrives; returns the whole answer.
    async fn stream_gemini(&self, system_prompt: &str, messages: &[Message], tx: &tokio::sync::mpsc::UnboundedSender<String>) -> Result<String> {
        let req = gemini_request(system_prompt, messages, crate::settings::get().max_tokens)?;
        let mut last_err = anyhow::anyhow!("no Gemini model configured");
        for model in model_chain(&self.model()) {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse");
            let mut resp = self.http.post(&url).header("x-goog-api-key", &self.llm.api_key).json(&req).send().await?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                if status.as_u16() == 429 { mark_exhausted(&model, &body); }
                last_err = anyhow::anyhow!("Gemini API error {} ({}): {}", status, model, body);
                if !try_next_model(status.as_u16()) { break; }
                continue;
            }
            let mut answer = GeminiAnswer::default();
            let mut buf: Vec<u8> = Vec::new();
            let take_lines = |buf: &mut Vec<u8>, answer: &mut GeminiAnswer, last: bool| {
                while let Some(pos) = buf.iter().position(|&b| b == b'\n').or(if last && !buf.is_empty() { Some(buf.len() - 1) } else { None }) {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    if let Some(piece) = sse_piece(&String::from_utf8_lossy(&line)) {
                        if !piece.text.is_empty() { let _ = tx.send(piece.text.clone()); }
                        answer.add(piece);
                    }
                }
            };
            loop {
                match resp.chunk().await {
                    Ok(Some(chunk)) => {
                        buf.extend_from_slice(&chunk);
                        take_lines(&mut buf, &mut answer, false);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        answer.error.get_or_insert(format!("connection lost: {e}"));
                        break;
                    }
                }
            }
            take_lines(&mut buf, &mut answer, true);
            let sent_text = !answer.text.is_empty();
            match answer.finish(&model) {
                Ok(t) => return Ok(t.for_student(Some(tx))),
                // Nothing shown yet: the next model can still give the whole answer.
                Err(e) if !sent_text => {
                    log::warn!("[llm] {e} — trying the next model");
                    last_err = anyhow::anyhow!(e);
                }
                // Part of the answer already went to the client: report an error instead of
                // ending as if the cut-off text were the full answer.
                Err(e) => return Err(anyhow::anyhow!(e)),
            }
        }
        Err(last_err)
    }

    /// Try `models` in order (next one on quota / overload / unknown model).
    async fn call_gemini_models(&self, models: &[String], system_prompt: &str, messages: &[Message], max_tokens: u32) -> Result<GeminiText> {
        let req = gemini_request(system_prompt, messages, max_tokens)?;

        let mut last_err = anyhow::anyhow!("no Gemini model configured");
        for model in models.iter().cloned() {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
            let resp = self.http.post(&url).header("x-goog-api-key", &self.llm.api_key).json(&req).send().await?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                if status.as_u16() == 429 { mark_exhausted(&model, &body); }
                last_err = anyhow::anyhow!("Gemini API error {} ({}): {}", status, model, body);
                if !try_next_model(status.as_u16()) { break; }
                continue;
            }
            // HTTP 200 does not mean a whole answer: check finishReason, and on a broken
            // answer try the next model instead of returning the partial text.
            let mut answer = GeminiAnswer::default();
            match resp.json::<serde_json::Value>().await {
                Ok(v)  => answer.add(gemini_piece(&v)),
                Err(e) => answer.error = Some(format!("unreadable response: {e}")),
            }
            match answer.finish(&model) {
                Ok(t) => return Ok(t),
                Err(e) => {
                    log::warn!("[llm] {e} — trying the next model");
                    last_err = anyhow::anyhow!(e);
                }
            }
        }
        Err(last_err)
    }
}

/// Gemini request body: system instruction + conversation (system messages skipped).
fn gemini_request(system_prompt: &str, messages: &[Message], max_tokens: u32) -> Result<GeminiRequest> {
    let contents: Vec<GeminiContent> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| GeminiContent {
            role:  if m.role == "assistant" { "model".to_string() } else { "user".to_string() },
            parts: vec![GeminiPart { text: m.content.clone() }],
        })
        .collect();
    if contents.is_empty() {
        return Err(anyhow::anyhow!("No messages to send to Gemini"));
    }
    Ok(GeminiRequest {
        system_instruction: GeminiSystemInstruction {
            parts: vec![GeminiPart { text: system_prompt.to_string() }],
        },
        contents,
        generation_config: GeminiGenerationConfig {
            max_output_tokens: max_tokens,
            temperature:       crate::settings::get().temperature as f32,
        },
    })
}

/// What one Gemini response object says: the whole reply of `generateContent`, or one
/// event of `streamGenerateContent`. Read as JSON values, so parts without text
/// (e.g. `thoughtSignature`) do not break parsing.
#[derive(Debug, Default, PartialEq)]
struct GeminiPiece {
    text:   String,
    finish: Option<String>,
    error:  Option<String>,
}

fn gemini_piece(v: &serde_json::Value) -> GeminiPiece {
    let cand = &v["candidates"][0];
    // All text parts, not only the first: Gemini may split one answer into several parts.
    // Thought summaries ("thought": true) are not part of the answer.
    let text = cand["content"]["parts"].as_array()
        .map(|parts| parts.iter()
            .filter(|p| p["thought"].as_bool() != Some(true))
            .filter_map(|p| p["text"].as_str())
            .collect())
        .unwrap_or_default();
    let error = v.get("error").map(|e| format!("{} {}", e["code"], e["message"].as_str().unwrap_or("")))
        .or_else(|| v["promptFeedback"]["blockReason"].as_str().map(|r| format!("prompt blocked: {r}")));
    GeminiPiece { text, finish: cand["finishReason"].as_str().map(str::to_string), error }
}

/// One SSE line from `streamGenerateContent` ("data: {…}"); None for other lines.
fn sse_piece(line: &str) -> Option<GeminiPiece> {
    let data = line.trim().strip_prefix("data:")?.trim();
    serde_json::from_str(data).ok().map(|v| gemini_piece(&v))
}

/// A Gemini answer put together from its pieces.
#[derive(Default)]
struct GeminiAnswer {
    text:   String,
    finish: Option<String>,
    error:  Option<String>,
}

impl GeminiAnswer {
    fn add(&mut self, piece: GeminiPiece) {
        self.text.push_str(&piece.text);
        if piece.finish.is_some() { self.finish = piece.finish; }
        if piece.error.is_some() { self.error = piece.error; }
    }

    /// The text only when the model said it finished. A missing finishReason (stream cut off),
    /// an error event, or a stop for SAFETY / RECITATION / OTHER means a broken answer.
    /// MAX_TOKENS is kept: it is a long answer cut at the configured limit, and the next
    /// model has the same limit.
    fn finish(self, model: &str) -> std::result::Result<GeminiText, String> {
        if let Some(e) = self.error {
            return Err(format!("Gemini error during the answer ({model}): {e}"));
        }
        if self.text.trim().is_empty() {
            return Err(format!("Gemini returned an empty answer ({model}, finishReason {:?})", self.finish));
        }
        match self.finish.as_deref() {
            Some("STOP") => Ok(GeminiText { text: self.text, truncated: false }),
            Some("MAX_TOKENS") => {
                log::warn!("[llm] {model} answer hit maxOutputTokens — consider raising max_tokens");
                Ok(GeminiText { text: self.text, truncated: true })
            }
            None => Err(format!("Gemini answer stopped before it finished ({model}, no finishReason)")),
            Some(other) => Err(format!("Gemini answer incomplete ({model}, finishReason {other})")),
        }
    }
}

/// A finished Gemini answer; `truncated` = it stopped at maxOutputTokens.
#[derive(Debug, PartialEq)]
struct GeminiText {
    text:      String,
    truncated: bool,
}

impl GeminiText {
    /// The answer as the student sees it: a cut answer gets `TRUNCATED_NOTE`, which is also
    /// sent to `tx` when the answer was streamed (the text itself already went there).
    fn for_student(self, tx: Option<&tokio::sync::mpsc::UnboundedSender<String>>) -> String {
        if !self.truncated { return self.text; }
        if let Some(tx) = tx { let _ = tx.send(TRUNCATED_NOTE.to_string()); }
        self.text + TRUNCATED_NOTE
    }
}

/// Added to an answer that stopped at maxOutputTokens, so the student knows it is incomplete.
pub const TRUNCATED_NOTE: &str = "\n\n_Jawaban terpotong karena terlalu panjang, coba tanyakan lebih spesifik._";

/// Whether an answer carries `TRUNCATED_NOTE` (logged as `query_logs.truncated`).
pub fn is_truncated(answer: &str) -> bool {
    answer.trim_end().ends_with(TRUNCATED_NOTE.trim())
}

/// Answer when no document matched (no LLM call).
pub const NO_CONTEXT_ANSWER: &str = "Maaf, saya tidak menemukan informasi yang relevan dengan pertanyaan tersebut \
     dalam dokumen yang tersedia.";

// ══════════════════════════════════════════════════════════════════
//  PROMPT BUILDERS
// ══════════════════════════════════════════════════════════════════

fn build_system_prompt() -> String {
    "Kamu adalah Asisten Akademik PMPSTI (Program Magister Teknik Sistem Informasi) \
     Universitas Gadjah Mada yang pintar dan ramah. \
     \n\nAturan:\n\
     1. Untuk pertanyaan seputar akademik, kurikulum, dokumen kampus, atau informasi PMPSTI: \
        jawab berdasarkan teks Konteks yang diberikan, dan sebutkan sumber dengan format [nomor].\n\
     2. Jika informasi spesifik tidak ada di Konteks, sampaikan bahwa informasi tersebut tidak \
        tersedia dalam dokumen yang diindeks, lalu sarankan pengguna menghubungi pihak program \
        studi secara langsung.\n\
     3. Untuk pertanyaan casual atau sapaan (contoh: apa kabar, halo, siapa kamu), jawab secara \
        natural dan ramah sebagai asisten akademik — tidak perlu merujuk dokumen.\n\
     4. DAFTAR/TABEL: Jika konteks berisi daftar (mitra, mata kuliah, dosen, dsb), WAJIB tampilkan \
        SEMUA entri secara lengkap — jangan meringkas, jangan hanya sebagian. \
        Gunakan tabel Markdown (| No | Nama | ... |) jika ada 3+ kolom data.\n\
     5. PROSEDUR/LANGKAH: Jika pertanyaan tentang cara/syarat/prosedur, tampilkan sebagai \
        numbered list yang urut dan lengkap. Jika ada prasyarat, sebutkan dulu.\n\
     6. PERBANDINGAN: Jika pertanyaan membandingkan dua hal, gunakan tabel Markdown dengan \
        aspek di baris dan objek di kolom.\n\
     7. Format jawaban menggunakan Markdown: **bold** untuk penekanan, tabel dengan | kolom |, \
        daftar dengan - atau nomor. JANGAN tulis simbol ** atau | secara literal tanpa maksud \
        pemformatan.\n\
     8. Jangan mengarang fakta akademik yang tidak ada di konteks.\n\
     9. VERSI: Jika beberapa sumber memberi angka atau aturan berbeda untuk hal yang sama \
        (misalnya jumlah SKS), pakai sumber dengan baris \"Versi\" yang paling baru \
        (tanggal berlaku terbaru) sebagai jawaban utama, lalu sebutkan singkat bahwa versi lama \
        menyebut nilai lain, keduanya dengan nomor sumbernya. Jika versinya tidak diketahui, \
        sebutkan kedua nilai dan sarankan konfirmasi ke program studi."
        .to_string()
}
fn build_user_message(query: &str, context: &str, clarification_hint: &str) -> String {
    format!(
        "Konteks dari dokumen:\n\
         ─────────────────────\n\
         {context}\n\
         ─────────────────────\n\
         {clarification_hint}\n\
         Pertanyaan: {query}"
    )
}

#[cfg(test)]
mod model_chain_tests {
    use super::*;

    fn stream(lines: &[&str]) -> GeminiAnswer {
        let mut a = GeminiAnswer::default();
        for l in lines { if let Some(p) = sse_piece(l) { a.add(p); } }
        a
    }

    #[test]
    fn reads_text_from_stream_lines() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"Tesis "},{"text":"6 SKS"}],"role":"model"}}]}"#;
        assert_eq!(sse_piece(line).unwrap().text, "Tesis 6 SKS");
        assert_eq!(sse_piece(r#"data: {"candidates":[{"finishReason":"STOP"}]}"#).unwrap().finish.as_deref(), Some("STOP"));
        assert_eq!(sse_piece(""), None);
    }

    #[test]
    fn whole_stream_with_stop_is_the_answer() {
        let a = stream(&[
            r#"data: {"candidates":[{"content":{"parts":[{"text":"Halo! Saya As"}]}}]}"#,
            "",
            r#"data: {"candidates":[{"content":{"parts":[{"text":"isten Akademik."}]},"finishReason":"STOP"}]}"#,
        ]);
        assert_eq!(a.finish("m").unwrap().text, "Halo! Saya Asisten Akademik.");
    }

    #[test]
    fn cut_off_stream_is_an_error_not_an_answer() {
        // E6 eval: HTTP 200, text stops mid-word, no finishReason (connection dropped / 503)
        let a = stream(&[r#"data: {"candidates":[{"content":{"parts":[{"text":"Halo! Saya As"}]}}]}"#]);
        assert!(a.finish("m").unwrap_err().contains("no finishReason"));
        // An error event in the middle of the stream
        let a = stream(&[
            r#"data: {"candidates":[{"content":{"parts":[{"text":"**P"}]}}]}"#,
            r#"data: {"error":{"code":503,"message":"The model is overloaded.","status":"UNAVAILABLE"}}"#,
        ]);
        assert!(a.finish("m").unwrap_err().contains("503"));
        // Stopped for another reason than STOP / MAX_TOKENS
        let a = stream(&[r#"data: {"candidates":[{"content":{"parts":[{"text":"Pendaftaran ujian"}]},"finishReason":"OTHER"}]}"#]);
        assert!(a.finish("m").unwrap_err().contains("OTHER"));
    }

    #[test]
    fn non_stream_reply_joins_all_parts_and_skips_thoughts() {
        // generateContent may split one answer into several parts; only parts[0] was kept before
        let v = serde_json::json!({"candidates":[{"content":{"role":"model","parts":[
            {"text":"thinking…","thought":true},
            {"text":"**P"},
            {"text":"rasyarat** tesis: 6 SKS [1]."},
            {"thoughtSignature":"abc"}
        ]},"finishReason":"STOP"}]});
        let mut a = GeminiAnswer::default();
        a.add(gemini_piece(&v));
        assert_eq!(a.finish("m").unwrap().text, "**Prasyarat** tesis: 6 SKS [1].");
    }

    #[test]
    fn truncated_answer_gets_the_note_in_both_paths() {
        let cut = || GeminiText { text: "| 1 | Suyanto |".into(), truncated: true };
        // Non-streaming: the note is appended to the returned answer
        let answer = cut().for_student(None);
        assert_eq!(answer, format!("| 1 | Suyanto |{TRUNCATED_NOTE}"));
        assert!(is_truncated(&answer));
        // Streaming: the note also goes to the client after the streamed text
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(is_truncated(&cut().for_student(Some(&tx))));
        assert_eq!(rx.try_recv().unwrap(), TRUNCATED_NOTE);
        // A finished answer is unchanged and sends nothing extra
        let done = GeminiText { text: "Tesis 6 SKS [1].".into(), truncated: false }.for_student(Some(&tx));
        assert_eq!(done, "Tesis 6 SKS [1].");
        assert!(!is_truncated(&done));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn max_tokens_is_kept_but_empty_or_blocked_is_not() {
        let mut a = GeminiAnswer::default();
        a.add(gemini_piece(&serde_json::json!({"candidates":[{"content":{"parts":[{"text":"| No | Nama |"}]},"finishReason":"MAX_TOKENS"}]})));
        assert!(a.finish("m").unwrap().truncated);
        let mut a = GeminiAnswer::default();
        a.add(gemini_piece(&serde_json::json!({"candidates":[{"finishReason":"STOP"}]})));
        assert!(a.finish("m").unwrap_err().contains("empty"));
        let mut a = GeminiAnswer::default();
        a.add(gemini_piece(&serde_json::json!({"promptFeedback":{"blockReason":"SAFETY"}})));
        assert!(a.finish("m").unwrap_err().contains("blocked"));
    }

    #[test]
    fn fallbacks_follow_primary_and_cooling_models_go_last() {
        let now = Instant::now();
        let none = HashMap::new();
        assert_eq!(chain_from("gemini-3.5-flash-lite", "gemini-3.1-flash-lite, gemini-3.5-flash-lite", &none, now),
                   vec!["gemini-3.5-flash-lite", "gemini-3.1-flash-lite"]);
        let mut cooling = HashMap::new();
        cooling.insert("gemini-3.5-flash-lite".to_string(), now + Duration::from_secs(60));
        assert_eq!(chain_from("gemini-3.5-flash-lite", "gemini-3.1-flash-lite", &cooling, now),
                   vec!["gemini-3.1-flash-lite", "gemini-3.5-flash-lite"]);
        assert_eq!(chain_from("llama3.2", "", &none, now), vec!["llama3.2"]);
    }
}
