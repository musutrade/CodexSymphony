//! Draft persistence is separate from the executable single-requirement queue.
use crate::draft::{self, Document, Source};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

type Error = (StatusCode, Json<Value>);
type Result<T> = std::result::Result<T, Error>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub version: i64,
    pub source: Source,
}
#[derive(Serialize)]
pub struct View {
    pub id: String,
    pub version: i64,
    pub state: &'static str,
    pub document: Document,
    pub source: Source,
    pub source_sha256: String,
    pub warnings: Vec<String>,
}
fn error(code: StatusCode, message: impl ToString) -> Error {
    (code, Json(json!({"error":message.to_string()})))
}
fn db(_: impl std::fmt::Display) -> Error {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "draft persistence unavailable",
    )
}
fn invalid(message: impl ToString) -> Error {
    error(StatusCode::UNPROCESSABLE_ENTITY, message)
}
pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/drafts", get(list).post(create))
        .route("/api/drafts/{id}", get(read).put(update))
}
fn decode(
    input: std::result::Result<Json<Write>, axum::extract::rejection::JsonRejection>,
) -> Result<Write> {
    match input {
        Ok(Json(value)) => Ok(value),
        Err(_) => Err(invalid(
            "invalid draft request; expected version and source",
        )),
    }
}
async fn create(
    State(pool): State<PgPool>,
    input: std::result::Result<Json<Write>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<View>> {
    let id = format!("draft-{}", crate::process::new_identity().map_err(db)?);
    save(&pool, id, decode(input)?, true).await.map(Json)
}
async fn update(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
    input: std::result::Result<Json<Write>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<View>> {
    save(&pool, id, decode(input)?, false).await.map(Json)
}
async fn save(pool: &PgPool, id: String, input: Write, create: bool) -> Result<View> {
    let document = draft::parse(&input.source).map_err(invalid)?;
    let mut tx = pool.begin().await.map_err(db)?;
    let version = expected(&mut tx, &id, input.version, create).await?;
    repositories(&mut tx, &document).await?;
    let result = persist(&mut tx, id, version, document, input.source).await?;
    tx.commit().await.map_err(db)?;
    Ok(result)
}
pub(crate) async fn persist(
    tx: &mut Transaction<'_, Postgres>,
    id: String,
    version: i64,
    document: Document,
    source: Source,
) -> Result<View> {
    let document_json = serde_json::to_value(&document).map_err(invalid)?;
    let source_json = serde_json::to_value(&source).map_err(invalid)?;
    let hash = format!("{:x}", Sha256::digest(source.text.as_bytes()));
    sqlx::query("INSERT INTO imported_draft(id,version,document,source,source_sha256) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET version=excluded.version,document=excluded.document,source=excluded.source,source_sha256=excluded.source_sha256,updated_at=now()")
        .bind(&id).bind(version).bind(&document_json).bind(&source_json).bind(&hash).execute(&mut **tx).await.map_err(db)?;
    sqlx::query("INSERT INTO imported_draft_revision(draft_id,version,document,source,source_sha256) VALUES($1,$2,$3,$4,$5)")
        .bind(&id).bind(version).bind(document_json).bind(source_json).bind(&hash).execute(&mut **tx).await.map_err(db)?;
    crate::group_store::invalidate(tx, &id).await?;
    Ok(view(id, version, document, source, hash))
}
pub(crate) async fn expected(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    expected: i64,
    create: bool,
) -> Result<i64> {
    let current: Option<i64> =
        sqlx::query_scalar("SELECT version FROM imported_draft WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    let actual = match (create, current) {
        (true, None) => 0,
        (false, Some(value)) => value,
        _ => return Err(error(StatusCode::NOT_FOUND, "draft not found")),
    };
    if actual != expected {
        return Err(error(
            StatusCode::CONFLICT,
            "draft version conflict; reload before editing",
        ));
    }
    match actual.checked_add(1) {
        Some(next) => Ok(next),
        None => Err(invalid("draft version exhausted")),
    }
}
pub(crate) async fn repositories(
    tx: &mut Transaction<'_, Postgres>,
    document: &Document,
) -> Result<()> {
    for child in &document.children {
        if let Some(id) = child.repository_id {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repository WHERE id=$1)")
                    .bind(id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(db)?;
            if !exists {
                return Err(invalid(format!(
                    "{}: repository {id} is not registered",
                    child.id
                )));
            }
        }
    }
    Ok(())
}
fn view(
    id: String,
    version: i64,
    document: Document,
    source: Source,
    source_sha256: String,
) -> View {
    let mut warnings = draft::warnings(&document);
    if source.label.trim().is_empty() {
        warnings.push("source.label: 待补齐来源说明".into());
    }
    View {
        id,
        version,
        state: "Draft",
        document,
        source,
        source_sha256,
        warnings,
    }
}
async fn read(State(pool): State<PgPool>, Path(id): Path<String>) -> Result<Json<View>> {
    let row: Option<(i64, Value, Value, String)> = sqlx::query_as(
        "SELECT version,document,source,source_sha256 FROM imported_draft WHERE id=$1",
    )
    .bind(&id)
    .fetch_optional(&pool)
    .await
    .map_err(db)?;
    let (version, document, source, hash) =
        row.ok_or_else(|| error(StatusCode::NOT_FOUND, "draft not found"))?;
    Ok(Json(view(
        id,
        version,
        serde_json::from_value(document).map_err(invalid)?,
        serde_json::from_value(source).map_err(invalid)?,
        hash,
    )))
}
async fn list(State(pool): State<PgPool>) -> Result<Json<Value>> {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',id,'version',version,'state','Draft','goal',document->'parent'->>'goal') FROM imported_draft ORDER BY created_at DESC,id").fetch_all(&pool).await.map_err(db)?;
    Ok(Json(json!({"drafts":rows})))
}

/// Reject Draft identities before legacy numeric-ID extraction or controls.
pub async fn guard_legacy(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = request.uri().path();
    let legacy = path
        .strip_prefix("/api/requirements/")
        .or_else(|| path.strip_prefix("/api/multi/requirements/"));
    if legacy.is_some_and(|suffix| suffix.starts_with("draft-")) {
        return error(
            StatusCode::CONFLICT,
            "parent/child drafts require group review; legacy execution and Ready are unavailable",
        )
        .into_response();
    }
    next.run(request).await
}
