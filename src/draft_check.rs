//! Draft test questions (Admin → Sources → review): before approving a draft, generate a few
//! questions a student could ask whose answers are in that draft, then check that search really
//! finds the draft for each question. Low ranks point at pages that are hard to find (missing
//! terms, bad chunks) or at another document that answers the same question (duplicate / older
//! version). Run on demand: 1 generation call + 1 embedding per question.

use pgvector::Vector;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::rag::RagEngine;

pub const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS draft_checks (
        document_id TEXT PRIMARY KEY,
        result      JSONB NOT NULL,
        created_at  TIMESTAMPTZ DEFAULT NOW()
    )",
];

/// How deep the check looks: found in the top 5 = easy to find (the answer uses up to 6 sources).
const TOP: i64 = 5;
const QUESTIONS: usize = 3;
const MAX_TEXT: usize = 7000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QA {
    pub question: String,
    pub answer:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionResult {
    pub question: String,
    pub answer:   String,
    /// Position of this draft among search results (1 = first); None = not in the top 5
    pub rank:     Option<i64>,
    /// The document found first when it is not this draft
    pub top_other: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub questions: Vec<QuestionResult>,
    /// Questions for which the draft is in the top 5
    pub found:     usize,
    pub checked_at: String,
}

const SYSTEM: &str = "Kamu membantu admin menguji dokumen untuk asisten akademik PMPSTI UGM. \
Dari teks dokumen yang diberikan, buat pertanyaan yang mungkin ditanyakan mahasiswa dan jawabannya \
ada di teks itu. Aturan: pertanyaan dalam bahasa Indonesia, singkat dan alami seperti diketik mahasiswa; \
jawaban singkat (maksimal 2 kalimat) dan HANYA dari teks; jangan menyebut 'dokumen ini'; utamakan fakta \
penting (angka, syarat, prosedur, nama). Balas HANYA dengan JSON: \
[{\"question\": \"...\", \"answer\": \"...\"}]";

/// Text sent to the model: the draft's chunks in order, cut at MAX_TEXT characters.
pub fn draft_text(chunks: &[String]) -> String {
    let mut out = String::new();
    for c in chunks {
        if out.len() + c.len() > MAX_TEXT { break; }
        if !out.is_empty() { out.push_str("\n\n"); }
        out.push_str(c);
    }
    if out.is_empty() {
        if let Some(first) = chunks.first() {
            out = first.chars().take(MAX_TEXT).collect();
        }
    }
    out
}

/// The JSON list from the model's reply (it may wrap it in ``` fences or add a sentence).
pub fn parse_questions(reply: &str) -> Vec<QA> {
    let start = reply.find('[');
    let end = reply.rfind(']');
    let (Some(s), Some(e)) = (start, end) else { return vec![] };
    if e <= s { return vec![]; }
    serde_json::from_str::<Vec<QA>>(&reply[s..=e]).unwrap_or_default()
        .into_iter()
        .map(|q| QA { question: q.question.trim().to_string(), answer: q.answer.trim().to_string() })
        .filter(|q| q.question.len() > 5)
        .take(QUESTIONS)
        .collect()
}

/// Generate the questions, check each one, and store the result for the review page.
pub async fn run(pool: &PgPool, rag: &RagEngine, document_id: &str) -> Result<CheckResult, String> {
    let rows = sqlx::query(
        "SELECT content, embedding IS NOT NULL AS embedded FROM documents
         WHERE document_id = $1 AND status = 'draft' ORDER BY chunk_index",
    ).bind(document_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    let chunks: Vec<String> = rows.iter().filter_map(|r| r.get::<Option<String>, _>("content")).collect();
    if chunks.is_empty() {
        return Err("Draft not found".into());
    }
    if !rows.iter().any(|r| r.get::<bool, _>("embedded")) {
        return Err("This draft has no embeddings yet (Gemini quota?), so search cannot find it. \
                    Press \"Check now\" on the source later, then test again.".into());
    }

    let prompt = format!("Buat {QUESTIONS} pertanyaan.\n\nTeks dokumen:\n─────\n{}\n─────", draft_text(&chunks));
    let reply = rag.complete(SYSTEM, &prompt, 1024).await.map_err(|e| format!("Gemini: {e}"))?;
    let qas = parse_questions(&reply);
    if qas.is_empty() {
        return Err("The model did not return questions; try again".into());
    }

    let mut questions = Vec::new();
    for qa in qas {
        let (rank, top_other) = match rag.search.embed_query(&qa.question).await {
            Ok(v) => rank_of(pool, document_id, v).await.unwrap_or((None, None)),
            Err(e) => {
                log::warn!("[draft-check] embedding failed: {e}");
                (None, None)
            }
        };
        questions.push(QuestionResult { question: qa.question, answer: qa.answer, rank, top_other });
    }
    let found = questions.iter().filter(|q| q.rank.is_some()).count();
    let result = CheckResult { questions, found, checked_at: chrono::Utc::now().to_rfc3339() };

    sqlx::query(
        "INSERT INTO draft_checks (document_id, result, created_at) VALUES ($1, $2, NOW())
         ON CONFLICT (document_id) DO UPDATE SET result = EXCLUDED.result, created_at = NOW()",
    ).bind(document_id).bind(serde_json::to_value(&result).map_err(|e| e.to_string())?)
     .execute(pool).await.map_err(|e| e.to_string())?;
    Ok(result)
}

/// Where the draft ranks for a question among the searchable documents plus this draft
/// (by meaning, as if it were approved), and which other document comes first.
async fn rank_of(pool: &PgPool, document_id: &str, embedding: Vec<f32>) -> Result<(Option<i64>, Option<String>), String> {
    // Best chunk per document, top TOP documents
    let rows = sqlx::query(
        "SELECT document_id, MIN(title) AS title, MIN(embedding <=> $1) AS dist
         FROM documents
         WHERE embedding IS NOT NULL AND (status = 'active' OR (status = 'draft' AND document_id = $2))
         GROUP BY document_id
         ORDER BY dist
         LIMIT $3",
    ).bind(Vector::from(embedding)).bind(document_id).bind(TOP)
     .fetch_all(pool).await.map_err(|e| e.to_string())?;
    let rank = rows.iter().position(|r| r.get::<String, _>("document_id") == document_id).map(|i| i as i64 + 1);
    let top_other = rows.first()
        .filter(|r| r.get::<String, _>("document_id") != document_id)
        .map(|r| r.get::<Option<String>, _>("title").unwrap_or_default());
    Ok((rank, top_other))
}

/// Stored result of the last check, if any.
pub async fn stored(pool: &PgPool, document_id: &str) -> Option<CheckResult> {
    sqlx::query("SELECT result FROM draft_checks WHERE document_id = $1")
        .bind(document_id).fetch_optional(pool).await.ok().flatten()
        .and_then(|r| serde_json::from_value(r.get::<serde_json::Value, _>("result")).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_json_and_keeps_three() {
        let reply = "Berikut:\n```json\n[{\"question\": \"Berapa SKS tesis?\", \"answer\": \"6 SKS.\"},\
            {\"question\": \"Kapan seminar proposal?\", \"answer\": \"Semester 2.\"},\
            {\"question\": \"Berapa IPK minimal?\", \"answer\": \"3,00.\"},\
            {\"question\": \"Siapa kaprodi?\", \"answer\": \"X.\"}]\n```";
        let q = parse_questions(reply);
        assert_eq!(q.len(), 3);
        assert_eq!(q[0], QA { question: "Berapa SKS tesis?".into(), answer: "6 SKS.".into() });
        assert!(parse_questions("tidak ada").is_empty());
        assert!(parse_questions("[{\"q\": 1}]").is_empty());
    }

    #[test]
    fn draft_text_is_capped() {
        let chunks = vec!["a".repeat(4000), "b".repeat(4000), "c".repeat(10)];
        let t = draft_text(&chunks);
        assert!(t.len() <= MAX_TEXT && t.starts_with('a') && !t.contains('b'));
        assert_eq!(draft_text(&["x".repeat(9000)]).len(), MAX_TEXT);
    }
}
