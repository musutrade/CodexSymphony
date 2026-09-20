//! Generation intent and result commit independently of execution ownership.
use crate::{
    budget::Usage,
    draft_api,
    generation::{self, Request},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
pub type Error = (axum::http::StatusCode, axum::Json<Value>);
pub type Result<T> = std::result::Result<T, Error>;
pub fn error(status: u16, message: impl ToString) -> Error {
    (
        axum::http::StatusCode::from_u16(status)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(json!({"error":message.to_string()})),
    )
}
pub fn db(_: impl std::fmt::Display) -> Error {
    error(503, "generation persistence unavailable")
}
pub fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
pub async fn read(pool: &PgPool, id: &str) -> Result<Value> {
    sqlx::query_scalar("SELECT to_jsonb(g) FROM draft_generation g WHERE id=$1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(db)?
        .ok_or_else(|| error(404, "generation not found"))
}
pub async fn list(pool: &PgPool) -> Result<Value> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(g) FROM draft_generation g ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(pool)
    .await
    .map_err(db)?;
    Ok(json!({"generations":rows}))
}
// Called only under the process-wide instance lock, before serving requests.
pub async fn recover(pool: &PgPool) -> Result<()> {
    sqlx::query("UPDATE draft_generation SET status='interrupted',error='Service restarted; outcome unknown. No automatic model retry.',usage=jsonb_set(usage,'{complete}','false'),completed_at=now() WHERE status='running'").execute(pool).await.map_err(db)?;
    Ok(())
}
pub async fn admit(pool: &PgPool, request: &Request) -> Result<(Value, bool)> {
    request.validate().map_err(|e| error(422, e))?;
    let mut tx = pool.begin().await.map_err(db)?;
    // Serialize only generation admissions; this never locks execution_control.
    sqlx::query("SELECT pg_advisory_xact_lock(61002)")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let (input, fingerprint) = identity(request)?;
    if let Some(existing) = existing(&mut tx, &request.request_id, &fingerprint).await? {
        return replay_matches(request, existing).map(replayed);
    }
    ensure_idle(&mut tx).await?;
    let id = request
        .draft_id
        .clone()
        .unwrap_or_else(|| format!("draft-{}", request.request_id));
    draft_api::expected(&mut tx, &id, request.version, request.draft_id.is_none()).await?;
    insert(&mut tx, request, &id, input, fingerprint).await?;
    committed(pool, tx, &request.request_id).await
}
fn replayed(record: Value) -> (Value, bool) {
    (record, false)
}
async fn committed(
    pool: &PgPool,
    tx: Transaction<'_, Postgres>,
    id: &str,
) -> Result<(Value, bool)> {
    tx.commit().await.map_err(db)?;
    Ok((read(pool, id).await?, true))
}
async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    request: &Request,
    id: &str,
    input: Value,
    fingerprint: String,
) -> Result<()> {
    sqlx::query("INSERT INTO draft_generation(id,request,fingerprint,draft_id,input_version,status,usage,limits) VALUES($1,$2,$3,$4,$5,'running',$6,$7)")
        .bind(&request.request_id).bind(input).bind(fingerprint).bind(id).bind(request.version).bind(json!(Usage::default())).bind(json!(generation::LIMITS)).execute(&mut **tx).await.map_err(db)?;
    Ok(())
}
async fn ensure_idle(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    let active: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM draft_generation WHERE status='running')")
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
    if active {
        return Err(error(
            409,
            "another draft generation is running; no model call started",
        ));
    }
    Ok(())
}
fn identity(request: &Request) -> Result<(Value, String)> {
    let input = serde_json::to_value(request).map_err(db)?;
    let mut content = input.clone();
    content.as_object_mut().unwrap().remove("request_id");
    Ok((input, hash(content.to_string().as_bytes())))
}
fn replay_matches(request: &Request, existing: Value) -> Result<Value> {
    if existing["id"] == request.request_id
        && existing["request"] != serde_json::to_value(request).map_err(db)?
    {
        return Err(error(
            409,
            "request_id already binds different generation input",
        ));
    }
    Ok(existing)
}
pub async fn replay(pool: &PgPool, request: &Request) -> Result<Option<Value>> {
    let (_, fingerprint) = identity(request)?;
    let mut tx = pool.begin().await.map_err(db)?;
    existing(&mut tx, &request.request_id, &fingerprint)
        .await?
        .map(|record| replay_matches(request, record))
        .transpose()
}
async fn existing(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    fingerprint: &str,
) -> Result<Option<Value>> {
    sqlx::query_scalar("SELECT to_jsonb(g) FROM draft_generation g WHERE id=$1 OR fingerprint=$2 ORDER BY (id=$1) DESC LIMIT 1").bind(id).bind(fingerprint).fetch_optional(&mut **tx).await.map_err(db)
}
pub async fn progress(pool: &PgPool, id: &str, usage: &Usage, evidence: &Value) -> Result<()> {
    sqlx::query(
        "UPDATE draft_generation SET usage=$2,evidence=$3 WHERE id=$1 AND status='running'",
    )
    .bind(id)
    .bind(json!(usage))
    .bind(evidence)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}
pub async fn fail(
    pool: &PgPool,
    id: &str,
    status: &str,
    message: &str,
    output: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE draft_generation SET status=$2,error=$3,output=$4,completed_at=now() WHERE id=$1 AND status='running'").bind(id).bind(status).bind(message).bind(output).execute(pool).await.map_err(db)?;
    Ok(())
}
pub async fn complete(pool: &PgPool, id: &str, output: String) -> Result<()> {
    let document = generation::document(id, output.clone()).map_err(|e| error(422, e))?;
    let mut tx = pool.begin().await.map_err(db)?;
    let row:(String,i64,Value)=sqlx::query_as("SELECT draft_id,input_version,request FROM draft_generation WHERE id=$1 AND status='running' FOR UPDATE").bind(id).fetch_one(&mut *tx).await.map_err(db)?;
    let version = draft_api::expected(&mut tx, &row.0, row.1, row.2["draft_id"].is_null()).await?;
    draft_api::repositories(&mut tx, &document).await?;
    draft_api::persist(
        &mut tx,
        row.0,
        version,
        document,
        generation::source(id, output.clone()),
    )
    .await?;
    sqlx::query("UPDATE draft_generation SET status='succeeded',output=$2,output_version=$3,completed_at=now() WHERE id=$1").bind(id).bind(output).bind(version).execute(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}
