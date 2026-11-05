use serde::{Deserialize, Serialize};

// ══════════════════════════════════════════════════════════════════
//  JWT CLAIMS
// ══════════════════════════════════════════════════════════════════

/// Payload yang di-encode ke dalam JWT token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub:   String,   // user id (sebagai string)
    pub email: String,
    pub role:  String,
    pub exp:   usize,    // unix timestamp expiry
    pub iat:   usize,    // unix timestamp issued-at
}

// ══════════════════════════════════════════════════════════════════
//  AUTH REQUEST / RESPONSE
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email:        String,
    pub password:     String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email:    String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token:        String,
    pub token_type:   String,   // "Bearer"
    pub expires_in:   u64,      // seconds
    pub user:         UserPublic,
}

/// Versi publik User — tanpa password_hash
#[derive(Debug, Serialize)]
pub struct UserPublic {
    pub id:           i64,
    pub email:        String,
    pub display_name: String,
    pub role:         String,
    pub created_at:   String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub display_name: Option<String>,
    pub email:        Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePasswordRequest {
    pub current_password: String,
    pub new_password:     String,
}

// ══════════════════════════════════════════════════════════════════
//  CHAT REQUEST / RESPONSE
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct AskRequest {
    pub query:           String,
    pub session_id:      Option<String>,
    pub category_filter: Option<String>,
    pub search_mode:     Option<String>,  // "hybrid" | "semantic" | "keyword"
    pub top_k:           Option<i64>,
    /// true = stream the answer as NDJSON lines (chat UI); default: one JSON response
    #[serde(default)]
    pub stream:          Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AskResponse {
    pub answer:     String,
    pub session_id: String,
    pub sources:    Vec<SourceRef>,
    pub search_time_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct SourceRef {
    pub title:      String,
    pub snippet:    String,
    pub source_url: String,
    pub category:   String,
    pub subcategory: String,
    pub score:      f64,
    /// Pages of a PDF document used for the answer (empty for web pages)
    pub pages:      Vec<i32>,
}

// ══════════════════════════════════════════════════════════════════
//  API KEY REQUEST / RESPONSE
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name:        String,
    pub permissions: Option<Vec<String>>,
    pub rate_limit:  Option<i32>,
    pub expires_at:  Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id:         i64,
    pub key:        String,   // full key — HANYA ditampilkan sekali saat create
    pub key_prefix: String,
    pub name:       String,
}

// ══════════════════════════════════════════════════════════════════
//  GENERIC RESPONSE WRAPPERS
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize)]
pub struct ApiSuccess<T: Serialize> {
    pub success: bool,
    pub data:    T,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub success: bool,
    pub error:   String,
    pub code:    u16,
}

impl<T: Serialize> ApiSuccess<T> {
    pub fn new(data: T) -> Self {
        Self { success: true, data }
    }
}

impl ApiError {
    pub fn new(code: u16, error: impl Into<String>) -> Self {
        Self { success: false, error: error.into(), code }
    }
}

/// What the client sees when something fails inside the server. Database and Gemini errors
/// can show SQL, table, model and host names, so their details go to the log only.
pub const SERVER_ERROR_MESSAGE: &str = "Terjadi kesalahan di server. Silakan coba lagi.";
/// What the student sees when the answer could not be written (Gemini failed or was cut off).
pub const ANSWER_ERROR_MESSAGE: &str = "Gagal mendapat jawaban. Silakan coba lagi sebentar lagi.";

/// 500 with a generic message; the real error is logged with the caller's file and line.
#[track_caller]
pub fn internal_error(e: impl std::fmt::Display) -> actix_web::HttpResponse {
    let at = std::panic::Location::caller();
    log::error!("[{}:{}] {e}", at.file(), at.line());
    actix_web::HttpResponse::InternalServerError().json(ApiError::new(500, SERVER_ERROR_MESSAGE))
}
// ══════════════════════════════════════════════════════════════════
//  DOCUMENT INGEST REQUESTS
// ══════════════════════════════════════════════════════════════════

/// Strategi chunking yang bisa dipilih saat ingest.
/// - `auto`        : Deteksi otomatis berdasarkan konten (default)
/// - `semantic`    : Embed tiap kalimat, split saat similarity drop (hemat quota dengan bijak)
/// - `sentence`    : Split di batas kalimat/paragraf, tanpa embedding (0 Gemini calls saat chunking)
/// - `structural`  : Baris/blok terstruktur (cocok untuk daftar, tabel, data dosen)
/// - `fixed`       : Split setiap N karakter (paling cepat, fallback)
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChunkStrategy {
    Auto,
    Semantic,
    Sentence,
    Structural,
    Fixed,
}

impl Default for ChunkStrategy {
    fn default() -> Self { ChunkStrategy::Auto }
}

impl ChunkStrategy {
    /// Deteksi strategi terbaik berdasarkan konten teks.
    /// Dipanggil saat strategy == Auto.
    pub fn detect(text: &str) -> Self {
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len().max(1);

        // Hitung baris yang dimulai dengan nomor (e.g. "1.", "2.", "No. 3")
        let numbered = lines.iter().filter(|l| {
            let t = l.trim();
            t.starts_with(|c: char| c.is_ascii_digit())
                && t.chars().nth(1).map(|c| c == '.' || c == ')').unwrap_or(false)
        }).count();

        // Hitung baris yang mengandung separator pipe (tabel)
        let piped = lines.iter().filter(|l| l.contains('|')).count();

        // Hitung baris pendek (<60 char) yang mengandung ':' — label:value
        let label_value = lines.iter().filter(|l| {
            let t = l.trim();
            t.len() < 80 && t.contains(':')
        }).count();

        // Structural: banyak baris bernomor, tabel, atau label:value
        if numbered > total / 5 || piped > total / 4 || label_value > total / 3 {
            return ChunkStrategy::Structural;
        }

        // Default: sentence (cocok untuk prosa, artikel, dll)
        ChunkStrategy::Sentence
    }
}

#[derive(Debug, Deserialize)]
pub struct IngestUrlRequest {
    pub url:            String,
    pub title:          Option<String>,
    pub category:       Option<String>,
    pub subcategory:    Option<String>,
    /// Strategi chunking. Default: "sentence" (tidak perlu Gemini quota saat chunking)
    #[serde(default)]
    pub chunk_strategy: ChunkStrategy,
}

#[derive(Debug, Deserialize)]
pub struct IngestTextRequest {
    pub content:        String,
    pub title:          String,
    pub source_url:     Option<String>,
    pub category:       Option<String>,
    pub subcategory:    Option<String>,
    /// Strategi chunking. Default: "sentence"
    #[serde(default)]
    pub chunk_strategy: ChunkStrategy,
}


#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub document_id: String,
    pub chunks:      usize,
    pub title:       String,
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[actix_web::test]
    async fn server_errors_hide_the_details() {
        let res = internal_error("error returned from database: relation \"users\" does not exist");
        assert_eq!(res.status(), 500);
        let body = actix_web::body::to_bytes(res.into_body()).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains(SERVER_ERROR_MESSAGE));
        assert!(!text.contains("relation") && !text.contains("users"));
    }
}
