//! Transactional Requirement truth. No runtime or GitHub facts are manufactured.
use crate::contract::{self, Contract, Repository};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};

type Tx<'a> = Transaction<'a, Postgres>;
type Result<T> = std::result::Result<T, ApiError>;
pub struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(_: serde_json::Error) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "stored record unavailable")
    }
}
impl From<&'static str> for ApiError {
    fn from(message: &'static str) -> Self {
        Self(StatusCode::UNPROCESSABLE_ENTITY, message)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    request_id: String,
    version: i64,
    contract: Contract,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RepositoryRequest {
    request_id: String,
    version: i64,
    repository: Repository,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ControlRequest {
    request_id: String,
    version: i64,
    repository_version: i64,
}
#[derive(Serialize)]
#[serde(tag = "operation")]
enum Command {
    Repository(RepositoryRequest),
    Create(DraftRequest),
    Edit { id: i64, input: DraftRequest },
    Ready { id: i64, input: ControlRequest },
    Withdraw { id: i64, input: ControlRequest },
}
impl Command {
    fn key(&self) -> &str {
        match self {
            Self::Repository(r) => &r.request_id,
            Self::Create(r) | Self::Edit { input: r, .. } => &r.request_id,
            Self::Ready { input: r, .. } | Self::Withdraw { input: r, .. } => &r.request_id,
        }
    }
}
pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/repository", get(repository).put(configure))
        .route("/api/requirements", get(list).post(create))
        .route("/api/requirements/{id}", get(detail).patch(edit))
        .route("/api/requirements/{id}/ready", post(ready))
        .route("/api/requirements/{id}/withdraw", post(withdraw))
}
fn decode<T>(
    input: std::result::Result<Json<T>, axum::extract::rejection::JsonRejection>,
) -> Result<T> {
    match input {
        Ok(Json(value)) => Ok(value),
        Err(_) => Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid JSON request",
        )),
    }
}
async fn repository(State(pool): State<PgPool>) -> Result<Json<Value>> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT jsonb_build_object('version',version,'repository',document)::text FROM repository",
    )
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        json!({"repositories": decode_all(rows)?, "deployment_network": [], "network_status": "not_configured", "runtime_ready": false, "repository_ready": false}),
    ))
}
fn decode_all(rows: Vec<String>) -> Result<Vec<Value>> {
    Ok(rows
        .iter()
        .map(String::as_str)
        .map(serde_json::from_str)
        .collect::<serde_json::Result<_>>()?)
}
async fn configure(
    State(pool): State<PgPool>,
    input: std::result::Result<Json<RepositoryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    execute(&pool, Command::Repository(decode(input)?)).await
}
async fn create(
    State(pool): State<PgPool>,
    input: std::result::Result<Json<DraftRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>)> {
    Ok((
        StatusCode::CREATED,
        execute(&pool, Command::Create(decode(input)?)).await?,
    ))
}
async fn edit(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<Json<DraftRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    execute(
        &pool,
        Command::Edit {
            id,
            input: decode(input)?,
        },
    )
    .await
}
async fn ready(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<Json<ControlRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    execute(
        &pool,
        Command::Ready {
            id,
            input: decode(input)?,
        },
    )
    .await
}
async fn withdraw(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<Json<ControlRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    execute(
        &pool,
        Command::Withdraw {
            id,
            input: decode(input)?,
        },
    )
    .await
}
async fn list(State(pool): State<PgPool>) -> Result<Json<Value>> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT jsonb_build_object('id',id,'version',version,'state',state,'title',contract->>'title','revision',revision)::text FROM requirement ORDER BY id DESC").fetch_all(&pool).await?;
    Ok(Json(json!({"requirements":decode_all(rows)?})))
}
async fn detail(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Value>> {
    let mut tx = pool.begin().await?;
    Ok(Json(read_requirement(&mut tx, id).await?))
}
async fn execute(pool: &PgPool, command: Command) -> Result<Json<Value>> {
    let key = command.key();
    contract::validate_request_id(key)?;
    let input = serde_json::to_string(&command)?;
    let mut tx = lock(pool).await?;
    if let Some(result) = replay(&mut tx, key, &input).await? {
        return Ok(Json(result));
    }
    let result = apply(&mut tx, &command).await?;
    sqlx::query(
        "INSERT INTO business_request(request_id,input,result) VALUES ($1,$2::jsonb,$3::jsonb)",
    )
    .bind(key)
    .bind(input)
    .bind(result.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(result))
}
async fn lock(pool: &PgPool) -> Result<Tx<'_>> {
    let mut tx = pool.begin().await?;
    // Single repository, low-volume control plane. Serializes configuration,
    // revocation and review in the same transaction; this is not a Run lock.
    sqlx::query("SELECT pg_advisory_xact_lock(13002)")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

async fn replay(tx: &mut Tx<'_>, key: &str, input: &str) -> Result<Option<Value>> {
    let row: Option<(bool, String)> = sqlx::query_as(
        "SELECT input=$2::jsonb,result::text FROM business_request WHERE request_id=$1",
    )
    .bind(key)
    .bind(input)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        None => Ok(None),
        Some((true, result)) => Ok(Some(serde_json::from_str(&result)?)),
        Some((false, _)) => Err(ApiError(
            StatusCode::CONFLICT,
            "request_id reused with different input",
        )),
    }
}
async fn apply(tx: &mut Tx<'_>, command: &Command) -> Result<Value> {
    match command {
        Command::Repository(input) => save_repository(tx, input).await,
        Command::Create(input) => new_requirement(tx, input).await,
        Command::Edit { id, input } => edit_requirement(tx, *id, input).await,
        Command::Ready { id, input } => review(tx, *id, input).await,
        Command::Withdraw { id, input } => unready(tx, *id, input).await,
    }
}
fn version(actual: i64, expected: i64) -> Result<()> {
    if actual != expected {
        return Err(ApiError(StatusCode::CONFLICT, "stale object version"));
    }
    Ok(())
}
async fn save_repository(tx: &mut Tx<'_>, input: &RepositoryRequest) -> Result<Value> {
    contract::validate_repository(&input.repository)?;
    let current: Option<i64> = sqlx::query_scalar("SELECT version FROM repository WHERE id=1")
        .fetch_optional(&mut **tx)
        .await?;
    version(current.unwrap_or(0), input.version)?;
    check_identity(tx, current, &input.repository).await?;
    let next = input.version + 1;
    sqlx::query("INSERT INTO repository(id,version,document) VALUES (1,$1,$2::jsonb) ON CONFLICT(id) DO UPDATE SET version=$1,document=$2::jsonb,revoked_through_version=CASE WHEN ($2::jsonb->>'revoked')::boolean THEN $1 ELSE repository.revoked_through_version END")
        .bind(next).bind(serde_json::to_string(&input.repository)?).execute(&mut **tx).await?;
    event(tx, "repository:1", "policy_updated", next).await?;
    Ok(json!({"version":next,"repository":input.repository}))
}
async fn check_identity(tx: &mut Tx<'_>, current: Option<i64>, next: &Repository) -> Result<()> {
    if current.is_some() {
        let (_, previous) = current_repository(tx).await?;
        contract::require(
            previous.remote == next.remote
                && previous.github_repository_id == next.github_repository_id,
            "only one repository identity is enabled",
        )?;
    }
    Ok(())
}
async fn event(tx: &mut Tx<'_>, id: &str, kind: &str, version: i64) -> Result<()> {
    sqlx::query("INSERT INTO business_event(object_id,kind,version) VALUES ($1,$2,$3)")
        .bind(id)
        .bind(kind)
        .bind(version)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn new_requirement(tx: &mut Tx<'_>, input: &DraftRequest) -> Result<Value> {
    version(0, input.version)?;
    contract::validate_contract(&input.contract)?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO requirement(version,state,contract) VALUES (1,'Draft',$1::jsonb) RETURNING id",
    )
    .bind(serde_json::to_string(&input.contract)?)
    .fetch_one(&mut **tx)
    .await?;
    event(tx, &format!("requirement:{id}"), "created", 1).await?;
    read_requirement(tx, id).await
}
async fn read_requirement(tx: &mut Tx<'_>, id: i64) -> Result<Value> {
    let row: Option<String> = sqlx::query_scalar("SELECT jsonb_build_object('id',id,'version',version,'state',state,'contract',contract,'revision',revision,'created_at',created_at::text,'creator','local-user','authorization_valid',COALESCE(state='Ready' AND (SELECT NOT (document->>'revoked')::boolean AND COALESCE((SELECT (document->>'repository_version')::bigint FROM requirement_revision WHERE requirement_id=r.id AND revision=r.revision),0)>revoked_through_version FROM repository WHERE id=1),false),'snapshots', COALESCE((SELECT jsonb_agg(document ORDER BY revision) FROM requirement_revision WHERE requirement_id=r.id),'[]'::jsonb))::text FROM requirement r WHERE id=$1")
        .bind(id).fetch_optional(&mut **tx).await?;
    Ok(serde_json::from_str(&row.ok_or(ApiError(
        StatusCode::NOT_FOUND,
        "requirement not found",
    ))?)?)
}
async fn editable(tx: &mut Tx<'_>, id: i64, expected: i64, state: &str) -> Result<Value> {
    let row = read_requirement(tx, id).await?;
    version(
        row["version"].as_i64().ok_or("invalid stored version")?,
        expected,
    )?;
    if row["state"] != state {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "action not permitted in current state",
        ));
    }
    Ok(row)
}
async fn edit_requirement(tx: &mut Tx<'_>, id: i64, input: &DraftRequest) -> Result<Value> {
    editable(tx, id, input.version, "Draft").await?;
    contract::validate_contract(&input.contract)?;
    sqlx::query("UPDATE requirement SET contract=$1::jsonb,version=version+1 WHERE id=$2")
        .bind(serde_json::to_string(&input.contract)?)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    event(
        tx,
        &format!("requirement:{id}"),
        "edited",
        input.version + 1,
    )
    .await?;
    read_requirement(tx, id).await
}
async fn review(tx: &mut Tx<'_>, id: i64, input: &ControlRequest) -> Result<Value> {
    let row = editable(tx, id, input.version, "Draft").await?;
    let (policy_version, repository) = current_repository(tx).await?;
    version(policy_version, input.repository_version)?;
    let contract: Contract = serde_json::from_value(row["contract"].clone())?;
    authorize_review(tx, id, &contract, &repository).await?;
    let revision = row["revision"].as_i64().ok_or("invalid stored revision")? + 1;
    let ac_ids: Vec<String> = (1..=contract.acceptance_criteria.len())
        .map(|n| format!("AC-{id}-{revision}-{n}"))
        .collect();
    let snapshot = json!({"revision":revision,"contract":contract,"ac_ids":ac_ids,"repository_version":policy_version,"repository":repository,"reviewer":"local-user"});
    sqlx::query("INSERT INTO requirement_revision(requirement_id,revision,document) VALUES ($1,$2,$3::jsonb)").bind(id).bind(revision).bind(snapshot.to_string()).execute(&mut **tx).await?;
    sqlx::query("UPDATE requirement SET state='Ready',version=version+1,revision=$2 WHERE id=$1")
        .bind(id)
        .bind(revision)
        .execute(&mut **tx)
        .await?;
    event(
        tx,
        &format!("requirement:{id}"),
        "reviewed_ready",
        input.version + 1,
    )
    .await?;
    read_requirement(tx, id).await
}
async fn authorize_review(
    tx: &mut Tx<'_>,
    id: i64,
    contract: &Contract,
    repository: &Repository,
) -> Result<()> {
    contract::authorize(contract, repository)?;
    crate::budget_store::freeze(tx, id, &repository.policy).await?;
    Ok(())
}
async fn current_repository(tx: &mut Tx<'_>) -> Result<(i64, Repository)> {
    let (version, document): (i64, String) =
        sqlx::query_as("SELECT version,document::text FROM repository WHERE id=1")
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(ApiError(
                StatusCode::CONFLICT,
                "repository is not configured",
            ))?;
    Ok((version, serde_json::from_str(&document)?))
}
async fn unready(tx: &mut Tx<'_>, id: i64, input: &ControlRequest) -> Result<Value> {
    editable(tx, id, input.version, "Ready").await?;
    sqlx::query("UPDATE requirement SET state='Draft',version=version+1 WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    event(
        tx,
        &format!("requirement:{id}"),
        "withdrawn",
        input.version + 1,
    )
    .await?;
    read_requirement(tx, id).await
}
