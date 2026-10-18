use log::{info, warn};
use std::{env, sync::Arc};
use serde::{Deserialize, Serialize};
use reqwest::Client;
use tokio::time::{sleep, Duration};

use crate::db::{Database, SearchResult};

pub struct SearchEngine {
    pub db:      Arc<Database>,
    api_key:     String,
    http_client: Client,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbedRequest {
    content: EmbedContent,
    task_type: &'static str,
    output_dimensionality: u32,
}

#[derive(Serialize)]
struct EmbedContent {
    parts: Vec<EmbedPart>,
}

#[derive(Serialize)]
struct EmbedPart {
    text: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embedding: EmbedValues,
}

#[derive(Deserialize)]
struct EmbedValues {
    values: Vec<f32>,
}

impl SearchEngine {
    pub fn new(db: Arc<Database>) -> Result<Self, anyhow::Error> {
        let api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| anyhow::anyhow!("GEMINI_API_KEY tidak ditemukan"))?;
        info!("SearchEngine siap (Gemini gemini-embedding-001, 768-dim).");
        Ok(Self {
            db,
            api_key,
            http_client: Client::new(),
        })
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, anyhow::Error> {
        let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent";
        let body = EmbedRequest {
            content: EmbedContent {
                parts: vec![EmbedPart { text: query.to_string() }],
            },
            // Harus pasangan dari RETRIEVAL_DOCUMENT yang dipakai saat ingest
            task_type: "RETRIEVAL_QUERY",
            output_dimensionality: 768,
        };

        // Retry dengan exponential backoff untuk 429 (rate limit)
        let delays = [1u64, 2, 4]; // detik
        let mut last_err = None;
        for (attempt, &delay_secs) in delays.iter().enumerate() {
            let res = self.http_client
                .post(url)
                .header("x-goog-api-key", &self.api_key)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;

            match res {
                Ok(resp) if resp.status() == 429 => {
                    warn!("Gemini embedding 429 (attempt {}), tunggu {}s…", attempt + 1, delay_secs);
                    last_err = Some(anyhow::anyhow!("Gemini embedding rate limit (429)"));
                    sleep(Duration::from_secs(delay_secs)).await;
                }
                Ok(resp) => {
                    let resp = resp.error_for_status()?;
                    let data = resp.json::<EmbedResponse>().await?;
                    return Ok(data.embedding.values);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Embedding gagal setelah 3 percobaan")))
    }

    // ── Hybrid (default) — fallback ke fulltext jika embedding gagal ──
    pub async fn search(
        &self,
        query:           &str,
        limit:           i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, anyhow::Error> {
        match self.embed_query(query).await {
            Ok(embedding) => {
                let mut results = self.db.search_hybrid(query, embedding, limit, category_filter).await?;
                self.enrich_snippets(&mut results, query);
                Ok(results)
            }
            Err(e) => {
                warn!("Embedding gagal ({}), fallback ke fulltext search", e);
                let mut results = self.db.search_fulltext(query, limit, category_filter).await?;
                self.enrich_snippets(&mut results, query);
                Ok(results)
            }
        }
    }

    // ── Pure vector ──
    pub async fn search_semantic(
        &self,
        query:           &str,
        limit:           i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, anyhow::Error> {
        let embedding = self.embed_query(query).await?;
        let mut results = self.db.search_vector(embedding, limit, category_filter).await?;
        self.enrich_snippets(&mut results, query);
        Ok(results)
    }

    // ── Reciprocal Rank Fusion ──
    pub async fn search_rrf(
        &self,
        query:           &str,
        limit:           i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, anyhow::Error> {
        let embedding = self.embed_query(query).await?;
        let mut results = self.db.search_rrf(query, embedding, limit, category_filter).await?;
        self.enrich_snippets(&mut results, query);
        Ok(results)
    }

    // ── Pure fulltext ──
    pub async fn search_keyword(
        &self,
        query:           &str,
        limit:           i64,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, anyhow::Error> {
        let mut results = self.db.search_fulltext(query, limit, category_filter).await?;
        self.enrich_snippets(&mut results, query);
        Ok(results)
    }

    fn enrich_snippets(&self, results: &mut Vec<SearchResult>, query: &str) {
        let terms: Vec<&str> = query.split_whitespace().collect();
        for r in results.iter_mut() {
            r.snippet = Database::extract_snippet(&r.content, &terms, 400);
        }
    }

    /// Numbered context for the prompt ([1]..[N] = source cards); used by answer_with_chunks.
    pub fn format_context_refs(results: &[&SearchResult]) -> String {
        results
            .iter()
            .enumerate()
            .map(|(i, r)| Self::format_one(i, r))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    fn format_one(i: usize, r: &SearchResult) -> String {
        let location = match (r.pages.as_slice(), r.page_number) {
            ([], Some(p)) => format!("hal. {}", p),
            ([], None)    => "—".to_string(),
            (pages, _)    => format!("hal. {}", format_pages(pages)),
        };
        let version = if r.version.is_empty() { String::new() } else { format!("Versi: {}\n", r.version) };
        format!(
            "[{}] {} ({})\nKategori: {} / {}\nSumber: {}\n{}\n{}",
            i + 1,
            r.title,
            location,
            r.category,
            r.subcategory,
            r.source_url,
            version,
            r.content,
        )
    }
}

/// "3, 5–7, 10" — compact page list for citations.
pub fn format_pages(pages: &[i32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            i += 1;
            end = pages[i];
        }
        parts.push(if end > start { format!("{start}–{end}") } else { start.to_string() });
        i += 1;
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::format_pages;

    #[test]
    fn compact_page_ranges() {
        assert_eq!(format_pages(&[3, 5, 6, 7, 10]), "3, 5–7, 10");
        assert_eq!(format_pages(&[24]), "24");
        assert_eq!(format_pages(&[]), "");
    }
}

#[cfg(test)]
mod version_line_tests {
    use super::*;
    use crate::db::DocVersion;

    fn result(version: &str) -> SearchResult {
        SearchResult {
            document_id: "d".into(), title: "Kurikulum".into(), content: "Tesis 6 SKS".into(),
            snippet: String::new(), category: "akademik".into(), subcategory: String::new(),
            source_url: "https://x.id/k.html".into(), page_number: None, chunk_index: Some(0),
            score: 1.0, pages: vec![], version: version.into(),
        }
    }

    #[test]
    fn context_shows_the_version_line_only_when_known() {
        let with = SearchEngine::format_context_refs(&[&result("2022 Revisi-2, berlaku sejak 2025-11-01")]);
        assert!(with.contains("Versi: 2022 Revisi-2, berlaku sejak 2025-11-01\n"));
        assert!(!SearchEngine::format_context_refs(&[&result("")]).contains("Versi:"));
    }

    #[test]
    fn describes_versions() {
        let v = |l: &str, d: Option<&str>| DocVersion { label: l.into(), valid_from: d.map(Into::into) }.describe();
        assert_eq!(v("", None), "");
        assert_eq!(v("2022", None), "2022");
        assert_eq!(v("", Some("2025-11-01")), "berlaku sejak 2025-11-01");
        assert_eq!(v("2022 Revisi-2", Some("2025-11-01")), "2022 Revisi-2, berlaku sejak 2025-11-01");
    }
}
