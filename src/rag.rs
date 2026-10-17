use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

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
    pub max_tokens: u32,
}

impl LlmConfig {
    pub fn ollama(model: &str) -> Self {
        Self {
            provider:   LlmProvider::Ollama,
            api_url:    "http://localhost:11434/api/chat".to_string(),
            api_key:    String::new(),
            model:      model.to_string(),
            max_tokens: 2048,
        }
    }

    pub fn openai_compatible(api_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            provider:   LlmProvider::OpenAICompatible,
            api_url:    api_url.to_string(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
            max_tokens: 2048,
        }
    }

    pub fn gemini(api_key: &str, model: &str) -> Self {
        Self {
            provider:   LlmProvider::Gemini,
            api_url:    String::new(),
            api_key:    api_key.to_string(),
            model:      model.to_string(),
            max_tokens: 2048,
        }
    }
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

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
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
        Self { search, llm, http: Client::new() }
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

        // Model klasifikasi: GEMINI_CLASSIFY_MODEL > GEMINI_MODEL > default flash-lite
        let model = std::env::var("GEMINI_CLASSIFY_MODEL")
            .or_else(|_| std::env::var("GEMINI_MODEL"))
            .unwrap_or_else(|_| "gemini-3.5-flash-lite".to_string());
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            model
        );

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

        let resp = self.http.post(&url)
            .header("x-goog-api-key", &api_key)
            .json(&body)
            .send().await.ok()?;

        if !resp.status().is_success() { return None; }

        let data: GeminiResp = resp.json().await.ok()?;
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

    /// Cari chunks dengan strategi retrieval modern:
    /// 1. RRF (default) / semantic / keyword sesuai search_mode
    /// 2. Query-type detection → fetch semua chunks untuk list/procedural
    /// 3. Comparison → fetch dari top-N dokumen berbeda
    /// 4. Smart document expansion → expand doc multi-chunk yang ter-hit
    pub async fn search_chunks(
        &self,
        query:           &str,
        category_filter: Option<&str>,
        search_mode:     &str,
        top_k:           i64,
    ) -> Result<Vec<SearchResult>> {
        let initial_chunks = match search_mode {
            "semantic" => self.search.search_semantic(query, top_k, category_filter).await?,
            "keyword"  => self.search.search_keyword(query, top_k, category_filter).await?,
            "hybrid"   => self.search.search(query, top_k, category_filter).await?,
            _          => self.search.search_rrf(query, top_k, category_filter).await?,
        };

        if initial_chunks.is_empty() {
            return Ok(vec![]);
        }

        let query_type = self.classify_query(query).await;

        // ── List & Procedural: fetch semua chunks dari matching docs ─────
        // List:       kerjasama, mata kuliah, dosen → harus lengkap semua entri
        // Procedural: syarat masuk, cara daftar     → harus lengkap semua langkah
        if matches!(query_type, QueryType::List | QueryType::Procedural) {
            let hit_doc_ids: Vec<String> = {
                let mut seen = std::collections::HashSet::new();
                initial_chunks.iter()
                    .filter(|c| !c.document_id.is_empty())
                    .filter_map(|c| {
                        if seen.insert(c.document_id.clone()) { Some(c.document_id.clone()) }
                        else { None }
                    })
                    .collect()
            };
            let mut all_chunks = self.search.db
                .fetch_all_chunks_for_documents(&hit_doc_ids)
                .await
                .unwrap_or(initial_chunks);
            let mut seen_keys = std::collections::HashSet::new();
            all_chunks.retain(|c| seen_keys.insert((c.document_id.clone(), c.chunk_index)));
            // Urut: skor tertinggi di depan, lalu per chunk_index (urutan asli)
            all_chunks.sort_by(|a, b| {
                b.score.partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.chunk_index.cmp(&b.chunk_index))
            });
            return Ok(all_chunks);
        }

        // ── Comparison: pastikan minimal 2 dokumen berbeda ter-representasi ─
        if matches!(query_type, QueryType::Comparison) {
            // Ambil top-2 dokumen berbeda, fetch semua chunk dari masing-masing
            let mut seen_docs = std::collections::HashSet::new();
            let top_doc_ids: Vec<String> = initial_chunks.iter()
                .filter(|c| !c.document_id.is_empty())
                .filter_map(|c| {
                    if seen_docs.insert(c.document_id.clone()) { Some(c.document_id.clone()) }
                    else { None }
                })
                .take(2)
                .collect();
            if top_doc_ids.len() >= 2 {
                let mut all_chunks = self.search.db
                    .fetch_all_chunks_for_documents(&top_doc_ids)
                    .await
                    .unwrap_or(initial_chunks);
                let mut seen_keys = std::collections::HashSet::new();
                all_chunks.retain(|c| seen_keys.insert((c.document_id.clone(), c.chunk_index)));
                all_chunks.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
                return Ok(all_chunks);
            }
            // Hanya 1 dokumen — lanjut ke smart expansion biasa
        }

        // ── Smart document expansion (General & Definition) ──────────────
        // Expand dokumen yang:
        // (a) 2+ chunks-nya ter-hit (dokumen sangat relevan), ATAU
        // (b) salah satu chunk-nya bukan chunk_index=0 (ada prefix yang belum diambil)
        let mut doc_hit_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut doc_has_nonzero_chunk: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &initial_chunks {
            if !c.document_id.is_empty() {
                *doc_hit_count.entry(c.document_id.clone()).or_insert(0) += 1;
                if c.chunk_index.unwrap_or(0) > 0 {
                    doc_has_nonzero_chunk.insert(c.document_id.clone());
                }
            }
        }
        let expand_ids: Vec<String> = doc_hit_count.into_iter()
            .filter(|(id, count)| *count >= 2 || doc_has_nonzero_chunk.contains(id))
            .map(|(id, _)| id)
            .collect();

        if expand_ids.is_empty() {
            return Ok(initial_chunks);
        }

        let mut expanded = self.search.db
            .fetch_all_chunks_for_documents(&expand_ids)
            .await
            .unwrap_or_default();
        // Tambahkan chunks dari dokumen lain yang tidak di-expand
        for c in initial_chunks {
            if !expand_ids.contains(&c.document_id) {
                expanded.push(c);
            }
        }
        // Deduplicate
        let mut seen_keys = std::collections::HashSet::new();
        expanded.retain(|c| seen_keys.insert((c.document_id.clone(), c.chunk_index)));
        Ok(expanded)
    }

    /// Panggil LLM dengan chunks yang sudah difilter oleh handler.
    /// chunks = apa yang akan ditampilkan sebagai kartu sumber → nomor [1]..[N] cocok.
    /// Menerima slice of references (sesuai return type filter_sources).
    pub async fn answer_with_chunks(
        &self,
        query:   &str,
        history: &[Message],
        chunks:  &[&SearchResult],
    ) -> Result<String> {
        if chunks.is_empty() {
            return Ok(
                "Maaf, saya tidak menemukan informasi yang relevan dengan pertanyaan tersebut \
                 dalam dokumen yang tersedia.".to_string()
            );
        }

        // Format context dari references
        let context = SearchEngine::format_context_refs(chunks);
        let query_type = self.classify_query(query).await;
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

        let answer = match self.llm.provider {
            LlmProvider::Ollama           => self.call_ollama(&messages).await?,
            LlmProvider::Gemini           => self.call_gemini(&system_prompt, &messages).await?,
            LlmProvider::OpenAICompatible => self.call_openai_compatible(&messages).await?,
        };

        Ok(answer)
    }

    async fn call_ollama(&self, messages: &[Message]) -> Result<String> {
        let req = OllamaChatRequest {
            model:    self.llm.model.clone(),
            messages: messages.to_vec(),
            stream:   false,
            options:  OllamaOptions { num_predict: self.llm.max_tokens, temperature: 0.3 },
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
        let req = ChatRequest {
            model:       self.llm.model.clone(),
            messages:    messages.to_vec(),
            max_tokens:  self.llm.max_tokens,
            temperature: 0.3,
            stream:      false,
        };

        let mut builder = self.http.post(&self.llm.api_url).json(&req);
        if !self.llm.api_key.is_empty() {
            builder = builder.bearer_auth(&self.llm.api_key);
        }

        let resp = builder.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body   = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("LLM API error {}: {}", status, body));
        }

        let data: ChatResponse = resp.json().await?;
        data.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("LLM returned empty choices"))
    }

    async fn call_gemini(&self, system_prompt: &str, messages: &[Message]) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.llm.model, self.llm.api_key
        );

        // Konversi messages ke format Gemini — skip system (sudah di system_instruction)
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

        let req = GeminiRequest {
            system_instruction: GeminiSystemInstruction {
                parts: vec![GeminiPart { text: system_prompt.to_string() }],
            },
            contents,
            generation_config: GeminiGenerationConfig {
                max_output_tokens: self.llm.max_tokens,
                temperature:       0.3,
            },
        };

        let resp = self.http.post(&url).json(&req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body   = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Gemini API error {}: {}", status, body));
        }

        let data: GeminiResponse = resp.json().await?;
        data.candidates
            .into_iter()
            .next()
            .and_then(|c| c.content.parts.into_iter().next())
            .map(|p| p.text)
            .ok_or_else(|| anyhow::anyhow!("Gemini returned empty response"))
    }
}

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
     8. Jangan mengarang fakta akademik yang tidak ada di konteks."
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
