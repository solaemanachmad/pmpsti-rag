//! Answer settings an admin can change without a redeploy (Admin → Settings): model, fallback
//! models, temperature, answer length and retrieval limits. Defaults come from the environment;
//! saved values live in `app_settings` and every change is kept in `app_settings_history`.
//! API keys are NOT stored here: they stay in the environment (Render), the page only shows
//! whether a key is set.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Main model for answers
    pub model:           String,
    /// Tried in order when the main model hits its quota (comma separated)
    pub fallback_models: String,
    pub temperature:     f64,
    /// Maximum answer length (tokens)
    pub max_tokens:      u32,
    /// Chunks fetched by the search before filtering
    pub candidates:      i64,
    /// Sources passed to the model (= source cards shown)
    pub max_sources:     usize,
    /// Minimum relevance score for a source. Hybrid top-1 scores on the test set are 0.41–0.60
    /// (median 0.55), so the default 0.35 never drops a correct answer (scripts/eval_retrieval.py).
    pub min_score:       f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            model: String::new(),
            fallback_models: String::new(),
            temperature: 0.3,
            max_tokens: 2048,
            candidates: 10,
            max_sources: 6,
            min_score: 0.35,
        }
    }
}

impl Settings {
    /// Defaults from the environment (what the app used before this page existed).
    pub fn from_env(model: &str) -> Self {
        let fallback = std::env::var("LLM_FALLBACK_MODELS").unwrap_or_else(|_| {
            if model.starts_with("gemini") { "gemini-3.1-flash-lite".to_string() } else { String::new() }
        });
        Settings { model: model.to_string(), fallback_models: fallback, ..Default::default() }
    }

    /// Reject values that would break answers. Returns the cleaned settings.
    pub fn validated(mut self) -> Result<Self, String> {
        self.model = self.model.trim().to_string();
        let names: Vec<String> = self.fallback_models.split(',')
            .map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
        if !valid_model(&self.model) {
            return Err("model: letters, digits and . _ - : / only (1–80 characters)".into());
        }
        if let Some(bad) = names.iter().find(|m| !valid_model(m)) {
            return Err(format!("fallback model \"{bad}\" is not a valid model name"));
        }
        if names.len() > 5 {
            return Err("at most 5 fallback models".into());
        }
        self.fallback_models = names.join(", ");
        if !(0.0..=1.5).contains(&self.temperature) {
            return Err("temperature must be between 0 and 1.5".into());
        }
        if !(256..=8192).contains(&self.max_tokens) {
            return Err("max_tokens must be between 256 and 8192".into());
        }
        if !(3..=30).contains(&self.candidates) {
            return Err("candidates must be between 3 and 30".into());
        }
        if !(1..=10).contains(&self.max_sources) {
            return Err("max_sources must be between 1 and 10".into());
        }
        if !(0.0..=1.0).contains(&self.min_score) {
            return Err("min_score must be between 0 and 1".into());
        }
        Ok(self)
    }
}

fn valid_model(m: &str) -> bool {
    !m.is_empty() && m.len() <= 80
        && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
}

struct Store {
    defaults: Settings,
    current:  RwLock<Arc<Settings>>,
}

fn store() -> &'static OnceLock<Store> {
    static S: OnceLock<Store> = OnceLock::new();
    &S
}

/// Settings in use right now (defaults until `init` ran, e.g. in unit tests).
pub fn get() -> Arc<Settings> {
    match store().get() {
        Some(s) => s.current.read().map(|c| c.clone()).unwrap_or_else(|_| Arc::new(s.defaults.clone())),
        None => Arc::new(Settings::default()),
    }
}

pub fn defaults() -> Settings {
    store().get().map(|s| s.defaults.clone()).unwrap_or_default()
}

/// Bumped by every save/reset, so a reload that read the database before it cannot put the
/// old values back.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn set_current(s: Settings) {
    if let Some(st) = store().get() {
        if let Ok(mut c) = st.current.write() { *c = Arc::new(s); }
    }
}

pub const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS app_settings (
        key        TEXT PRIMARY KEY,
        value      JSONB NOT NULL,
        updated_by BIGINT,
        updated_at TIMESTAMPTZ DEFAULT NOW()
    )",
    "CREATE TABLE IF NOT EXISTS app_settings_history (
        id         BIGSERIAL PRIMARY KEY,
        key        TEXT NOT NULL,
        old_value  JSONB,
        new_value  JSONB,
        changed_by BIGINT,
        changed_at TIMESTAMPTZ DEFAULT NOW()
    )",
];

const KEY: &str = "answers";

/// Create the tables, set the defaults and load saved values; then re-read them every minute.
pub async fn init(pool: &PgPool, defaults: Settings) {
    for sql in SCHEMA {
        if let Err(e) = sqlx::query(sql).execute(pool).await {
            log::warn!("settings schema skipped: {e}");
        }
    }
    let _ = store().set(Store { defaults: defaults.clone(), current: RwLock::new(Arc::new(defaults)) });
    reload(pool).await;
    let pool = pool.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            reload(&pool).await;
        }
    });
}

/// Saved values on top of the defaults (a missing field keeps its default). On a database error
/// the settings in use are kept (not reset to the defaults).
async fn reload(pool: &PgPool) {
    let generation = GENERATION.load(Ordering::SeqCst);
    let saved = match sqlx::query("SELECT value FROM app_settings WHERE key = $1")
        .bind(KEY).fetch_optional(pool).await
    {
        Ok(row) => row.map(|r| r.get::<serde_json::Value, _>("value")),
        Err(e) => {
            log::warn!("settings reload failed, keeping the current ones: {e}");
            return;
        }
    };
    let merged = merge(&defaults(), saved);
    if GENERATION.load(Ordering::SeqCst) == generation {
        set_current(merged);
    }
}

fn merge(defaults: &Settings, saved: Option<serde_json::Value>) -> Settings {
    let Some(serde_json::Value::Object(saved)) = saved else { return defaults.clone() };
    let mut base = serde_json::to_value(defaults).unwrap_or_default();
    if let serde_json::Value::Object(b) = &mut base {
        for (k, v) in saved { b.insert(k, v); }
    }
    serde_json::from_value::<Settings>(base).ok()
        .and_then(|s| s.validated().ok())
        .unwrap_or_else(|| defaults.clone())
}

/// Save new settings (validated), record the change, and use them right away.
pub async fn save(pool: &PgPool, new: Settings, user_id: Option<i64>) -> Result<Settings, String> {
    let new = new.validated()?;
    let old = get();
    let value = serde_json::to_value(&new).map_err(|e| e.to_string())?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_by, updated_at) VALUES ($1, $2, $3, NOW())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by = EXCLUDED.updated_by, updated_at = NOW()",
    ).bind(KEY).bind(&value).bind(user_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO app_settings_history (key, old_value, new_value, changed_by) VALUES ($1, $2, $3, $4)")
        .bind(KEY).bind(serde_json::to_value(&*old).ok()).bind(&value).bind(user_id)
        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    GENERATION.fetch_add(1, Ordering::SeqCst);
    tx.commit().await.map_err(|e| e.to_string())?;
    set_current(new.clone());
    GENERATION.fetch_add(1, Ordering::SeqCst); // a reload that overlapped the save is discarded
    Ok(new)
}

/// Back to the environment defaults (the change is recorded).
pub async fn reset(pool: &PgPool, user_id: Option<i64>) -> Result<Settings, String> {
    let old = get();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM app_settings WHERE key = $1").bind(KEY)
        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO app_settings_history (key, old_value, new_value, changed_by) VALUES ($1, $2, NULL, $3)")
        .bind(KEY).bind(serde_json::to_value(&*old).ok()).bind(user_id)
        .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    GENERATION.fetch_add(1, Ordering::SeqCst);
    tx.commit().await.map_err(|e| e.to_string())?;
    let d = defaults();
    set_current(d.clone());
    GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(d)
}

#[derive(Debug, Serialize)]
pub struct Change {
    pub changed_at: String,
    pub changed_by: Option<String>,
    /// Fields that changed: (name, old, new); empty new = reset to defaults
    pub fields:     Vec<(String, String, String)>,
}

/// Last changes, newest first, with the fields that changed.
pub async fn history(pool: &PgPool, limit: i64) -> Result<Vec<Change>, String> {
    let rows = sqlx::query(
        "SELECT to_json(h.changed_at)#>>'{}' AS at, u.email, h.old_value, h.new_value
         FROM app_settings_history h LEFT JOIN users u ON u.id = h.changed_by
         WHERE h.key = $1 ORDER BY h.id DESC LIMIT $2",
    ).bind(KEY).bind(limit).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| {
        let old: Option<serde_json::Value> = r.get("old_value");
        let new: Option<serde_json::Value> = r.get("new_value");
        Change { changed_at: r.get("at"), changed_by: r.get("email"), fields: diff(old.as_ref(), new.as_ref()) }
    }).collect())
}

fn diff(old: Option<&serde_json::Value>, new: Option<&serde_json::Value>) -> Vec<(String, String, String)> {
    let show = |v: Option<&serde_json::Value>| match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let Some(new) = new else { return vec![("all".into(), "custom".into(), String::new())] };
    let keys: Vec<String> = new.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    keys.into_iter()
        .filter(|k| old.and_then(|o| o.get(k)) != new.get(k))
        .map(|k| { let o = show(old.and_then(|o| o.get(&k))); let n = show(new.get(&k)); (k, o, n) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_cleans() {
        let s = Settings { model: " gemini-3.5-flash-lite ".into(), fallback_models: "gemini-3.1-flash-lite,, x ".into(), ..Default::default() };
        let v = s.validated().unwrap();
        assert_eq!(v.model, "gemini-3.5-flash-lite");
        assert_eq!(v.fallback_models, "gemini-3.1-flash-lite, x");
        assert!(Settings { model: "a b".into(), ..Default::default() }.validated().is_err());
        assert!(Settings { model: "m".into(), temperature: 3.0, ..Default::default() }.validated().is_err());
        assert!(Settings { model: "m".into(), max_sources: 0, ..Default::default() }.validated().is_err());
    }

    #[test]
    fn saved_values_override_defaults_field_by_field() {
        let d = Settings::from_env("gemini-3.5-flash-lite");
        let m = merge(&d, Some(serde_json::json!({"temperature": 0.1, "unknown": 1})));
        assert_eq!(m.temperature, 0.1);
        assert_eq!(m.model, "gemini-3.5-flash-lite");
        // an invalid saved value falls back to the defaults instead of breaking answers
        assert_eq!(merge(&d, Some(serde_json::json!({"max_tokens": 5}))), d);
    }

    #[test]
    fn diff_lists_changed_fields() {
        let a = serde_json::json!({"model": "a", "temperature": 0.3});
        let b = serde_json::json!({"model": "b", "temperature": 0.3});
        assert_eq!(diff(Some(&a), Some(&b)), vec![("model".to_string(), "a".to_string(), "b".to_string())]);
    }
}
