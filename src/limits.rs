//! Request limits and the caller's IP address.
//!
//! - `client_ip`: in production every request reaches the backend through the SvelteKit proxy on
//!   Vercel, so the socket peer is a Vercel address shared by all visitors. The proxy sends the
//!   visitor's address in `X-Real-IP` together with `X-Proxy-Secret`; the header is trusted only
//!   when that secret matches `PROXY_SECRET`, so nobody calling the backend directly can choose
//!   their own IP. Without `PROXY_SECRET` the socket peer is used (local development).
//! - `Limiter`: fixed-window counters kept in memory (reset on restart, which is acceptable for
//!   abuse protection). Pruned regularly from `main`.
//! - `reject_cross_site`: blocks requests that change data when a browser sends them from another
//!   site (CSRF). Together with the `SameSite=Lax` login cookie.

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::Method;
use actix_web::middleware::Next;
use actix_web::{HttpRequest, HttpResponse};
use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const PROXY_SECRET_HEADER: &str = "X-Proxy-Secret";
pub const REAL_IP_HEADER: &str = "X-Real-IP";

fn proxy_secret() -> Option<&'static str> {
    static SECRET: OnceLock<Option<String>> = OnceLock::new();
    SECRET
        .get_or_init(|| std::env::var("PROXY_SECRET").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .as_deref()
}

/// Compare without stopping at the first different byte, so the time taken does not reveal
/// how much of the secret was right.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The visitor's IP address (see the module docs for when `X-Real-IP` is trusted).
pub fn client_ip(req: &HttpRequest) -> String {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    trusted_real_ip(proxy_secret(), header(PROXY_SECRET_HEADER), header(REAL_IP_HEADER))
        .unwrap_or_else(|| req.connection_info().peer_addr().unwrap_or("unknown").to_string())
}

/// `X-Real-IP` when the proxy secret is configured, matches, and the header is a valid address.
fn trusted_real_ip(secret: Option<&str>, sent_secret: Option<&str>, real_ip: Option<&str>) -> Option<String> {
    let (secret, sent) = (secret?, sent_secret?);
    if !same_secret(secret, sent) {
        return None;
    }
    real_ip?.parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

/// True when `PROXY_SECRET` is set (logged at startup).
pub fn proxy_secret_configured() -> bool {
    proxy_secret().is_some()
}

/// True when the request came through the SvelteKit proxy (it sent the matching secret).
fn sent_by_proxy(req: &HttpRequest) -> bool {
    let sent = req.headers().get(PROXY_SECRET_HEADER).and_then(|v| v.to_str().ok()).map(str::trim);
    matches!((proxy_secret(), sent), (Some(secret), Some(sent)) if same_secret(secret, sent))
}

// ── Cross-site requests (CSRF) ──

/// Sites whose pages may send requests that change data: the frontend, plus the Vite dev and
/// preview servers. Set once at startup from FRONTEND_URL and APP_BASE_URL.
static ALLOWED_ORIGINS: OnceLock<Vec<String>> = OnceLock::new();

pub fn set_allowed_origins(urls: &[&str]) {
    let mut list: Vec<String> = urls.iter().map(|u| normalize_origin(u)).filter(|u| !u.is_empty()).collect();
    list.extend(["http://localhost:5173", "http://localhost:4173"].map(String::from));
    list.dedup();
    let _ = ALLOWED_ORIGINS.set(list);
}

/// "https://Pmpsti-RAG.vercel.app/" → "https://pmpsti-rag.vercel.app"
fn normalize_origin(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Whether a request may go on. Reading (GET, HEAD, OPTIONS) is always allowed. A request that
/// changes data is refused only when it carries an `Origin` that is not one of ours: browsers
/// always send `Origin` on such requests from another site, while tools (curl, the eval
/// scripts, API keys) send none.
fn origin_allowed(method: &Method, origin: Option<&str>, allowed: &[String]) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    match origin {
        None => true,
        Some(o) => allowed.contains(&normalize_origin(o)),
    }
}

/// Middleware: 403 for a request that changes data and comes from another site's page.
/// Requests from the frontend proxy skip the check: the proxy already compared the browser's
/// `Origin` with its own address (which also covers Vercel Preview addresses).
pub async fn reject_cross_site(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let origin = req.headers().get(actix_web::http::header::ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string);
    let allowed = ALLOWED_ORIGINS.get().map(Vec::as_slice).unwrap_or(&[]);
    if !sent_by_proxy(req.request()) && !origin_allowed(req.method(), origin.as_deref(), allowed) {
        log::warn!("[csrf] refused {} {} from origin {:?}", req.method(), req.path(), origin);
        let res = HttpResponse::Forbidden()
            .json(crate::models::ApiError::new(403, "Permintaan dari situs lain ditolak"));
        return Ok(req.into_response(res).map_into_right_body());
    }
    Ok(next.call(req).await?.map_into_left_body())
}

/// HTTP client for calls to outside services, so a stalled connection cannot hang a request:
/// `connect` limits the connection setup, `read` the wait for each piece of the response (works
/// for streamed answers too), and `total` the whole call when set.
pub fn http_client(connect: Duration, read: Duration, total: Option<Duration>) -> reqwest::Client {
    let mut b = reqwest::Client::builder().connect_timeout(connect).read_timeout(read);
    if let Some(t) = total {
        b = b.timeout(t);
    }
    b.build().unwrap_or_else(|e| {
        log::error!("HTTP client with timeouts could not be built ({e}); using the default client");
        reqwest::Client::new()
    })
}

struct Window {
    count: u32,
    start: Instant,
    length: Duration,
}

/// Fixed-window counters by key, e.g. "guest-ip:1.2.3.4" or "user:42".
#[derive(Default)]
pub struct Limiter {
    windows: DashMap<String, Window>,
}

impl Limiter {
    /// Count one request for `key`. Err(seconds until the window resets) when `limit` requests
    /// were already counted in the current window of length `window`.
    pub fn hit(&self, key: &str, limit: u32, window: Duration) -> Result<(), u64> {
        let now = Instant::now();
        let mut w = self.windows.entry(key.to_string())
            .or_insert_with(|| Window { count: 0, start: now, length: window });
        if now.duration_since(w.start) >= w.length {
            w.count = 0;
            w.start = now;
            w.length = window;
        }
        if w.count >= limit {
            return Err(w.length.saturating_sub(now.duration_since(w.start)).as_secs().max(1));
        }
        w.count += 1;
        Ok(())
    }

    /// Drop windows that have ended, so the map does not grow without bound.
    pub fn prune(&self) {
        let now = Instant::now();
        self.windows.retain(|_, w| now.duration_since(w.start) < w.length);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.windows.len()
    }
}

/// "3 jam" / "12 menit" / "40 detik", for limit messages shown to students.
pub fn wait_text(secs: u64) -> String {
    match secs {
        s if s >= 3600 => format!("{} jam", (s + 3599) / 3600),
        s if s >= 60 => format!("{} menit", (s + 59) / 60),
        s => format!("{s} detik"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_ip_needs_the_matching_secret() {
        assert_eq!(trusted_real_ip(Some("s3cret"), Some("s3cret"), Some("203.0.113.7")).as_deref(), Some("203.0.113.7"));
        assert_eq!(trusted_real_ip(Some("s3cret"), Some("wrong!"), Some("203.0.113.7")), None);
        assert_eq!(trusted_real_ip(Some("s3cret"), None, Some("203.0.113.7")), None);
        // No secret configured: never trust the header
        assert_eq!(trusted_real_ip(None, Some("anything"), Some("203.0.113.7")), None);
        // Not an address
        assert_eq!(trusted_real_ip(Some("s3cret"), Some("s3cret"), Some("1.2.3.4, 5.6.7.8")), None);
        assert_eq!(trusted_real_ip(Some("s3cret"), Some("s3cret"), Some("2001:db8::1")).as_deref(), Some("2001:db8::1"));
    }

    #[test]
    fn limiter_counts_per_key_and_window() {
        let l = Limiter::default();
        let w = Duration::from_secs(60);
        assert!(l.hit("a", 2, w).is_ok());
        assert!(l.hit("a", 2, w).is_ok());
        let wait = l.hit("a", 2, w).unwrap_err();
        assert!(wait > 0 && wait <= 60);
        assert!(l.hit("b", 2, w).is_ok(), "other keys are counted separately");
    }

    #[test]
    fn limiter_resets_after_the_window_and_prunes() {
        let l = Limiter::default();
        let w = Duration::from_millis(30);
        assert!(l.hit("a", 1, w).is_ok());
        assert!(l.hit("a", 1, w).is_err());
        std::thread::sleep(Duration::from_millis(40));
        l.prune();
        assert_eq!(l.len(), 0);
        assert!(l.hit("a", 1, w).is_ok());
    }

    #[test]
    fn writes_from_other_sites_are_refused() {
        let allowed = vec!["https://pmpsti-rag.vercel.app".to_string(), "http://localhost:5173".to_string()];
        let ok = |m: Method, o: Option<&str>| origin_allowed(&m, o, &allowed);
        // Our own pages (any case, trailing slash)
        assert!(ok(Method::POST, Some("https://pmpsti-rag.vercel.app")));
        assert!(ok(Method::DELETE, Some("https://PMPSTI-RAG.vercel.app/")));
        assert!(ok(Method::POST, Some("http://localhost:5173")));
        // Tools without Origin (curl, eval scripts, API keys)
        assert!(ok(Method::POST, None));
        // Another site's form or script
        assert!(!ok(Method::POST, Some("https://evil.example")));
        assert!(!ok(Method::PATCH, Some("null")));
        assert!(!ok(Method::PUT, Some("https://pmpsti-rag.vercel.app.evil.example")));
        // Reading is never blocked
        assert!(ok(Method::GET, Some("https://evil.example")));
        assert!(ok(Method::OPTIONS, Some("https://evil.example")));
    }

    #[test]
    fn wait_text_rounds_up() {
        assert_eq!(wait_text(30), "30 detik");
        assert_eq!(wait_text(61), "2 menit");
        assert_eq!(wait_text(3601), "2 jam");
    }
}
