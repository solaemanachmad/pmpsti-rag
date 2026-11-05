//! Topic categories for documents (Admin → Documents), two levels: "Akademik › Persuratan".
//!
//! - Table `categories`: `parent_id` NULL = top level. Seeded with `DEFAULT_TAXONOMY` when the
//!   table is empty; after that the list belongs to the admins (Admin → Categories: add,
//!   rename, reorder, delete/merge). Lookups use the `slug`, so a rename keeps the slug.
//! - `documents.category_id` points at a category; NULL = uncategorized. The old free-text
//!   `category` / `subcategory` columns are left exactly as they are: source cards in the chat
//!   build their titles from `subcategory`, so this module never rewrites them.
//! - Existing documents are filed ONCE, on the start that adds `documents.category_id`, from
//!   their free-text values (`legacy_target`): a known pair, or a subcategory equal to a category
//!   name. Anything else stays uncategorized for the admin to file — no guessing. Not repeated
//!   later: NULL can also mean "the admin chose uncategorized" or "its category was deleted".
//! - A new version of a document inherits the category of the version it replaces when the
//!   draft is approved (`inherit_in_tx`, called from `sources::approve_in_tx`).

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::handlers::{require_auth, AppState};
use crate::models::{ApiError, ApiSuccess, JwtClaims};

/// The starting list (top level → subcategories). Names are what admins see.
pub const DEFAULT_TAXONOMY: &[(&str, &[&str])] = &[
    ("Akademik", &["Kurikulum", "Tesis", "Kelulusan & Wisuda", "Persuratan", "Aturan & Panduan Umum"]),
    ("Sumber Daya Manusia", &["Dosen", "Tenaga Kependidikan", "Pengelola Prodi"]),
    ("Profil Prodi", &["Identitas & Pendahuluan", "Kelompok Bidang Keahlian", "Kerja Sama"]),
    ("Lainnya", &[]),
];

/// Free-text (category, subcategory) pairs in use before categories existed → (top level,
/// subcategory) in `DEFAULT_TAXONOMY`. Compared case-insensitively after trimming.
const LEGACY_MAP: &[(&str, &str, &str, &str)] = &[
    ("Buku", "Akademik", "Akademik", "Aturan & Panduan Umum"),
    ("Informasi", "Kelompok Bidang Keahlian", "Profil Prodi", "Kelompok Bidang Keahlian"),
    ("Informasi", "Kerja Sama", "Profil Prodi", "Kerja Sama"),
    ("Informasi", "Pengelolaan Program Studi", "Sumber Daya Manusia", "Pengelola Prodi"),
    ("Informasi", "Rata-Rata Beban Dosen", "Sumber Daya Manusia", "Dosen"),
    ("Informasi", "Tenaga Kependidikan", "Sumber Daya Manusia", "Tenaga Kependidikan"),
    ("Panduan", "Kelulusan dan Wisuda", "Akademik", "Kelulusan & Wisuda"),
    ("Panduan", "Kurikulum", "Akademik", "Kurikulum"),
    ("Panduan", "Layanan Persuratan", "Akademik", "Persuratan"),
    ("Panduan", "Pendahuluan", "Profil Prodi", "Identitas & Pendahuluan"),
    ("Panduan", "Tesis", "Akademik", "Tesis"),
    ("Website", "Pengelola", "Sumber Daya Manusia", "Pengelola Prodi"),
    ("Website", "Profil", "Sumber Daya Manusia", "Dosen"),
];

/// "Kelulusan & Wisuda" → "kelulusan-wisuda". A subcategory's slug is "parent/child".
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn child_slug(parent: &str, child: &str) -> String {
    format!("{}/{}", slugify(parent), slugify(child))
}

/// Where a document with these free-text values belongs (slug of the subcategory), or None.
pub fn legacy_target(category: &str, subcategory: &str) -> Option<String> {
    let (cat, sub) = (category.trim().to_lowercase(), subcategory.trim().to_lowercase());
    if let Some(&(_, _, parent, child)) = LEGACY_MAP.iter()
        .find(|(c, s, _, _)| c.to_lowercase() == cat && s.to_lowercase() == sub)
    {
        return Some(child_slug(parent, child));
    }
    // A subcategory already written as one of our category names, e.g. "Akademik / Kurikulum"
    if sub.is_empty() {
        return None;
    }
    DEFAULT_TAXONOMY.iter()
        .flat_map(|(parent, children)| children.iter().map(move |child| (*parent, *child)))
        .find(|(_, child)| child.to_lowercase() == sub)
        .map(|(parent, child)| child_slug(parent, child))
}

// ── Database ──────────────────────────────────────────────────────

/// Create the table, seed it when empty, add `documents.category_id`, file existing documents.
/// Called from `Database::setup`; every statement is idempotent.
pub async fn setup(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS categories (
            id         BIGSERIAL PRIMARY KEY,
            parent_id  BIGINT REFERENCES categories(id) ON DELETE RESTRICT,
            name       TEXT NOT NULL,
            slug       TEXT NOT NULL UNIQUE,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )").execute(pool).await?;
    let column_existed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                        WHERE table_name = 'documents' AND column_name = 'category_id')")
        .fetch_one(pool).await?;
    sqlx::query("ALTER TABLE documents ADD COLUMN IF NOT EXISTS category_id BIGINT REFERENCES categories(id) ON DELETE SET NULL")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS documents_category_id_idx ON documents(category_id)")
        .execute(pool).await?;

    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM categories").fetch_one(pool).await?;
    if existing == 0 {
        seed(pool).await?;
    }
    // One-shot: only on the start that created the column (see the module docs)
    if !column_existed {
        let filed = backfill(pool).await?;
        log::info!("Categories: filed {filed} document chunks from their old free-text category");
    }
    Ok(())
}

/// Insert the default list. Safe if two instances start at once: existing slugs are skipped
/// and parents are looked up by slug.
async fn seed(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (i, (parent, children)) in DEFAULT_TAXONOMY.iter().enumerate() {
        sqlx::query("INSERT INTO categories (name, slug, sort_order) VALUES ($1, $2, $3)
                     ON CONFLICT (slug) DO NOTHING")
            .bind(parent).bind(slugify(parent)).bind(i as i32)
            .execute(&mut *tx).await?;
        let parent_id: i64 = sqlx::query_scalar("SELECT id FROM categories WHERE slug = $1")
            .bind(slugify(parent)).fetch_one(&mut *tx).await?;
        for (j, child) in children.iter().enumerate() {
            sqlx::query("INSERT INTO categories (parent_id, name, slug, sort_order) VALUES ($1, $2, $3, $4)
                         ON CONFLICT (slug) DO NOTHING")
                .bind(parent_id).bind(child).bind(child_slug(parent, child)).bind(j as i32)
                .execute(&mut *tx).await?;
        }
    }
    tx.commit().await
}

/// File uncategorized documents whose free-text values match `legacy_target`.
/// Returns the number of chunk rows updated.
async fn backfill(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let pairs = sqlx::query(
        "SELECT DISTINCT COALESCE(category, '') AS category, COALESCE(subcategory, '') AS subcategory
         FROM documents WHERE category_id IS NULL")
        .fetch_all(pool).await?;
    let mut updated = 0;
    for row in pairs {
        let (category, subcategory): (String, String) = (row.get("category"), row.get("subcategory"));
        let Some(slug) = legacy_target(&category, &subcategory) else { continue };
        updated += sqlx::query(
            "UPDATE documents SET category_id = (SELECT id FROM categories WHERE slug = $1)
             WHERE category_id IS NULL AND COALESCE(category, '') = $2 AND COALESCE(subcategory, '') = $3
               AND EXISTS (SELECT 1 FROM categories WHERE slug = $1)")
            .bind(&slug).bind(&category).bind(&subcategory)
            .execute(pool).await?
            .rows_affected();
    }
    Ok(updated)
}

#[derive(Debug, Serialize)]
pub struct Category {
    pub id:         i64,
    pub parent_id:  Option<i64>,
    pub name:       String,
    pub slug:       String,
    pub sort_order: i32,
    /// Documents filed directly in this category (not its subcategories); archived ones not counted
    pub document_count: i64,
}

/// Every category, parents first, each group in sort order; the UI builds the tree.
pub async fn list(pool: &PgPool) -> Result<Vec<Category>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT c.id, c.parent_id, c.name, c.slug, c.sort_order,
                COUNT(DISTINCT d.document_id) AS document_count
         FROM categories c
         LEFT JOIN documents d ON d.category_id = c.id AND d.status <> 'archived'
         GROUP BY c.id
         ORDER BY (c.parent_id IS NOT NULL), c.sort_order, c.name")
        .fetch_all(pool).await?;
    Ok(rows.iter().map(|r| Category {
        id: r.get("id"),
        parent_id: r.get("parent_id"),
        name: r.get("name"),
        slug: r.get("slug"),
        sort_order: r.get("sort_order"),
        document_count: r.get("document_count"),
    }).collect())
}

async fn uncategorized_count(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(DISTINCT document_id) FROM documents
                        WHERE category_id IS NULL AND status <> 'archived'")
        .fetch_one(pool).await
}

/// File a document (all its chunks) under `category_id`, or uncategorize it with None.
/// Returns the chunk rows updated (0 = no such document).
pub async fn set_document_category(pool: &PgPool, document_id: &str, category_id: Option<i64>) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE documents SET category_id = $2 WHERE document_id = $1")
        .bind(document_id).bind(category_id)
        .execute(pool).await?
        .rows_affected())
}

/// When a draft replaces older versions, give it their category unless it already has one.
/// Runs inside the approve transaction, so only approval writes to the active document.
pub async fn inherit_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: &str,
    replaced: &[String],
) -> Result<u64, sqlx::Error> {
    if replaced.is_empty() {
        return Ok(0);
    }
    Ok(sqlx::query(
        "UPDATE documents
         SET category_id = (SELECT MIN(category_id) FROM documents WHERE document_id = ANY($2::text[]))
         WHERE document_id = $1 AND category_id IS NULL")
        .bind(document_id).bind(replaced)
        .execute(&mut **tx).await?
        .rows_affected())
}

async fn exists(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
        .bind(id).fetch_one(pool).await
}

/// Why a category change was refused (mapped to an HTTP status by `CatError::response`).
#[derive(Debug)]
pub enum CatError {
    BadRequest(String),
    NotFound,
    Conflict(String),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for CatError {
    fn from(e: sqlx::Error) -> Self { CatError::Db(e) }
}

impl CatError {
    fn response(self) -> HttpResponse {
        match self {
            CatError::BadRequest(m) => HttpResponse::BadRequest().json(ApiError::new(400, m)),
            CatError::NotFound => HttpResponse::NotFound().json(ApiError::new(404, "Category not found")),
            CatError::Conflict(m) => HttpResponse::Conflict().json(ApiError::new(409, m)),
            CatError::Db(e) => crate::models::internal_error(e),
        }
    }
}

const MAX_NAME_LEN: usize = 80;

/// Trimmed name, or why it is not usable.
fn clean_name(name: &str) -> Result<String, CatError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CatError::BadRequest("Name is required".into()));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(CatError::BadRequest(format!("Name is longer than {MAX_NAME_LEN} characters")));
    }
    if slugify(name).is_empty() {
        return Err(CatError::BadRequest("Name needs at least one letter or digit".into()));
    }
    Ok(name.to_string())
}

/// Refuse a name another category under the same parent already has (case-insensitive).
async fn ensure_unique_name(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    parent_id: Option<i64>,
    name: &str,
    except: Option<i64>,
) -> Result<(), CatError> {
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories
                        WHERE parent_id IS NOT DISTINCT FROM $1 AND LOWER(name) = LOWER($2)
                          AND id IS DISTINCT FROM $3)")
        .bind(parent_id).bind(name).bind(except)
        .fetch_one(&mut **tx).await?;
    if taken {
        return Err(CatError::Conflict(format!("\"{name}\" already exists here")));
    }
    Ok(())
}

/// Add a category at the top level (`parent_id` None) or under a top-level category; it goes
/// last among its siblings. The slug comes from the name ("parent-slug/child" for a
/// subcategory); a slug kept by a renamed category gets a numeric suffix. Returns the new id.
pub async fn create(pool: &PgPool, name: &str, parent_id: Option<i64>) -> Result<i64, CatError> {
    let name = clean_name(name)?;
    let mut tx = pool.begin().await?;
    let base = match parent_id {
        None => slugify(&name),
        Some(pid) => {
            let parent = sqlx::query("SELECT slug, parent_id FROM categories WHERE id = $1")
                .bind(pid).fetch_optional(&mut *tx).await?
                .ok_or_else(|| CatError::BadRequest(format!("Unknown parent category {pid}")))?;
            if parent.get::<Option<i64>, _>("parent_id").is_some() {
                return Err(CatError::BadRequest("Categories have two levels: pick a top-level parent".into()));
            }
            format!("{}/{}", parent.get::<String, _>("slug"), slugify(&name))
        }
    };
    ensure_unique_name(&mut tx, parent_id, &name, None).await?;
    let mut slug = base.clone();
    let mut n = 2;
    while sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM categories WHERE slug = $1)")
        .bind(&slug).fetch_one(&mut *tx).await?
    {
        slug = format!("{base}-{n}");
        n += 1;
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO categories (parent_id, name, slug, sort_order)
         VALUES ($1, $2, $3, (SELECT COALESCE(MAX(sort_order) + 1, 0) FROM categories
                              WHERE parent_id IS NOT DISTINCT FROM $1))
         RETURNING id")
        .bind(parent_id).bind(&name).bind(&slug)
        .fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}

/// Change the name only; the slug stays so lookups and rules keep working.
pub async fn rename(pool: &PgPool, id: i64, name: &str) -> Result<(), CatError> {
    let name = clean_name(name)?;
    let mut tx = pool.begin().await?;
    let parent_id: Option<i64> = sqlx::query_scalar("SELECT parent_id FROM categories WHERE id = $1")
        .bind(id).fetch_optional(&mut *tx).await?
        .ok_or(CatError::NotFound)?;
    ensure_unique_name(&mut tx, parent_id, &name, Some(id)).await?;
    sqlx::query("UPDATE categories SET name = $2 WHERE id = $1")
        .bind(id).bind(&name).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Set the order of the siblings under `parent_id` (None = top level). `ids` must list exactly
/// those siblings, so a stale page cannot drop or misplace one.
pub async fn reorder(pool: &PgPool, parent_id: Option<i64>, ids: &[i64]) -> Result<(), CatError> {
    let mut tx = pool.begin().await?;
    let mut siblings: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM categories WHERE parent_id IS NOT DISTINCT FROM $1 FOR UPDATE")
        .bind(parent_id).fetch_all(&mut *tx).await?;
    let mut wanted = ids.to_vec();
    siblings.sort_unstable();
    wanted.sort_unstable();
    if siblings != wanted {
        return Err(CatError::Conflict("The category list changed; reload and try again".into()));
    }
    for (i, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE categories SET sort_order = $2 WHERE id = $1")
            .bind(id).bind(i as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Delete a category that has no subcategories. Its documents move to `move_to` (merge) or,
/// with None, become uncategorized. Returns how many documents were moved.
pub async fn delete(pool: &PgPool, id: i64, move_to: Option<i64>) -> Result<i64, CatError> {
    if move_to == Some(id) {
        return Err(CatError::BadRequest("Pick another category to move the documents to".into()));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM categories WHERE id = $1 FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?
        .ok_or(CatError::NotFound)?;
    let children: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE parent_id = $1")
        .bind(id).fetch_one(&mut *tx).await?;
    if children > 0 {
        return Err(CatError::Conflict("Delete or move its subcategories first".into()));
    }
    if let Some(target) = move_to {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
            .bind(target).fetch_one(&mut *tx).await?;
        if !ok {
            return Err(CatError::BadRequest(format!("Unknown category {target}")));
        }
    }
    let moved: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT document_id) FROM documents WHERE category_id = $1")
        .bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE documents SET category_id = $2 WHERE category_id = $1")
        .bind(id).bind(move_to).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM categories WHERE id = $1").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(moved)
}

// ── HTTP ──────────────────────────────────────────────────────────

fn require_admin(req: &HttpRequest, state: &AppState) -> Result<JwtClaims, HttpResponse> {
    let claims = require_auth(req, &state.jwt_secret)?;
    if claims.role != "admin" {
        return Err(HttpResponse::Forbidden().json(ApiError::new(403, "Hanya admin")));
    }
    Ok(claims)
}

/// GET /api/admin/categories — every category with its document count, plus how many
/// documents are still uncategorized.
pub async fn list_handler(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let pool = state.db.pool();
    match (list(pool).await, uncategorized_count(pool).await) {
        (Ok(categories), Ok(uncategorized)) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({
            "categories": categories,
            "uncategorized": uncategorized,
        }))),
        (Err(e), _) | (_, Err(e)) => crate::models::internal_error(e),
    }
}

#[derive(Deserialize)]
pub struct SetCategoryBody {
    /// null = uncategorized
    pub category_id: Option<i64>,
}

/// PATCH /api/admin/documents/{id}/category — {"category_id": 7} or {"category_id": null}
pub async fn set_document_category_handler(
    req:   HttpRequest,
    state: web::Data<AppState>,
    path:  web::Path<String>,
    body:  web::Json<SetCategoryBody>,
) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    let pool = state.db.pool();
    if let Some(id) = body.category_id {
        match exists(pool, id).await {
            Ok(true) => {}
            Ok(false) => return HttpResponse::BadRequest().json(ApiError::new(400, format!("Unknown category {id}"))),
            Err(e) => return crate::models::internal_error(e),
        }
    }
    match set_document_category(pool, &path.into_inner(), body.category_id).await {
        Ok(0) => HttpResponse::NotFound().json(ApiError::new(404, "Document not found")),
        Ok(n) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "updated_chunks": n }))),
        Err(e) => crate::models::internal_error(e),
    }
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub name: String,
    /// null / missing = top level
    #[serde(default)]
    pub parent_id: Option<i64>,
}

/// POST /api/admin/categories — {"name": "Beasiswa", "parent_id": 1}
pub async fn create_handler(req: HttpRequest, state: web::Data<AppState>, body: web::Json<CreateBody>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match create(state.db.pool(), &body.name, body.parent_id).await {
        Ok(id) => HttpResponse::Created().json(ApiSuccess::new(serde_json::json!({ "id": id }))),
        Err(e) => e.response(),
    }
}

#[derive(Deserialize)]
pub struct RenameBody {
    pub name: String,
}

/// PATCH /api/admin/categories/{id} — {"name": "New name"}; the slug is kept.
pub async fn rename_handler(
    req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>, body: web::Json<RenameBody>,
) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match rename(state.db.pool(), path.into_inner(), &body.name).await {
        Ok(()) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "renamed": true }))),
        Err(e) => e.response(),
    }
}

#[derive(Deserialize)]
pub struct ReorderBody {
    #[serde(default)]
    pub parent_id: Option<i64>,
    pub ids: Vec<i64>,
}

/// PUT /api/admin/categories/order — {"parent_id": null, "ids": [3, 1, 2]}
pub async fn reorder_handler(req: HttpRequest, state: web::Data<AppState>, body: web::Json<ReorderBody>) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match reorder(state.db.pool(), body.parent_id, &body.ids).await {
        Ok(()) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "reordered": true }))),
        Err(e) => e.response(),
    }
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    /// category that receives the documents (merge); missing = they become uncategorized
    pub move_to: Option<i64>,
}

/// DELETE /api/admin/categories/{id}?move_to=7
pub async fn delete_handler(
    req: HttpRequest, state: web::Data<AppState>, path: web::Path<i64>, query: web::Query<DeleteQuery>,
) -> HttpResponse {
    if let Err(r) = require_admin(&req, &state) { return r; }
    match delete(state.db.pool(), path.into_inner(), query.move_to).await {
        Ok(moved) => HttpResponse::Ok().json(ApiSuccess::new(serde_json::json!({ "moved_documents": moved }))),
        Err(e) => e.response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn slugs_are_readable_and_unique() {
        assert_eq!(slugify("Kelulusan & Wisuda"), "kelulusan-wisuda");
        assert_eq!(slugify("Sumber Daya Manusia"), "sumber-daya-manusia");
        let mut seen = HashSet::new();
        for (parent, children) in DEFAULT_TAXONOMY {
            assert!(seen.insert(slugify(parent)), "duplicate slug for {parent}");
            for child in *children {
                assert!(seen.insert(child_slug(parent, child)), "duplicate slug for {child}");
            }
        }
    }

    #[test]
    fn every_legacy_target_exists_in_the_taxonomy() {
        let slugs: HashSet<String> = DEFAULT_TAXONOMY.iter()
            .flat_map(|(p, cs)| cs.iter().map(move |c| child_slug(p, c)))
            .collect();
        for (cat, sub, parent, child) in LEGACY_MAP {
            assert!(slugs.contains(&child_slug(parent, child)), "{cat} / {sub} → {parent} › {child} is not a category");
        }
    }

    #[test]
    fn known_pairs_are_mapped_case_insensitively() {
        assert_eq!(legacy_target("Panduan", "Layanan Persuratan").as_deref(), Some("akademik/persuratan"));
        assert_eq!(legacy_target(" informasi ", "rata-rata beban dosen").as_deref(), Some("sumber-daya-manusia/dosen"));
        assert_eq!(legacy_target("Website", "Profil").as_deref(), Some("sumber-daya-manusia/dosen"));
    }

    #[test]
    fn subcategory_matching_a_category_name_is_mapped() {
        // Older documents used "Akademik / Kurikulum"
        assert_eq!(legacy_target("Akademik", "Kurikulum").as_deref(), Some("akademik/kurikulum"));
        assert_eq!(legacy_target("", "Tenaga Kependidikan").as_deref(), Some("sumber-daya-manusia/tenaga-kependidikan"));
    }

    #[test]
    fn names_are_cleaned_and_checked() {
        assert_eq!(clean_name("  Beasiswa ").unwrap(), "Beasiswa");
        assert!(matches!(clean_name("   "), Err(CatError::BadRequest(_))));
        assert!(matches!(clean_name("&&"), Err(CatError::BadRequest(_))));
        assert!(matches!(clean_name(&"a".repeat(MAX_NAME_LEN + 1)), Err(CatError::BadRequest(_))));
    }

    #[test]
    fn unknown_values_stay_uncategorized() {
        assert_eq!(legacy_target("Umum", "—"), None);
        assert_eq!(legacy_target("Informasi", ""), None);
        assert_eq!(legacy_target("", ""), None);
    }
}
