use anyhow::Result;
use chrono::Utc;
use pgvector::Vector;
use sqlx::{PgPool, Row};

// ══════════════════════════════════════════════════════════════════
//  MODELS
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub document_id:  String,
    pub title:        String,
    pub content:      String,
    pub snippet:      String,   // extracted window, siap tampil ke user
    pub category:     String,
    pub subcategory:  String,
    pub source_url:   String,
    pub page_number:  Option<i32>,
    pub chunk_index:  Option<i32>,
    pub score:        f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct User {
    pub id:             i64,
    pub email:          String,
    #[serde(skip_serializing)]
    pub password_hash:  String,
    pub display_name:   String,
    pub role:           String,
    pub is_active:      bool,
    pub email_verified: bool,
    pub created_at:     String,
    pub updated_at:     String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatSession {
    pub id:          String,
    pub user_id:     i64,
    pub title:       String,
    pub messages:    Vec<ChatMessage>,
    pub created_at:  String,
    pub updated_at:  String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatSource {
    pub title:      String,
    pub snippet:    String,
    pub source_url: String,
    pub category:   String,
    pub subcategory: String,
    pub score:      f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role:       String,   // "user" | "assistant"
    pub content:    String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources:    Vec<ChatSource>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionListItem {
    pub id:            String,
    pub title:         String,
    pub updated_at:    String,
    pub message_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiKeyInfo {
    pub id:           i64,
    pub key_prefix:   String,
    pub name:         String,
    pub permissions:  Vec<String>,
    pub rate_limit:   i64,
    pub is_active:    bool,
    pub last_used_at: Option<String>,
    pub created_at:   String,
    pub expires_at:   Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct QueryLogStats {
    pub total_queries:        i64,
    pub unique_queries:       i64,
    pub avg_results:          f64,
    pub avg_search_time_ms:   f64,
    pub zero_result_queries:  i64,
    pub language_distribution: Vec<(String, i64)>,
    pub domain_distribution:   Vec<(String, i64)>,
    pub queries_per_day:       Vec<(String, i64)>,
    pub top_queries:           Vec<(String, i64)>,
}

// ══════════════════════════════════════════════════════════════════
//  DATABASE
// ══════════════════════════════════════════════════════════════════

pub struct Database {
    pub pool: PgPool,
}

impl Database {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ════════════════════════════════════════════════════════════
    //  SETUP & MIGRATION
    //  Aman dijalankan berulang — semua pakai IF NOT EXISTS /
    //  ADD COLUMN IF NOT EXISTS (PostgreSQL 9.6+)
    // ════════════════════════════════════════════════════════════
    pub async fn setup(&self) -> Result<()> {
        // Extension
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&self.pool)
            .await?;

        // Tabel utama dokumen (sudah ada dari ingest, tidak diubah)
        sqlx::query("
            CREATE TABLE IF NOT EXISTS documents (
                id            SERIAL PRIMARY KEY,
                document_id   TEXT NOT NULL,
                title         TEXT,
                content       TEXT,
                document_type TEXT,
                source_url    TEXT,
                category      TEXT DEFAULT '',
                subcategory   TEXT DEFAULT '',
                file_hash     TEXT DEFAULT '',
                embedding     vector(768),
                page_number   INTEGER,
                chunk_index   INTEGER,
                created_at    TIMESTAMPTZ DEFAULT NOW(),
                updated_at    TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE INDEX IF NOT EXISTS documents_embedding_idx
            ON documents USING hnsw (embedding vector_cosine_ops)
        ").execute(&self.pool).await?;

        // GIN index untuk full-text search
        sqlx::query("
            CREATE INDEX IF NOT EXISTS documents_fts_idx
            ON documents USING gin (to_tsvector('indonesian', content))
        ").execute(&self.pool).await?;

        // Unique index per chunk (document_id + chunk_index)
        sqlx::query("
            CREATE UNIQUE INDEX IF NOT EXISTS documents_doc_chunk_key
            ON documents(document_id, chunk_index)
        ").execute(&self.pool).await?;

        // Index kategori untuk filter
        sqlx::query("
            CREATE INDEX IF NOT EXISTS documents_category_idx
            ON documents(category, subcategory)
        ").execute(&self.pool).await?;

        // Index source_url untuk lookup
        sqlx::query("
            CREATE INDEX IF NOT EXISTS documents_source_url_idx
            ON documents(source_url)
        ").execute(&self.pool).await?;

        // Users
        sqlx::query("
            CREATE TABLE IF NOT EXISTS users (
                id            BIGSERIAL PRIMARY KEY,
                email         TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                display_name  TEXT DEFAULT '',
                role          TEXT DEFAULT 'user',
                is_active     BOOLEAN DEFAULT TRUE,
                created_at    TIMESTAMPTZ DEFAULT NOW(),
                updated_at    TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        // Chat sessions
        sqlx::query("
            CREATE TABLE IF NOT EXISTS chat_sessions (
                id         TEXT PRIMARY KEY,
                user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                title      TEXT DEFAULT '',
                messages   JSONB DEFAULT '[]',
                created_at TIMESTAMPTZ DEFAULT NOW(),
                updated_at TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE INDEX IF NOT EXISTS chat_sessions_user_idx
            ON chat_sessions(user_id, updated_at DESC)
        ").execute(&self.pool).await?;

        // API keys
        sqlx::query("
            CREATE TABLE IF NOT EXISTS api_keys (
                id           BIGSERIAL PRIMARY KEY,
                user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                key_prefix   TEXT NOT NULL,
                key_hash     TEXT NOT NULL,
                name         TEXT NOT NULL DEFAULT 'Default',
                permissions  JSONB DEFAULT '[\"search\",\"read\"]',
                rate_limit   INTEGER DEFAULT 30,
                is_active    BOOLEAN DEFAULT TRUE,
                last_used_at TIMESTAMPTZ,
                created_at   TIMESTAMPTZ DEFAULT NOW(),
                expires_at   TIMESTAMPTZ
            )
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE UNIQUE INDEX IF NOT EXISTS api_keys_prefix_idx
            ON api_keys(key_prefix)
        ").execute(&self.pool).await?;

        // Query logs
        sqlx::query("
            CREATE TABLE IF NOT EXISTS query_logs (
                id                BIGSERIAL PRIMARY KEY,
                query_text        TEXT NOT NULL,
                detected_language TEXT DEFAULT '',
                detected_domain   TEXT DEFAULT '',
                num_results       INTEGER DEFAULT 0,
                top_score         REAL DEFAULT 0.0,
                search_time_ms    BIGINT DEFAULT 0,
                session_id        TEXT,
                user_id           BIGINT,
                created_at        TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE INDEX IF NOT EXISTS query_logs_created_idx
            ON query_logs(created_at DESC)
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE INDEX IF NOT EXISTS query_logs_language_idx
            ON query_logs(detected_language)
        ").execute(&self.pool).await?;


        // Guest quotas (public chat, no auth)
        sqlx::query("
            CREATE TABLE IF NOT EXISTS guest_quotas (
                guest_token TEXT PRIMARY KEY,
                ask_count   INTEGER DEFAULT 0,
                last_ask_at TIMESTAMPTZ DEFAULT NOW(),
                created_at  TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        // Email verification tokens
        sqlx::query("
            CREATE TABLE IF NOT EXISTS email_verifications (
                id         BIGSERIAL PRIMARY KEY,
                user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token      TEXT UNIQUE NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                used_at    TIMESTAMPTZ,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE INDEX IF NOT EXISTS email_verif_token_idx
            ON email_verifications(token)
        ").execute(&self.pool).await?;

        // Password reset tokens
        sqlx::query("
            CREATE TABLE IF NOT EXISTS password_resets (
                id         BIGSERIAL PRIMARY KEY,
                user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token      TEXT UNIQUE NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                used_at    TIMESTAMPTZ,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        sqlx::query("
            CREATE INDEX IF NOT EXISTS password_resets_token_idx
            ON password_resets(token)
        ").execute(&self.pool).await?;

        // ── Safe migrations: tambah kolom baru kalau belum ada ──
        // (PostgreSQL 9.6+ mendukung ADD COLUMN IF NOT EXISTS)
        let migrations = [
            // Users
            "ALTER TABLE users ADD COLUMN IF NOT EXISTS display_name TEXT DEFAULT ''",
            "ALTER TABLE users ADD COLUMN IF NOT EXISTS role TEXT DEFAULT 'user'",
            "ALTER TABLE users ADD COLUMN IF NOT EXISTS is_active BOOLEAN DEFAULT TRUE",
            // Query logs
            "ALTER TABLE query_logs ADD COLUMN IF NOT EXISTS user_id BIGINT",
            "ALTER TABLE query_logs ADD COLUMN IF NOT EXISTS detected_domain TEXT DEFAULT ''",
            // Documents — safe migration untuk DB yang sudah ada
            "ALTER TABLE documents ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ DEFAULT NOW()",
            "ALTER TABLE documents ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT NOW()",
            "CREATE UNIQUE INDEX IF NOT EXISTS documents_doc_chunk_key ON documents(document_id, chunk_index)",
            "CREATE INDEX IF NOT EXISTS documents_category_idx ON documents(category, subcategory)",
            "CREATE INDEX IF NOT EXISTS documents_source_url_idx ON documents(source_url)",
            // Auth
            "ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified BOOLEAN DEFAULT FALSE",
        ];

        for sql in &migrations {
            if let Err(e) = sqlx::query(sql).execute(&self.pool).await {
                log::warn!("Migration skipped ({}): {}", sql, e);
            }
        }

        // Tabel faq_pins: admin bisa pin query tertentu agar selalu muncul di FAQ
        sqlx::query("
            CREATE TABLE IF NOT EXISTS faq_pins (
                id         BIGSERIAL PRIMARY KEY,
                question   TEXT NOT NULL UNIQUE,
                pinned_by  BIGINT REFERENCES users(id) ON DELETE SET NULL,
                sort_order INTEGER DEFAULT 0,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
        ").execute(&self.pool).await?;

        log::info!("Database schema ready.");
        Ok(())
    }


    // ════════════════════════════════════════════════════════════
    //  SEARCH — PURE VECTOR
    // ════════════════════════════════════════════════════════════
    pub async fn search_vector(
        &self,
        query_embedding: Vec<f32>,
        limit: i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        let vec = Vector::from(query_embedding);

        let rows = sqlx::query(
            "SELECT document_id, title, content, category, subcategory,
                    COALESCE(source_url, '') AS source_url,
                    page_number, chunk_index,
                    COALESCE(1.0 - (embedding <=> $1), 0.0) AS score
             FROM documents
             WHERE embedding IS NOT NULL
               AND ($3::text IS NULL OR category = $3)
             ORDER BY embedding <=> $1
             LIMIT $2",
        )
        .bind(&vec)
        .bind(limit)
        .bind(category_filter)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows_to_results(rows))
    }

    // ════════════════════════════════════════════════════════════
    //  SEARCH — HYBRID (Vector 70% + BM25 30%)
    // ════════════════════════════════════════════════════════════
    pub async fn search_hybrid(
        &self,
        query_text: &str,
        query_embedding: Vec<f32>,
        limit: i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        let vec = Vector::from(query_embedding);

        let rows = sqlx::query(
            "SELECT document_id, title, content, category, subcategory,
                    COALESCE(source_url, '') AS source_url,
                    page_number, chunk_index,
                    (
                        0.7 * COALESCE(1.0 - (embedding <=> $1), 0.0)
                      + 0.3 * ts_rank(
                            to_tsvector('indonesian', content),
                            plainto_tsquery('indonesian', $2),
                            1
                        )
                    ) AS score
             FROM documents
             WHERE ($4::text IS NULL OR category = $4)
             ORDER BY score DESC
             LIMIT $3",
        )
        .bind(&vec)
        .bind(query_text)
        .bind(limit)
        .bind(category_filter)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows_to_results(rows))
    }

    // ════════════════════════════════════════════════════════════
    //  SEARCH — RECIPROCAL RANK FUSION (RRF)
    //  Jalankan semantic + keyword secara terpisah, gabungkan dengan
    //  RRF(k=60): score = 1/(k + rank_semantic) + 1/(k + rank_keyword)
    //  Lebih robust dari fixed-weight hybrid karena tidak bergantung
    //  pada skala score yang berbeda antara cosine distance dan ts_rank.
    // ════════════════════════════════════════════════════════════
    pub async fn search_rrf(
        &self,
        query_text:      &str,
        query_embedding: Vec<f32>,
        limit:           i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        // Ambil lebih banyak kandidat dari masing-masing, lalu fuse
        let candidate_limit = (limit * 3).max(30);
        let vec = Vector::from(query_embedding);

        // Semantic candidates dengan rank
        let sem_rows = sqlx::query(
            "SELECT document_id, title, content, category, subcategory,
                    COALESCE(source_url, '') AS source_url,
                    page_number, chunk_index,
                    ROW_NUMBER() OVER (ORDER BY embedding <=> $1) AS rank
             FROM documents
             WHERE embedding IS NOT NULL
               AND ($3::text IS NULL OR category = $3)
             ORDER BY embedding <=> $1
             LIMIT $2",
        )
        .bind(&vec)
        .bind(candidate_limit)
        .bind(category_filter)
        .fetch_all(&self.pool)
        .await?;

        // Keyword candidates dengan rank (fallback graceful jika tidak ada match)
        let kw_rows = sqlx::query(
            "SELECT document_id, title, content, category, subcategory,
                    COALESCE(source_url, '') AS source_url,
                    page_number, chunk_index,
                    ROW_NUMBER() OVER (ORDER BY ts_rank(to_tsvector('indonesian', content),
                                       plainto_tsquery('indonesian', $1), 1) DESC) AS rank
             FROM documents
             WHERE ($3::text IS NULL OR category = $3)
             ORDER BY ts_rank(to_tsvector('indonesian', content),
                              plainto_tsquery('indonesian', $1), 1) DESC
             LIMIT $2",
        )
        .bind(query_text)
        .bind(candidate_limit)
        .bind(category_filter)
        .fetch_all(&self.pool)
        .await?;

        // Build map: chunk_key → (SearchResult, rrf_score)
        use std::collections::HashMap;
        let k = 60.0f64;
        let mut scores: HashMap<(String, Option<i32>), (SearchResult, f64)> = HashMap::new();

        for row in &sem_rows {
            let doc_id:     String       = row.get("document_id");
            let chunk_idx:  Option<i32>  = row.get("chunk_index");
            let rank:       i64          = row.get("rank");
            let rrf = 1.0 / (k + rank as f64);
            let key = (doc_id.clone(), chunk_idx);
            scores.entry(key)
                .and_modify(|e| e.1 += rrf)
                .or_insert_with(|| {
                    let sr = SearchResult {
                        document_id: doc_id,
                        title:       row.get::<Option<String>, _>("title").unwrap_or_default(),
                        content:     row.get::<Option<String>, _>("content").unwrap_or_default(),
                        snippet:     String::new(),
                        category:    row.get::<Option<String>, _>("category").unwrap_or_default(),
                        subcategory: row.get::<Option<String>, _>("subcategory").unwrap_or_default(),
                        source_url:  row.get("source_url"),
                        page_number: row.get("page_number"),
                        chunk_index: chunk_idx,
                        score:       0.0,
                    };
                    (sr, rrf)
                });
        }

        for row in &kw_rows {
            let doc_id:     String       = row.get("document_id");
            let chunk_idx:  Option<i32>  = row.get("chunk_index");
            let rank:       i64          = row.get("rank");
            let rrf = 1.0 / (k + rank as f64);
            let key = (doc_id.clone(), chunk_idx);
            scores.entry(key)
                .and_modify(|e| e.1 += rrf)
                .or_insert_with(|| {
                    let sr = SearchResult {
                        document_id: doc_id,
                        title:       row.get::<Option<String>, _>("title").unwrap_or_default(),
                        content:     row.get::<Option<String>, _>("content").unwrap_or_default(),
                        snippet:     String::new(),
                        category:    row.get::<Option<String>, _>("category").unwrap_or_default(),
                        subcategory: row.get::<Option<String>, _>("subcategory").unwrap_or_default(),
                        source_url:  row.get("source_url"),
                        page_number: row.get("page_number"),
                        chunk_index: chunk_idx,
                        score:       0.0,
                    };
                    (sr, rrf)
                });
        }

        // Sort by RRF score descending, assign final score, take limit
        let mut results: Vec<SearchResult> = scores
            .into_values()
            .map(|(mut sr, rrf_score)| { sr.score = rrf_score; sr })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit as usize);

        Ok(results)
    }

    // ════════════════════════════════════════════════════════════
    //  SEARCH — PURE FULL-TEXT (BM25)
    //  Used by the "keyword" mode and as the fallback when Gemini embedding fails.
    // ════════════════════════════════════════════════════════════
    pub async fn search_fulltext(
        &self,
        query_text: &str,
        limit: i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        // OR between words (not AND like plainto_tsquery), without question words:
        // document hit@5 rose from 7% to 83% in scripts/eval_retrieval.py.
        let Some(tsquery) = keyword_tsquery(query_text) else { return Ok(vec![]) };
        let rows = sqlx::query(
            "SELECT document_id, title, content, category, subcategory,
                    COALESCE(source_url, '') AS source_url,
                    page_number, chunk_index,
                    ts_rank(
                        to_tsvector('indonesian', content),
                        to_tsquery('indonesian', $1),
                        1
                    )::float8 AS score
             FROM documents
             WHERE to_tsvector('indonesian', content)
                   @@ to_tsquery('indonesian', $1)
               AND ($3::text IS NULL OR category = $3)
             ORDER BY score DESC
             LIMIT $2",
        )
        .bind(&tsquery)
        .bind(limit)
        .bind(category_filter)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows_to_results(rows))
    }


    // ════════════════════════════════════════════════════════════
    //  SNIPPET EXTRACTION
    //  Sliding window cari bagian content paling relevan dengan query.
    //  Diadaptasi dari program Kitab — term matching + phrase bonus.
    // ════════════════════════════════════════════════════════════

    /// Ekstrak snippet terbaik dari content berdasarkan query terms.
    /// Memecah content menjadi window ~200 kata, cari window dengan
    /// skor term-match tertinggi, truncate di batas kalimat.
    pub fn extract_snippet(content: &str, query_terms: &[&str], max_chars: usize) -> String {
        if content.is_empty() {
            return String::new();
        }

        // Jika tidak ada terms atau content pendek, langsung truncate
        if query_terms.is_empty() || content.chars().count() <= max_chars {
            return truncate_at_sentence(content, max_chars);
        }

        // Pecah content menjadi kata-kata, lalu buat sliding windows
        let words: Vec<&str> = content.split_whitespace().collect();
        let window_words = 60usize; // ~300-400 karakter per window
        let step = 20usize;

        if words.len() <= window_words {
            return truncate_at_sentence(content, max_chars);
        }

        let mut best_score = 0usize;
        let mut best_start = 0usize;

        let mut start = 0;
        while start + window_words <= words.len() {
            let window_text = words[start..start + window_words].join(" ");
            let normalized = window_text.to_lowercase();

            let score: usize = query_terms.iter().map(|term| {
                let t = term.to_lowercase();
                // Phrase match (spasi) → 3×, single term → 1×
                if t.contains(' ') {
                    if normalized.contains(&t) { 3 } else { 0 }
                } else {
                    // Hitung frekuensi kemunculan
                    let count = normalized.matches(t.as_str()).count();
                    count.min(3) // cap agar satu term tidak mendominasi
                }
            }).sum();

            if score > best_score {
                best_score = score;
                best_start = start;
            }

            start += step;
        }

        // Ambil window terbaik + sedikit konteks setelahnya
        let end = (best_start + window_words + 20).min(words.len());
        let snippet_raw = words[best_start..end].join(" ");

        truncate_at_sentence(&snippet_raw, max_chars)
    }

    /// Util: truncate di batas kalimat terdekat sebelum max_chars
    fn truncate_snippet(content: &str, max_chars: usize) -> String {
        truncate_at_sentence(content, max_chars)
    }


    // ════════════════════════════════════════════════════════════
    //  QUERY LOGGING
    // ════════════════════════════════════════════════════════════

    pub async fn log_query(
        &self,
        query_text:        &str,
        detected_language: &str,
        detected_domain:   &str,
        num_results:       i32,
        top_score:         f32,
        search_time_ms:    i64,
        session_id:        Option<&str>,
        user_id:           Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO query_logs
                (query_text, detected_language, detected_domain,
                 num_results, top_score, search_time_ms, session_id, user_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(query_text)
        .bind(detected_language)
        .bind(detected_domain)
        .bind(num_results)
        .bind(top_score)
        .bind(search_time_ms)
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_query_log_stats(&self) -> Result<QueryLogStats> {
        let total_queries: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM query_logs"
        ).fetch_one(&self.pool).await?;

        let unique_queries: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT query_text) FROM query_logs"
        ).fetch_one(&self.pool).await?;

        let avg_results: f64 = sqlx::query_scalar(
            "SELECT COALESCE(AVG(num_results), 0)::float8 FROM query_logs"
        ).fetch_one(&self.pool).await?;

        let avg_search_time_ms: f64 = sqlx::query_scalar(
            "SELECT COALESCE(AVG(search_time_ms), 0)::float8 FROM query_logs"
        ).fetch_one(&self.pool).await?;

        let zero_result_queries: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM query_logs WHERE num_results = 0"
        ).fetch_one(&self.pool).await?;

        let language_distribution: Vec<(String, i64)> =
            sqlx::query("SELECT detected_language, COUNT(*) as cnt FROM query_logs GROUP BY detected_language ORDER BY cnt DESC")
                .fetch_all(&self.pool).await?
                .iter()
                .map(|r| (r.get::<String, _>("detected_language"), r.get::<i64, _>("cnt")))
                .collect();

        let domain_distribution: Vec<(String, i64)> =
            sqlx::query("SELECT detected_domain, COUNT(*) as cnt FROM query_logs GROUP BY detected_domain ORDER BY cnt DESC")
                .fetch_all(&self.pool).await?
                .iter()
                .map(|r| (r.get::<String, _>("detected_domain"), r.get::<i64, _>("cnt")))
                .collect();

        let queries_per_day: Vec<(String, i64)> =
            sqlx::query("SELECT DATE(created_at)::text as day, COUNT(*) as cnt FROM query_logs WHERE created_at >= NOW() - INTERVAL '30 days' GROUP BY day ORDER BY day DESC")
                .fetch_all(&self.pool).await?
                .iter()
                .map(|r| (r.get::<String, _>("day"), r.get::<i64, _>("cnt")))
                .collect();

        let top_queries: Vec<(String, i64)> =
            sqlx::query("SELECT query_text, COUNT(*) as cnt FROM query_logs GROUP BY query_text ORDER BY cnt DESC LIMIT 20")
                .fetch_all(&self.pool).await?
                .iter()
                .map(|r| (r.get::<String, _>("query_text"), r.get::<i64, _>("cnt")))
                .collect();

        Ok(QueryLogStats {
            total_queries,
            unique_queries,
            avg_results,
            avg_search_time_ms,
            zero_result_queries,
            language_distribution,
            domain_distribution,
            queries_per_day,
            top_queries,
        })
    }

    pub async fn cleanup_old_query_logs(&self, days: i64) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM query_logs WHERE created_at < NOW() - ($1::bigint * INTERVAL '1 day')"
        )
        .bind(days)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }


    // ════════════════════════════════════════════════════════════
    //  USER MANAGEMENT
    // ════════════════════════════════════════════════════════════

    pub async fn create_user(
        &self,
        email:         &str,
        password_hash: &str,
        display_name:  &str,
    ) -> Result<User, String> {
        let row = sqlx::query(
            "INSERT INTO users (email, password_hash, display_name, role, is_active, email_verified)
             VALUES ($1, $2, $3, 'user', FALSE, FALSE)
             RETURNING id, email, password_hash, display_name, role, is_active,
                       created_at::text, updated_at::text",
        )
        .bind(email)
        .bind(password_hash)
        .bind(display_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                "Email sudah terdaftar".to_string()
            } else {
                format!("Database error: {}", e)
            }
        })?;

        Ok(row_to_user(&row))
    }

    /// Buat user dengan role tertentu (digunakan admin)
    pub async fn create_user_with_role(
        &self,
        email:         &str,
        password_hash: &str,
        display_name:  &str,
        role:          &str,
    ) -> Result<User, String> {
        let safe_role = if role == "admin" { "admin" } else { "user" };
        let row = sqlx::query(
            "INSERT INTO users (email, password_hash, display_name, role, is_active, email_verified)
             VALUES ($1, $2, $3, $4, TRUE, TRUE)
             RETURNING id, email, password_hash, display_name, role, is_active,
                       created_at::text, updated_at::text",
        )
        .bind(email)
        .bind(password_hash)
        .bind(if display_name.is_empty() { email } else { display_name })
        .bind(safe_role)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                "Email sudah terdaftar".to_string()
            } else {
                format!("Database error: {}", e)
            }
        })?;

        Ok(row_to_user(&row))
    }

    // ════════════════════════════════════════════════════════════
    //  EMAIL VERIFICATION
    // ════════════════════════════════════════════════════════════

    pub async fn create_verification_token(
        &self,
        user_id: i64,
        token:   &str,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO email_verifications (user_id, token, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '24 hours')",
        )
        .bind(user_id)
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn verify_email_token(&self, token: &str) -> Result<Option<i64>, String> {
        // Cek token valid dan belum expired/used
        let row = sqlx::query(
            "SELECT id, user_id FROM email_verifications
             WHERE token = $1
               AND expires_at > NOW()
               AND used_at IS NULL",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let verif_id: i64 = row.get("id");
        let user_id:  i64 = row.get("user_id");

        // Tandai token sebagai used
        sqlx::query("UPDATE email_verifications SET used_at = NOW() WHERE id = $1")
            .bind(verif_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;

        // Aktifkan user
        sqlx::query(
            "UPDATE users SET is_active = TRUE, email_verified = TRUE, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(Some(user_id))
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, String> {
        let row = sqlx::query(
            "SELECT id, email, password_hash, display_name, role, is_active, email_verified,
                    created_at::text, updated_at::text
             FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(row.map(|r| row_to_user(&r)))
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> Result<Option<User>, String> {
        let row = sqlx::query(
            "SELECT id, email, password_hash, display_name, role, is_active, email_verified,
                    created_at::text, updated_at::text
             FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(row.map(|r| row_to_user(&r)))
    }

    pub async fn update_user_profile(
        &self,
        user_id:      i64,
        display_name: Option<&str>,
        email:        Option<&str>,
    ) -> Result<(), String> {
        if let Some(name) = display_name {
            sqlx::query(
                "UPDATE users SET display_name = $1, updated_at = NOW() WHERE id = $2"
            )
            .bind(name)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        }

        if let Some(new_email) = email {
            sqlx::query(
                "UPDATE users SET email = $1, updated_at = NOW() WHERE id = $2"
            )
            .bind(new_email)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                    "Email sudah digunakan".to_string()
                } else {
                    format!("Database error: {}", e)
                }
            })?;
        }

        Ok(())
    }

    pub async fn update_user_password(
        &self,
        user_id:          i64,
        new_password_hash: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2"
        )
        .bind(new_password_hash)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ════════════════════════════════════════════════════════════
    //  PASSWORD RESET
    // ════════════════════════════════════════════════════════════

    pub async fn create_password_reset_token(
        &self,
        user_id: i64,
        token:   &str,
    ) -> Result<(), String> {
        // Hapus token lama yang belum dipakai
        sqlx::query("DELETE FROM password_resets WHERE user_id = $1 AND used_at IS NULL")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO password_resets (user_id, token, expires_at)
             VALUES ($1, $2, NOW() + INTERVAL '1 hour')",
        )
        .bind(user_id)
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn consume_password_reset_token(
        &self,
        token: &str,
    ) -> Result<Option<i64>, String> {
        let row = sqlx::query(
            "SELECT id, user_id FROM password_resets
             WHERE token = $1
               AND expires_at > NOW()
               AND used_at IS NULL",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let reset_id: i64 = row.get("id");
        let user_id:  i64 = row.get("user_id");

        sqlx::query("UPDATE password_resets SET used_at = NOW() WHERE id = $1")
            .bind(reset_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;

        Ok(Some(user_id))
    }

    pub async fn deactivate_user(&self, user_id: i64) -> Result<(), String> {
        sqlx::query(
            "UPDATE users SET is_active = FALSE, updated_at = NOW() WHERE id = $1"
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }


    // ════════════════════════════════════════════════════════════
    //  CHAT SESSIONS
    // ════════════════════════════════════════════════════════════

    pub async fn save_session(&self, session: &ChatSession) -> Result<(), String> {
        let messages_json = serde_json::to_value(&session.messages)
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO chat_sessions (id, user_id, title, messages, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5::timestamptz, $6::timestamptz)
             ON CONFLICT (id) DO UPDATE
             SET title = EXCLUDED.title,
                 messages = EXCLUDED.messages,
                 updated_at = EXCLUDED.updated_at::timestamptz",
        )
        .bind(&session.id)
        .bind(session.user_id)
        .bind(&session.title)
        .bind(&messages_json)
        .bind(&session.created_at)
        .bind(&session.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn get_session(
        &self,
        session_id: &str,
        user_id:    i64,
    ) -> Result<Option<ChatSession>, String> {
        let row = sqlx::query(
            "SELECT id, user_id, title, messages::text,
                    created_at::text, updated_at::text
             FROM chat_sessions WHERE id = $1 AND user_id = $2",
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(row.map(|r| {
            let messages_str: String = r.get("messages");
            let messages: Vec<ChatMessage> =
                serde_json::from_str(&messages_str).unwrap_or_default();
            ChatSession {
                id:         r.get("id"),
                user_id:    r.get("user_id"),
                title:      r.get("title"),
                messages,
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            }
        }))
    }

    pub async fn get_user_sessions(
        &self,
        user_id: i64,
    ) -> Result<Vec<SessionListItem>, String> {
        let rows = sqlx::query(
            "SELECT id, title, messages::text, updated_at::text
             FROM chat_sessions
             WHERE user_id = $1
             ORDER BY updated_at DESC LIMIT 50",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(rows.iter().map(|r| {
            let messages_str: String = r.get("messages");
            let messages: Vec<ChatMessage> =
                serde_json::from_str(&messages_str).unwrap_or_default();
            SessionListItem {
                id:            r.get("id"),
                title:         r.get("title"),
                updated_at:    r.get("updated_at"),
                message_count: messages.len(),
            }
        }).collect())
    }

    pub async fn delete_session(
        &self,
        session_id: &str,
        user_id:    i64,
    ) -> Result<bool, String> {
        let res = sqlx::query(
            "DELETE FROM chat_sessions WHERE id = $1 AND user_id = $2"
        )
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn delete_sessions_batch(
        &self,
        session_ids: &[String],
        user_id:     i64,
    ) -> Result<usize, String> {
        // ANY($1) lebih efisien daripada loop
        let res = sqlx::query(
            "DELETE FROM chat_sessions WHERE id = ANY($1) AND user_id = $2"
        )
        .bind(session_ids)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(res.rows_affected() as usize)
    }

    pub async fn rename_session(
        &self,
        session_id: &str,
        user_id:    i64,
        title:      &str,
    ) -> Result<bool, String> {
        let res = sqlx::query(
            "UPDATE chat_sessions
             SET title = $1, updated_at = NOW()
             WHERE id = $2 AND user_id = $3",
        )
        .bind(title)
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn cleanup_old_sessions(&self, days: i64) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM chat_sessions
             WHERE updated_at < NOW() - ($1::bigint * INTERVAL '1 day')"
        )
        .bind(days)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }


    // ════════════════════════════════════════════════════════════
    //  API KEYS
    // ════════════════════════════════════════════════════════════

    /// Buat API key baru. key_prefix = 8 char awal key (untuk lookup),
    /// key_hash = argon2/bcrypt hash dari full key (untuk verifikasi).
    pub async fn create_api_key(
        &self,
        user_id:     i64,
        key_prefix:  &str,
        key_hash:    &str,
        name:        &str,
        permissions: &[&str],
        rate_limit:  i32,
        expires_at:  Option<&str>,
    ) -> Result<i64, String> {
        let perms = serde_json::to_value(permissions)
            .map_err(|e| e.to_string())?;

        let row = sqlx::query_scalar::<_, i64>(
            "INSERT INTO api_keys
                (user_id, key_prefix, key_hash, name, permissions, rate_limit, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)
             RETURNING id",
        )
        .bind(user_id)
        .bind(key_prefix)
        .bind(key_hash)
        .bind(name)
        .bind(&perms)
        .bind(rate_limit)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| format!("Gagal membuat API key: {}", e))?;

        Ok(row)
    }

    pub async fn get_api_keys(&self, user_id: i64) -> Result<Vec<ApiKeyInfo>, String> {
        let rows = sqlx::query(
            "SELECT id, key_prefix, name, permissions::text, rate_limit, is_active,
                    last_used_at::text, created_at::text, expires_at::text
             FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(rows.iter().map(|r| {
            let perms_str: String = r.get("permissions");
            let permissions: Vec<String> =
                serde_json::from_str(&perms_str).unwrap_or_default();
            ApiKeyInfo {
                id:           r.get("id"),
                key_prefix:   r.get("key_prefix"),
                name:         r.get("name"),
                permissions,
                rate_limit:   r.get::<i32, _>("rate_limit") as i64,
                is_active:    r.get("is_active"),
                last_used_at: r.try_get("last_used_at").ok().flatten(),
                created_at:   r.get("created_at"),
                expires_at:   r.try_get("expires_at").ok().flatten(),
            }
        }).collect())
    }

    /// Lookup berdasarkan prefix, kembalikan data untuk verifikasi hash.
    /// Returns: (key_id, user_id, key_hash, permissions_json, rate_limit)
    pub async fn lookup_api_key(
        &self,
        key_prefix: &str,
    ) -> Result<Option<(i64, i64, String, String, i32)>, String> {
        let row = sqlx::query(
            "SELECT id, user_id, key_hash, permissions::text, rate_limit, expires_at
             FROM api_keys
             WHERE key_prefix = $1 AND is_active = TRUE",
        )
        .bind(key_prefix)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let Some(r) = row else { return Ok(None) };

        // Cek expiry
        let expires: Option<String> = r.try_get("expires_at").ok().flatten();
        if let Some(exp_str) = expires {
            if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&exp_str) {
                if exp < Utc::now() {
                    return Ok(None); // sudah expired
                }
            }
        }

        Ok(Some((
            r.get("id"),
            r.get("user_id"),
            r.get("key_hash"),
            r.get("permissions"),
            r.get("rate_limit"),
        )))
    }

    pub async fn touch_api_key(&self, key_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE api_keys SET last_used_at = NOW() WHERE id = $1"
        )
        .bind(key_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn revoke_api_key(&self, key_id: i64, user_id: i64) -> Result<bool, String> {
        let res = sqlx::query(
            "UPDATE api_keys SET is_active = FALSE WHERE id = $1 AND user_id = $2"
        )
        .bind(key_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(res.rows_affected() > 0)
    }


    // ════════════════════════════════════════════════════════════
    //  UTILITIES
    // ════════════════════════════════════════════════════════════

    pub async fn document_exists(&self, file_hash: &str) -> Result<bool, sqlx::Error> {
        let row = sqlx::query(
            "SELECT 1 FROM documents WHERE file_hash = $1 LIMIT 1"
        )
        .bind(file_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn list_categories(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT DISTINCT category FROM documents ORDER BY category"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("category")).collect())
    }
}


// ══════════════════════════════════════════════════════════════════
//  PRIVATE HELPERS
// ══════════════════════════════════════════════════════════════════

/// Question and function words that make keyword search fail when every word must match.
/// Keep in sync with STOPWORDS in scripts/eval_retrieval.py.
const KEYWORD_STOPWORDS: &[&str] = &[
    "apa", "apakah", "siapa", "bagaimana", "berapa", "kapan", "mana", "dimana", "di", "ke",
    "dari", "dan", "atau", "yang", "untuk", "dengan", "pada", "dalam", "ini", "itu", "adalah",
    "ada", "saja", "sudah", "belum", "bisa", "dapat", "harus", "akan", "juga", "saya", "kamu",
    "kami", "mohon", "tolong", "sebutkan", "jelaskan", "semua", "beserta", "tentang", "serta",
    "jika", "the", "a", "an", "of", "to", "in", "on", "for", "and", "or", "is", "are", "do",
    "does", "did", "how", "what", "who", "which", "when", "where", "can", "i", "my", "me",
    "with", "about",
];

/// Turn a free-form question into an OR tsquery ("dosen | pembimbing | tesis").
/// Only ASCII letters/digits survive, so the result is safe to pass to `to_tsquery`.
/// `None` when no words are left.
fn keyword_tsquery(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mut terms: Vec<&str> = Vec::new();
    for word in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.len() > 1 && !KEYWORD_STOPWORDS.contains(&word) && !terms.contains(&word) {
            terms.push(word);
        }
    }
    if terms.is_empty() { None } else { Some(terms.join(" | ")) }
}

#[cfg(test)]
mod tests {
    use super::keyword_tsquery;

    #[test]
    fn keyword_tsquery_drops_question_words_and_joins_with_or() {
        assert_eq!(keyword_tsquery("Bagaimana alur ujian tesis?").as_deref(), Some("alur | ujian | tesis"));
        assert_eq!(keyword_tsquery("berapa jumlah kerjasama PMPSTI?").as_deref(), Some("jumlah | kerjasama | pmpsti"));
    }

    #[test]
    fn keyword_tsquery_strips_operators_and_duplicates() {
        assert_eq!(keyword_tsquery("tesis & (tesis) | !'drop'").as_deref(), Some("tesis | drop"));
        assert_eq!(keyword_tsquery("apa?"), None);
        assert_eq!(keyword_tsquery("   "), None);
    }
}

fn rows_to_results(rows: Vec<sqlx::postgres::PgRow>) -> Vec<SearchResult> {
    rows.into_iter()
        .map(|row| {
            let content: String = row.get("content");
            SearchResult {
                document_id: row.try_get("document_id").unwrap_or_default(),
                title:       row.get("title"),
                snippet:     {
                    let cleaned = clean_snippet(&content);
                    if cleaned.is_empty() { cleaned }
                    else { truncate_at_sentence(&cleaned, 400) }
                },
                content,
                category:    row.get("category"),
                subcategory: row.get("subcategory"),
                source_url:  row.get("source_url"),
                page_number: row.try_get("page_number").ok().flatten(),
                chunk_index: row.try_get("chunk_index").ok().flatten(),
                score:       row.try_get("score").unwrap_or(0.0),
            }
        })
        .collect()
}

fn row_to_user(row: &sqlx::postgres::PgRow) -> User {
    User {
        id:             row.get("id"),
        email:          row.get("email"),
        password_hash:  row.get("password_hash"),
        display_name:   row.get("display_name"),
        role:           row.get("role"),
        is_active:      row.get("is_active"),
        email_verified: row.try_get("email_verified").unwrap_or(false),
        created_at:     row.get("created_at"),
        updated_at:     row.get("updated_at"),
    }
}

/// Truncate string di batas kalimat (. ! ?) terdekat sebelum max_chars.
/// Fallback ke hard cut jika tidak ada batas kalimat.
/// Bersihkan snippet dari konten JS/CSS yang tidak berguna sebelum ditampilkan ke user.
/// Jika konten terdeteksi sebagai JS/CSS (bukan teks natural), kembalikan string kosong.
fn clean_snippet(text: &str) -> String {
    let trimmed = text.trim();
    // Deteksi pola JS/CSS yang khas dari Quarto/Bootstrap
    let js_indicators = [
        "const ", "function ", "var ", "let ", "=>", "document.querySelector",
        "window.", "classList.", "getAttribute(", "getElementById",
        "addEventListener(", "toggleBodyColor", "bsSheetEl",
    ];
    let first_200: String = trimmed.chars().take(200).collect();
    let is_js = js_indicators.iter().any(|pat| first_200.contains(pat));
    if is_js {
        return String::new(); // kosongkan — jangan tampilkan JS ke user
    }
    trimmed.to_string()
}

fn truncate_at_sentence(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    // Ambil max_chars karakter dulu
    let truncated: String = text.chars().take(max_chars).collect();

    // Cari batas kalimat terakhir (., !, ?, \n) di 40% akhir window
    let search_from = (max_chars as f32 * 0.6) as usize;
    let search_zone: String = truncated.chars().skip(search_from).collect();

    if let Some(pos) = search_zone.rfind(|c: char| matches!(c, '.' | '!' | '?' | '\n')) {
        let cut = search_from + pos + 1;
        let result: String = truncated.chars().take(cut).collect();
        return result.trim_end().to_string();
    }

    // Fallback: cut di batas kata terakhir
    if let Some(pos) = truncated.rfind(' ') {
        return format!("{}...", &truncated[..pos]);
    }

    format!("{}...", truncated)
}

// ══════════════════════════════════════════════════════════════════
//  ADMIN MODELS
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Serialize)]
pub struct AdminUser {
    pub id:           i64,
    pub email:        String,
    pub display_name: String,
    pub role:         String,
    pub is_active:    bool,
    pub created_at:   String,
}

#[derive(Debug, serde::Serialize)]
pub struct AdminSession {
    pub id:            String,
    pub user_id:       i64,
    pub user_email:    String,
    pub title:         String,
    pub message_count: i64,
    pub updated_at:    String,
}

// ── Struct untuk ingest dokumen baru ──────────────────────────
#[derive(Debug, Clone)]
pub struct DocumentChunk {
    pub document_id:   String,
    pub title:         String,
    pub content:       String,
    pub source_url:    String,
    pub category:      String,
    pub subcategory:   String,
    pub document_type: String,
    pub chunk_index:   i32,
    pub embedding:     Vec<f32>,
}

#[derive(Debug, serde::Serialize)]
pub struct AdminDocument {
    pub document_id:   String,
    pub title:         String,
    pub document_type: String,
    pub category:      String,
    pub subcategory:   String,
    pub source_url:    String,
    pub chunk_count:   i64,
}

impl Database {
    // ════════════════════════════════════════════════════════════
    //  ADMIN — USER MANAGEMENT
    // ════════════════════════════════════════════════════════════

    pub async fn admin_list_users(&self) -> Result<Vec<AdminUser>, String> {
        let rows = sqlx::query(
            "SELECT id, email, COALESCE(display_name,'') as display_name,
                    COALESCE(role,'user') as role, is_active, created_at::text
             FROM users ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(rows.iter().map(|r| AdminUser {
            id:           r.get("id"),
            email:        r.get("email"),
            display_name: r.get("display_name"),
            role:         r.get("role"),
            is_active:    r.get("is_active"),
            created_at:   r.get("created_at"),
        }).collect())
    }

    pub async fn admin_set_role(&self, user_id: i64, role: &str) -> Result<(), String> {
        sqlx::query("UPDATE users SET role = $1, updated_at = NOW() WHERE id = $2")
            .bind(role)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn admin_set_active(&self, user_id: i64, active: bool) -> Result<(), String> {
        sqlx::query("UPDATE users SET is_active = $1, updated_at = NOW() WHERE id = $2")
            .bind(active)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Admin approve: set email_verified = true DAN is_active = true sekaligus.
    /// Digunakan saat user tidak menerima email verifikasi dan butuh manual approval.
    pub async fn admin_verify_user(&self, user_id: i64) -> Result<(), String> {
        sqlx::query(
            "UPDATE users SET email_verified = TRUE, is_active = TRUE, updated_at = NOW() WHERE id = $1"
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn admin_delete_user(&self, user_id: i64) -> Result<(), String> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ════════════════════════════════════════════════════════════
    //  ADMIN — SESSIONS
    // ════════════════════════════════════════════════════════════

    pub async fn admin_list_sessions(&self) -> Result<Vec<AdminSession>, String> {
        let rows = sqlx::query(
            "SELECT s.id,
                    s.user_id,
                    COALESCE(u.email, '') as user_email,
                    COALESCE(s.title, '') as title,
                    COALESCE(
                        jsonb_array_length(
                            CASE jsonb_typeof(s.messages::jsonb)
                                WHEN 'array' THEN s.messages::jsonb
                                ELSE '[]'::jsonb
                            END
                        ), 0
                    )::bigint as message_count,
                    COALESCE(s.updated_at::text, '') as updated_at
             FROM chat_sessions s
             LEFT JOIN users u ON u.id = s.user_id
             ORDER BY s.updated_at DESC NULLS LAST
             LIMIT 200"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(rows.iter().map(|r| AdminSession {
            id:            r.get("id"),
            user_id:       r.get("user_id"),
            user_email:    r.get("user_email"),
            title:         r.get("title"),
            message_count: r.get("message_count"),
            updated_at:    r.get("updated_at"),
        }).collect())
    }

    pub async fn admin_delete_session(&self, session_id: &str) -> Result<bool, String> {
        let res = sqlx::query("DELETE FROM chat_sessions WHERE id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.rows_affected() > 0)
    }

    // ════════════════════════════════════════════════════════════
    //  ADMIN — DOCUMENTS
    // ════════════════════════════════════════════════════════════

    // ════════════════════════════════════════════════════════════
    //  DOCUMENT INGEST — insert chunks dengan embedding
    // ════════════════════════════════════════════════════════════

    pub async fn insert_document_chunks(&self, chunks: &[DocumentChunk]) -> Result<usize, String> {
        let mut inserted = 0usize;
        for chunk in chunks {
            // Kalau embedding kosong (mis. quota Gemini habis), simpan NULL agar
            // pgvector tidak error. Chunk tetap bisa dicari via fulltext search.
            let vec: Option<pgvector::Vector> = if chunk.embedding.is_empty() {
                None
            } else {
                Some(pgvector::Vector::from(chunk.embedding.clone()))
            };
            sqlx::query(
                "INSERT INTO documents
                    (document_id, title, content, source_url, category, subcategory,
                     document_type, chunk_index, embedding, created_at, updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,NOW(),NOW())
                 ON CONFLICT (document_id, chunk_index) DO UPDATE
                 SET content=EXCLUDED.content, embedding=EXCLUDED.embedding,
                     title=EXCLUDED.title, updated_at=NOW()"
            )
            .bind(&chunk.document_id)
            .bind(&chunk.title)
            .bind(&chunk.content)
            .bind(&chunk.source_url)
            .bind(&chunk.category)
            .bind(&chunk.subcategory)
            .bind(&chunk.document_type)
            .bind(chunk.chunk_index)
            .bind(vec)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            inserted += 1;
        }
        Ok(inserted)
    }

    pub async fn admin_list_documents(&self) -> Result<Vec<AdminDocument>, String> {
        let rows = sqlx::query(
            "SELECT document_id,
                    COALESCE(MIN(title),'') as title,
                    COALESCE(MIN(document_type),'') as document_type,
                    COALESCE(MIN(category),'') as category,
                    COALESCE(MIN(subcategory),'') as subcategory,
                    COALESCE(MIN(source_url),'') as source_url,
                    COUNT(*) as chunk_count
             FROM documents
             GROUP BY document_id
             ORDER BY MIN(category), document_id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        Ok(rows.iter().map(|r| AdminDocument {
            document_id:   r.get("document_id"),
            title:         r.get("title"),
            document_type: r.get("document_type"),
            category:      r.get("category"),
            subcategory:   r.get("subcategory"),
            source_url:    r.get("source_url"),
            chunk_count:   r.get("chunk_count"),
        }).collect())
    }

    pub async fn admin_delete_document(&self, document_id: &str) -> Result<u64, String> {
        let res = sqlx::query("DELETE FROM documents WHERE document_id = $1")
            .bind(document_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.rows_affected())
    }

    pub async fn admin_update_document(
        &self,
        document_id: &str,
        title: &str,
        category: &str,
        subcategory: &str,
    ) -> Result<u64, String> {
        let res = sqlx::query(
            "UPDATE documents SET title = $1, category = $2, subcategory = $3
             WHERE document_id = $4"
        )
        .bind(title)
        .bind(category)
        .bind(subcategory)
        .bind(document_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(res.rows_affected())
    }

    // ── Chunk-level operations ────────────────────────────────────────────────

    pub async fn admin_list_chunks(&self, document_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let rows = sqlx::query(
            "SELECT id::bigint AS id, chunk_index, content, title, category, subcategory, source_url, created_at::text
             FROM documents WHERE document_id = $1 ORDER BY chunk_index ASC"
        )
        .bind(document_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let chunks = rows.iter().map(|r| serde_json::json!({
            // id is SERIAL (int4) when the backend creates the table, but bigint on Neon (created
            // by an export script); the query casts to bigint so both decode. A type mismatch in
            // `get` panics, which dropped the connection (HTTP 500 when opening a document's chunks).
            "id":          r.get::<i64, _>("id"),
            "chunk_index": r.get::<Option<i32>, _>("chunk_index").unwrap_or(0),
            "content":     r.get::<Option<String>, _>("content").unwrap_or_default(),
            "title":       r.get::<Option<String>, _>("title").unwrap_or_default(),
            "category":    r.get::<Option<String>, _>("category").unwrap_or_default(),
            "subcategory": r.get::<Option<String>, _>("subcategory").unwrap_or_default(),
            "source_url":  r.get::<Option<String>, _>("source_url").unwrap_or_default(),
            "created_at":  r.get::<Option<String>, _>("created_at").unwrap_or_default(),
        })).collect();
        Ok(chunks)
    }

    // Fetch ALL chunks for a list of document_ids (for document expansion in RAG)
    pub async fn fetch_all_chunks_for_documents(
        &self,
        document_ids: &[String],
    ) -> Result<Vec<SearchResult>, sqlx::Error> {
        if document_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query(
            "SELECT document_id, title, content, category, subcategory,
                    COALESCE(source_url, '') AS source_url,
                    page_number, chunk_index, 1.0::float8 AS score
             FROM documents
             WHERE document_id = ANY($1::text[])
             ORDER BY document_id, chunk_index"
        )
        .bind(document_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows_to_results(rows))
    }

    pub async fn admin_delete_chunk(&self, chunk_id: i32) -> Result<u64, String> {
        let res = sqlx::query("DELETE FROM documents WHERE id = $1")
            .bind(chunk_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.rows_affected())
    }

    pub async fn admin_update_chunk(&self, chunk_id: i32, content: &str) -> Result<(), String> {
        sqlx::query(
            "UPDATE documents SET content = $1, updated_at = NOW() WHERE id = $2"
        )
        .bind(content)
        .bind(chunk_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ════════════════════════════════════════════════════════════
    //  GUEST QUOTA — public chat tanpa auth (maks 5 pertanyaan)
    // ════════════════════════════════════════════════════════════

    /// Return ask_count for given guest_token (0 if new)
    pub async fn guest_ask_count(&self, token: &str) -> i32 {
        let row = sqlx::query(
            "SELECT ask_count FROM guest_quotas WHERE guest_token = $1"
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await;

        match row {
            Ok(Some(r)) => r.get::<i32, _>("ask_count"),
            _ => 0,
        }
    }

    /// Increment ask_count. Returns new count.
    pub async fn guest_increment(&self, token: &str) -> i32 {
        let row = sqlx::query(
            "INSERT INTO guest_quotas (guest_token, ask_count, last_ask_at)
             VALUES ($1, 1, NOW())
             ON CONFLICT (guest_token)
             DO UPDATE SET ask_count = guest_quotas.ask_count + 1,
                           last_ask_at = NOW()
             RETURNING ask_count"
        )
        .bind(token)
        .fetch_one(&self.pool)
        .await;

        match row {
            Ok(r) => r.get::<i32, _>("ask_count"),
            Err(_) => 0,
        }
    }

    // ── FAQ ────────────────────────────────────────────────────────────────────

    /// Ambil FAQ untuk ditampilkan di halaman publik:
    /// 1. Semua pinned questions (admin-pilih), diurutkan sort_order
    /// 2. Top queries dari query_logs yang belum ada di pinned, length 20-150 char,
    ///    sampai total `limit` pertanyaan.
    pub async fn faq_public(&self, limit: i64) -> Result<Vec<String>, String> {
        // Pinned questions
        let pinned: Vec<String> = sqlx::query(
            "SELECT question FROM faq_pins ORDER BY sort_order ASC, created_at ASC LIMIT $1"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| r.get::<String, _>("question"))
        .collect();

        let remaining = limit - pinned.len() as i64;
        let mut result = pinned.clone();

        if remaining > 0 {
            // Top queries dari query_logs, exclude yang sudah di pinned
            let pinned_arr: Vec<&str> = pinned.iter().map(|s| s.as_str()).collect();
            let top: Vec<String> = sqlx::query(
                r#"SELECT query_text
                   FROM query_logs
                   WHERE LENGTH(query_text) BETWEEN 15 AND 200
                     AND query_text NOT IN (SELECT question FROM faq_pins)
                   GROUP BY query_text
                   ORDER BY COUNT(*) DESC, MAX(created_at) DESC
                   LIMIT $1"#
            )
            .bind(remaining)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| r.get::<String, _>("query_text"))
            .filter(|q| !pinned_arr.contains(&q.as_str()))
            .collect();

            result.extend(top);
        }

        Ok(result)
    }

    /// List semua pinned FAQ (untuk admin)
    pub async fn faq_list_pins(&self) -> Result<Vec<FaqPin>, String> {
        sqlx::query_as::<_, FaqPin>(
            "SELECT id, question, sort_order, created_at::TEXT AS created_at FROM faq_pins ORDER BY sort_order ASC, created_at ASC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())
    }

    /// Pin sebuah pertanyaan ke FAQ
    pub async fn faq_pin(&self, question: &str, admin_id: i64) -> Result<i64, String> {
        let row = sqlx::query(
            "INSERT INTO faq_pins (question, pinned_by, sort_order)
             VALUES ($1, $2, (SELECT COALESCE(MAX(sort_order),0)+1 FROM faq_pins))
             ON CONFLICT (question) DO UPDATE SET sort_order = faq_pins.sort_order
             RETURNING id"
        )
        .bind(question)
        .bind(admin_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(row.get::<i64, _>("id"))
    }

    /// Unpin pertanyaan dari FAQ
    pub async fn faq_unpin(&self, pin_id: i64) -> Result<(), String> {
        sqlx::query("DELETE FROM faq_pins WHERE id = $1")
            .bind(pin_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Top queries dari query_logs (untuk admin pilih yang mau di-pin)
    pub async fn faq_top_queries(&self, limit: i64) -> Result<Vec<TopQuery>, String> {
        sqlx::query_as::<_, TopQuery>(
            r#"SELECT query_text, COUNT(*)::BIGINT AS count, MAX(created_at)::TEXT AS last_asked
               FROM query_logs
               WHERE LENGTH(query_text) BETWEEN 15 AND 200
               GROUP BY query_text
               ORDER BY count DESC, MAX(created_at) DESC
               LIMIT $1"#
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())
    }

}

// ══════════════════════════════════════════════════════════════════
//  ADMIN QUERY LOGS
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct FaqPin {
    pub id:         i64,
    pub question:   String,
    pub sort_order: i32,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct TopQuery {
    pub query_text:  String,
    pub count:       i64,
    pub last_asked:  String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminQueryLog {
    pub id:              i64,
    pub query_text:      String,
    pub detected_language: String,
    pub num_results:     i32,
    pub search_time_ms:  i64,
    pub user_id:         Option<i64>,
    pub session_id:      Option<String>,
    pub created_at:      String,
}

impl Database {
    pub async fn admin_list_query_logs(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AdminQueryLog>, i64)> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM query_logs")
            .fetch_one(&self.pool).await?;

        let rows = sqlx::query(
            "SELECT id, query_text, detected_language, num_results, search_time_ms,
                    user_id, session_id, created_at::text
             FROM query_logs
             ORDER BY created_at DESC
             LIMIT $1 OFFSET $2"
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool).await?;

        let logs = rows.iter().map(|r| AdminQueryLog {
            id:               r.get("id"),
            query_text:       r.get("query_text"),
            detected_language: r.get("detected_language"),
            num_results:      r.get("num_results"),
            search_time_ms:   r.get("search_time_ms"),
            user_id:          r.try_get("user_id").ok(),
            session_id:       r.try_get("session_id").ok(),
            created_at:       r.get("created_at"),
        }).collect();

        Ok((logs, total))
    }
}

// ══════════════════════════════════════════════════════════════════
//  SEED ADMIN USER
// ══════════════════════════════════════════════════════════════════

/// Buat atau update admin user dari env ADMIN_EMAIL + ADMIN_PASSWORD.
/// Aman dipanggil berkali-kali (idempoten).
pub async fn seed_admin_user(db: &std::sync::Arc<Database>) {
    let email = match std::env::var("ADMIN_EMAIL") {
        Ok(e) if !e.is_empty() => e,
        _ => {
            log::info!("ADMIN_EMAIL tidak diset — skip seed admin.");
            return;
        }
    };
    let password = match std::env::var("ADMIN_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            log::warn!("ADMIN_EMAIL diset tapi ADMIN_PASSWORD kosong — skip seed admin.");
            return;
        }
    };

    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{SaltString, rand_core::OsRng};

    let salt = SaltString::generate(&mut OsRng);
    let hash = match Argon2::default().hash_password(password.as_bytes(), &salt) {
        Ok(h) => h.to_string(),
        Err(e) => {
            log::error!("Gagal hash password admin: {e}");
            return;
        }
    };

    // Upsert: insert jika belum ada, update password+role jika sudah ada
    let result = sqlx::query(
        "INSERT INTO users (email, password_hash, display_name, role, is_active, email_verified)
         VALUES ($1, $2, 'Administrator', 'admin', true, true)
         ON CONFLICT (email) DO UPDATE
           SET password_hash  = EXCLUDED.password_hash,
               role           = 'admin',
               email_verified = true,
               is_active      = true,
               updated_at     = NOW()"
    )
    .bind(&email)
    .bind(&hash)
    .execute(&db.pool)
    .await;

    match result {
        Ok(_)  => log::info!("Admin user OK: {}", email),
        Err(e) => log::error!("Gagal seed admin: {e}"),
    }
}
