//! Business questions survive their originating RPC and app-server process.
use crate::{
    execution::RunKey,
    run_store, runtime,
    runtime_store::{self, Result, require},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Question {
    pub id: String,
    pub version: i64,
    pub requirement_id: i64,
    pub revision: i64,
    pub run_id: String,
    pub rpc_id: Value,
    pub original: Value,
    pub created_at: i64,
    pub answer: Option<Value>,
    pub resume_state: String,
    pub resumed_run: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub version: i64,
    pub answer: Value,
}

pub async fn ask(pool: &PgPool, key: &RunKey, original: &Value, now: i64) -> Result<Question> {
    validate(original)?;
    let mut tx = run_store::lock(pool).await?;
    let existing = existing(&mut tx, key, original).await?;
    if let Some(existing) = existing {
        require(existing.original == *original, "question RPC id conflict")?;
        return Ok(existing);
    }
    admission(&mut tx, key, original).await?;
    let question = insert(&mut tx, key, original, now).await?;
    tx.commit().await?;
    Ok(question)
}
fn validate(original: &Value) -> Result<()> {
    require(
        runtime::rpc_id_valid(&original["id"]),
        "invalid question RPC id",
    )?;
    require(
        original.to_string().len() <= runtime::MAX_REQUEST,
        "question too large",
    )?;
    let params: crate::runtime_protocol::ToolRequestUserInputParams =
        serde_json::from_value(original["params"].clone())
            .map_err(|_| runtime_store::invalid("invalid question parameters"))?;
    require(
        !params.questions.is_empty() && params.questions.len() <= 16,
        "invalid question count",
    )?;
    validate_questions(&params.questions)
}
fn validate_questions(questions: &[Value]) -> Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    for question in questions {
        let id = question["id"].as_str().unwrap_or("");
        require(
            runtime::text_valid(id, 200) && ids.insert(id.to_owned()),
            "invalid question identity",
        )?;
        question_text(question)?;
    }
    Ok(())
}
pub async fn list(pool: &PgPool, requirement: i64) -> Result<Vec<Question>> {
    sqlx::query_as("SELECT * FROM runtime_question WHERE requirement_id=$1 ORDER BY created_at,id")
        .bind(requirement)
        .fetch_all(pool)
        .await
}
pub async fn answer(pool: &PgPool, id: &str, answer: &Answer, now: i64) -> Result<Question> {
    require(
        answer.answer.to_string().len() <= runtime::MAX_REQUEST,
        "answer too large",
    )?;
    let mut tx = run_store::lock(pool).await?;
    let question: Question =
        sqlx::query_as("SELECT * FROM runtime_question WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    answer_version(&question, answer)?;
    answer_allowed(&mut tx, &question, answer).await?;
    let updated=sqlx::query_as("UPDATE runtime_question SET answer=$2,answered_at=$3,resume_state='pending' WHERE id=$1 AND answer IS NULL AND version=$4 RETURNING *")
        .bind(id).bind(&answer.answer).bind(now).bind(answer.version).fetch_one(&mut *tx).await?;
    // Do not clear pause, cancellation, any blocker or budget exhaustion here.
    tx.commit().await?;
    Ok(updated)
}

pub async fn live_answers(pool: &PgPool, key: &RunKey, now: i64) -> Result<Vec<Question>> {
    let mut tx = run_store::lock(pool).await?;
    if !runtime_store::allowed(&mut tx, key).await? {
        return Ok(Vec::new());
    }
    let (connected, created, waiting): (bool, i64, Option<i64>) = sqlx::query_as(
        "SELECT connected,created_at,waiting_since FROM runtime_session WHERE run_id=$1",
    )
    .bind(&key.run_id)
    .fetch_one(&mut *tx)
    .await?;
    if !connected || runtime::expired(now, created, waiting) {
        return Ok(Vec::new());
    }
    sqlx::query_as("SELECT * FROM runtime_question WHERE run_id=$1 AND resume_state='pending' ORDER BY created_at,id")
        .bind(&key.run_id).fetch_all(&mut *tx).await
}
pub async fn delivered(pool: &PgPool, id: &str) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let run:String=sqlx::query_scalar("UPDATE runtime_question SET resume_state='live' WHERE id=$1 AND resume_state='pending' RETURNING run_id")
        .bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE runtime_session SET waiting_since=NULL WHERE run_id=$1 AND NOT EXISTS(SELECT 1 FROM runtime_question WHERE run_id=$1 AND resume_state IN ('waiting','pending'))")
        .bind(run).execute(&mut *tx).await?;
    tx.commit().await
}

pub async fn expire(pool: &PgPool, now: i64) -> Result<()> {
    sqlx::query("UPDATE agent_run a SET stop_requested=true,blocker='runtime_timeout: stop and preserve; question remains answerable' FROM runtime_session s WHERE a.id=s.run_id AND NOT a.quiescent AND ($1-s.created_at>=28800 OR $1-s.waiting_since>=7200)")
        .bind(now).execute(pool).await?;
    sqlx::query("UPDATE runtime_session s SET connected=false FROM agent_run a,execution_control c WHERE s.run_id=a.id AND (a.quiescent OR a.stop_requested OR a.incarnation<>c.incarnation)")
        .execute(pool).await?;
    Ok(())
}

/// Only a new, independently preflighted launch can consume a saved answer's
/// recovery intent. The Requirement keeps its existing owner and budget account.
pub async fn reserve_resume(
    pool: &PgPool,
    source: &str,
    launch: &crate::execution::Launch,
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let source = resumable(&mut tx, source, &launch.key.incarnation).await?;
    let Some((id, revision)) = source else {
        return Ok(false);
    };
    if !resume_admitted(&mut tx, launch, id, revision).await? {
        return Ok(false);
    }
    link_resume(&mut tx, id, revision, launch).await?;
    tx.commit().await?;
    Ok(true)
}

pub(crate) async fn resumable(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: &str,
    incarnation: &str,
) -> Result<Option<(i64, i64)>> {
    sqlx::query_as("SELECT a.requirement_id,a.revision FROM agent_run a JOIN requirement r ON r.id=a.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=a.revision CROSS JOIN repository p WHERE a.id=$1 AND a.quiescent AND a.phase='execution' AND r.state IN ('Running','Failed') AND r.revision=a.revision AND NOT r.paused AND NOT c.paused AND c.recovery_complete AND c.incarnation=$2 AND p.id=1 AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND EXISTS(SELECT 1 FROM workspace_snapshot WHERE run_id=a.id) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE requirement_id=r.id AND NOT quiescent) AND NOT EXISTS(SELECT 1 FROM runtime_blocker b JOIN agent_run old ON old.id=b.run_id WHERE old.requirement_id=r.id AND NOT b.resolved) AND (a.user_paused OR a.storage_resume_requested OR EXISTS(SELECT 1 FROM runtime_question q WHERE q.run_id=a.id AND q.resume_state='pending')) AND NOT EXISTS(SELECT 1 FROM runtime_question q WHERE q.requirement_id=r.id AND q.resume_state='waiting') AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') AND NOT EXISTS(SELECT 1 FROM agent_run newer WHERE newer.requirement_id=r.id AND newer.run_sequence>a.run_sequence)")
        .bind(source).bind(incarnation).fetch_optional(&mut **tx).await
}

async fn existing(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &RunKey,
    original: &Value,
) -> Result<Option<Question>> {
    let identity: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_run WHERE id=$1 AND request_id=$2 AND incarnation=$3)",
    )
    .bind(&key.run_id)
    .bind(&key.request_id)
    .bind(&key.incarnation)
    .fetch_one(&mut **tx)
    .await?;
    require(identity, "question Run identity mismatch")?;
    let existing: Option<Question> =
        sqlx::query_as("SELECT * FROM runtime_question WHERE run_id=$1 AND rpc_id=$2")
            .bind(&key.run_id)
            .bind(&original["id"])
            .fetch_optional(&mut **tx)
            .await?;
    Ok(existing)
}

async fn admission(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &RunKey,
    original: &Value,
) -> Result<()> {
    require(runtime_store::allowed(tx, key).await?, "Run unavailable")?;
    let current: bool = sqlx::query_scalar(
        "SELECT connected AND thread_id=$2 AND turn_id=$3 FROM runtime_session WHERE run_id=$1",
    )
    .bind(&key.run_id)
    .bind(original["params"]["threadId"].as_str())
    .bind(original["params"]["turnId"].as_str())
    .fetch_one(&mut **tx)
    .await?;
    require(current, "question from stale session")?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runtime_question WHERE run_id=$1")
        .bind(&key.run_id)
        .fetch_one(&mut **tx)
        .await?;
    require(count < 32, "question count limit")?;
    Ok(())
}

async fn answer_allowed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    question: &Question,
    answer: &Answer,
) -> Result<()> {
    let authorized:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=r.revision CROSS JOIN repository p WHERE r.id=$1 AND r.revision=$2 AND r.state IN ('Running','Failed') AND p.id=1 AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version)")
        .bind(question.requirement_id).bind(question.revision).fetch_one(&mut **tx).await?;
    require(authorized, "question authorization or revision invalid")?;
    require(
        runtime::validate_answers(&question.original, &answer.answer),
        "answers do not match original questions",
    )?;
    Ok(())
}

async fn link_resume(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    revision: i64,
    launch: &crate::execution::Launch,
) -> Result<()> {
    crate::run_store::insert_run(tx, id, revision, launch).await?;
    sqlx::query("INSERT INTO run_workspace(run_id,identity,restored_from) SELECT $1,job->'workspace',source_run FROM runtime_resume WHERE job->'launch'=$2 AND status='prepared'")
        .bind(&launch.key.run_id).bind(serde_json::json!(launch)).execute(&mut **tx).await?;
    sqlx::query("UPDATE runtime_resume SET status='dispatched' WHERE job->'launch'=$1 AND status='prepared'")
        .bind(serde_json::json!(launch)).execute(&mut **tx).await?;
    sqlx::query("UPDATE runtime_question SET resume_state='linked',resumed_run=$2 WHERE requirement_id=$1 AND resume_state='pending'")
        .bind(id).bind(&launch.key.run_id).execute(&mut **tx).await?;
    sqlx::query("UPDATE requirement SET state='Running',version=version+1 WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn question_text(question: &Value) -> Result<()> {
    require(
        question["question"]
            .as_str()
            .is_some_and(|text| runtime::text_valid(text, 8192)),
        "invalid question text",
    )?;
    Ok(())
}

async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &RunKey,
    original: &Value,
    now: i64,
) -> Result<Question> {
    let id = crate::process::new_identity().map_err(sqlx::Error::Io)?;
    let question:Question=sqlx::query_as("INSERT INTO runtime_question(id,requirement_id,revision,run_id,rpc_id,original,created_at) SELECT $2,requirement_id,revision,id,$3,$4,$5 FROM agent_run WHERE id=$1 RETURNING *")
        .bind(&key.run_id).bind(id).bind(&original["id"]).bind(original).bind(now).fetch_one(&mut **tx).await?;
    sqlx::query(
        "UPDATE runtime_session SET waiting_since=COALESCE(waiting_since,$2) WHERE run_id=$1",
    )
    .bind(&key.run_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(question)
}

fn answer_version(question: &Question, answer: &Answer) -> Result<()> {
    require(
        question.version == answer.version,
        "question version changed",
    )?;
    require(
        question.answer.is_none() && question.resume_state == "waiting",
        "question already answered or invalid",
    )?;
    Ok(())
}

pub(crate) async fn budget_available(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<bool> {
    let balance = crate::budget_store::balance(tx, id).await?;
    Ok(!balance.exhausted && !balance.exposure.reached(balance.limits))
}

async fn resume_admitted(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    launch: &crate::execution::Launch,
    id: i64,
    revision: i64,
) -> Result<bool> {
    Ok(
        crate::preparation_store::claim_ready(tx, launch, id, revision).await?
            && budget_available(tx, id).await?,
    )
}
