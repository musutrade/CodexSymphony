//! Durable protocol requests and mutually exclusive ending declarations.
use crate::{execution::RunKey, run_store, runtime};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
pub(crate) type Tx<'a> = Transaction<'a, Postgres>;
pub type Result<T> = std::result::Result<T, sqlx::Error>;
pub(crate) fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}
pub(crate) fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

/// Called while holding the same advisory lock as pause, revocation and Broker.
pub(crate) async fn allowed(tx: &mut Tx<'_>, key: &RunKey) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_run a JOIN requirement r ON r.id=a.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=a.revision CROSS JOIN repository p WHERE a.id=$1 AND a.request_id=$2 AND a.incarnation=$3 AND c.incarnation=a.incarnation AND c.recovery_complete AND NOT c.paused AND NOT r.paused AND r.state='Running' AND r.revision=a.revision AND NOT a.stop_requested AND NOT a.quiescent AND a.blocker IS NULL AND a.state IN ('Created','Running') AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version AND p.id=COALESCE((v.document->>'repository_id')::bigint,1) AND plugin_scope_allows('agent:codex',plugin_scope_repository(a.requirement_id,a.revision)) AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM runtime_session s WHERE s.run_id=a.id AND s.end_kind IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM runtime_blocker b JOIN agent_run old ON old.id=b.run_id WHERE old.requirement_id=r.id AND NOT b.resolved))")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).fetch_one(&mut **tx).await
}

pub async fn open(pool: &PgPool, key: &RunKey, now: i64) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    require(allowed(&mut tx, key).await?, "Run unavailable")?;
    sqlx::query("SELECT plugin_scope_admit('agent:codex',id,requirement_id,revision,plugin_scope_repository(requirement_id,revision)) FROM agent_run WHERE id=$1")
        .bind(&key.run_id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO runtime_session(run_id,created_at,last_progress) SELECT id,extract(epoch FROM created_at)::bigint,$2 FROM agent_run WHERE id=$1")
        .bind(&key.run_id).bind(now).execute(&mut *tx).await?;
    tx.commit().await
}
pub async fn thread(pool: &PgPool, key: &RunKey, thread: &str, now: i64) -> Result<()> {
    require(runtime::text_valid(thread, 200), "invalid thread identity")?;
    let mut tx = run_store::lock(pool).await?;
    require(allowed(&mut tx, key).await?, "Run unavailable")?;
    let changed=sqlx::query("UPDATE runtime_session SET thread_id=$2,connected=true,last_progress=$3 WHERE run_id=$1 AND thread_id IS NULL")
        .bind(&key.run_id).bind(thread).bind(now).execute(&mut *tx).await?;
    require(changed.rows_affected() == 1, "thread already bound")?;
    tx.commit().await
}
pub async fn turn(pool: &PgPool, key: &RunKey, turn: &str, call: &str, now: i64) -> Result<()> {
    require(runtime::text_valid(turn, 200), "invalid turn identity")?;
    let mut tx = run_store::lock(pool).await?;
    require(allowed(&mut tx, key).await?, "Run unavailable")?;
    sqlx::query("UPDATE runtime_session SET turn_id=$2,call_id=$3,last_progress=$4 WHERE run_id=$1 AND connected")
        .bind(&key.run_id).bind(turn).bind(call).bind(now).execute(&mut *tx).await?;
    tx.commit().await
}
pub async fn close(pool: &PgPool, key: &RunKey) -> Result<()> {
    sqlx::query("UPDATE runtime_session SET connected=false WHERE run_id=$1 AND EXISTS(SELECT 1 FROM agent_run WHERE id=$1 AND request_id=$2 AND incarnation=$3)")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).execute(pool).await?;
    Ok(())
}

pub async fn can_continue(pool: &PgPool, key: &RunKey, now: i64) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if !allowed(&mut tx, key).await? {
        return Ok(false);
    }
    let times: (i64, Option<i64>, bool) = sqlx::query_as(
        "SELECT created_at,waiting_since,connected FROM runtime_session WHERE run_id=$1",
    )
    .bind(&key.run_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(times.2 && times.1.is_none() && !runtime::expired(now, times.0, times.1))
}

/// A replay is returned before lifecycle admission, so even an ending request
/// receives its exact durable result. Reusing its id with other bytes is denied.
pub async fn request(pool: &PgPool, key: &RunKey, original: &Value) -> Result<Option<Value>> {
    require(
        original.to_string().len() <= runtime::MAX_REQUEST,
        "request too large",
    )?;
    require(runtime::rpc_id_valid(&original["id"]), "invalid RPC id")?;
    let mut tx = run_store::lock(pool).await?;
    if let Some(saved) = saved(&mut tx, key, original).await? {
        return Ok(Some(saved));
    }
    insert_request(&mut tx, key, original).await?;
    tx.commit().await?;
    Ok(None)
}
async fn saved(tx: &mut Tx<'_>, key: &RunKey, original: &Value) -> Result<Option<Value>> {
    let identity: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_run WHERE id=$1 AND request_id=$2 AND incarnation=$3)",
    )
    .bind(&key.run_id)
    .bind(&key.request_id)
    .bind(&key.incarnation)
    .fetch_one(&mut **tx)
    .await?;
    require(identity, "Run identity mismatch")?;
    let saved: Option<(Value, Option<Value>)> =
        sqlx::query_as("SELECT original,result FROM runtime_request WHERE run_id=$1 AND rpc_id=$2")
            .bind(&key.run_id)
            .bind(&original["id"])
            .fetch_optional(&mut **tx)
            .await?;
    match saved {
        Some((request, result)) => {
            require(request == *original, "RPC id reused with different request")?;
            require(
                result.is_some(),
                "request pending; reconcile original intent",
            )?;
            Ok(result)
        }
        None => Ok(None),
    }
}
async fn validate_session(tx: &mut Tx<'_>, key: &RunKey, original: &Value) -> Result<()> {
    let matches:bool=sqlx::query_scalar("SELECT connected AND thread_id=$2 AND turn_id=$3 AND end_kind IS NULL FROM runtime_session WHERE run_id=$1")
        .bind(&key.run_id).bind(original["params"]["threadId"].as_str()).bind(original["params"]["turnId"].as_str()).fetch_one(&mut **tx).await?;
    require(matches, "stale thread/turn request")
}
pub async fn result(pool: &PgPool, key: &RunKey, id: &Value, value: &Value) -> Result<()> {
    require(
        value.to_string().len() <= runtime::MAX_REQUEST,
        "response too large",
    )?;
    let changed = sqlx::query(
        "UPDATE runtime_request SET result=$3 WHERE run_id=$1 AND rpc_id=$2 AND result IS NULL",
    )
    .bind(&key.run_id)
    .bind(id)
    .bind(value)
    .execute(pool)
    .await?;
    require(changed.rows_affected() == 1, "request already resolved")
}

pub async fn end(
    pool: &PgPool,
    key: &RunKey,
    original: &Value,
    kind: &str,
    payload: &Value,
) -> Result<Value> {
    let mut tx = run_store::lock(pool).await?;
    ending_session(&mut tx, key, original).await?;
    ending_allowed(&mut tx, key, kind, payload).await?;
    let result = runtime::reply(
        true,
        "Ending intent persisted; execution group stop and preservation pending.",
    );
    save_ending(&mut tx, key, original, kind, payload, &result).await?;
    ending_blocker(&mut tx, key, kind, payload).await?;
    sqlx::query("UPDATE agent_run SET stop_requested=true WHERE id=$1")
        .bind(&key.run_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(result)
}
async fn candidate(tx: &mut Tx<'_>, key: &RunKey, payload: &Value) -> Result<()> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM run_workspace WHERE run_id=$1 AND candidate_sha=$2)",
    )
    .bind(&key.run_id)
    .bind(payload["candidate_sha"].as_str())
    .fetch_one(&mut **tx)
    .await?;
    require(valid, "completion does not identify Broker candidate")
}

pub async fn evidence(pool: &PgPool, key: &RunKey, channel: &str, bytes: &[u8]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO runtime_evidence(run_id,channel) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(&key.run_id)
    .bind(channel)
    .execute(&mut *tx)
    .await?;
    let (kept, records, expired): (i64, i32,Option<i64>) = sqlx::query_as(
        "SELECT kept_bytes,records,expired_at FROM runtime_evidence WHERE run_id=$1 AND channel=$2 FOR UPDATE",
    )
    .bind(&key.run_id)
    .bind(channel)
    .fetch_one(&mut *tx)
    .await?;
    let capacity = if records < runtime::MAX_RECORDS as i32 {
        runtime::MAX_EVIDENCE
            .saturating_sub(kept as usize)
            .min(runtime::MAX_REQUEST)
    } else {
        0
    };
    let policy: Option<Value> = sqlx::query_scalar("SELECT p.document FROM storage_guard g JOIN storage_policy p ON p.version=g.policy_version WHERE g.id=1")
        .fetch_optional(&mut *tx).await?;
    let capacity = if expired.is_some() {
        0
    } else {
        evidence_capacity(capacity, records, policy.as_ref())
    };
    let take = bytes.len().min(capacity);
    if take > 0 {
        sqlx::query("INSERT INTO runtime_evidence_chunk VALUES($1,$2,$3,$4)")
            .bind(&key.run_id)
            .bind(channel)
            .bind(records)
            .bind(&bytes[..take])
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE runtime_evidence SET kept_bytes=kept_bytes+$3,discarded_bytes=LEAST(9223372036854775807::numeric,discarded_bytes::numeric+$4)::bigint,records=records+$5,truncated=truncated OR $4>0 WHERE run_id=$1 AND channel=$2")
        .bind(&key.run_id).bind(channel).bind(take as i64).bind((bytes.len()-take) as i64).bind(i32::from(take>0)).execute(&mut *tx).await?;
    tx.commit().await
}

fn evidence_capacity(capacity: usize, records: i32, policy: Option<&Value>) -> usize {
    let Some(policy) = policy else {
        return capacity;
    };
    let count = policy["entry_count"].as_u64().unwrap_or(0);
    if records as u64 >= count {
        return 0;
    }
    // Leave room for the row identity and PostgreSQL tuple overhead.
    capacity.min(
        policy["entry_bytes"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(512) as usize,
    )
}

/// Independent success fact: declaration + exact candidate + complete group
/// receipt + preserved work. A turn notification cannot satisfy this query.
pub async fn finalize(pool: &PgPool) -> Result<()> {
    sqlx::query("UPDATE agent_run a SET state='Succeeded',phase='validation' FROM runtime_session s,workspace_snapshot snap,run_workspace w WHERE a.id=s.run_id AND a.id=snap.run_id AND a.id=w.run_id AND a.quiescent AND a.state='Interrupted' AND s.end_kind='completion' AND snap.candidate AND w.candidate_sha=s.end_payload->>'candidate_sha' AND snap.manifest->>'head'=w.candidate_sha")
        .execute(pool).await?;
    Ok(())
}

pub async fn stop(pool: &PgPool, key: &RunKey, reason: &str) -> Result<()> {
    close(pool, key).await?;
    run_store::block(pool, &key.run_id, reason).await
}
pub async fn input(pool: &PgPool, key: &RunKey) -> Result<String> {
    let document:Value=sqlx::query_scalar("SELECT COALESCE(i.document,v.document) FROM requirement_revision v JOIN agent_run a ON a.requirement_id=v.requirement_id AND a.revision=v.revision LEFT JOIN linked_run_input i ON i.run_id=a.id WHERE a.id=$1")
        .bind(&key.run_id).fetch_one(pool).await?;
    // Answers belong to the reviewed business revision. A subsequent pause or
    // storage recovery changes the Run again without invalidating those answers.
    // Keep resumed_run as historical delivery evidence, not a lifetime filter.
    let answers:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('question_id',q.id,'version',q.version,'original',q.original,'answer',q.answer,'source_run',q.run_id) FROM runtime_question q JOIN agent_run a ON a.requirement_id=q.requirement_id AND a.revision=q.revision WHERE a.id=$1 AND q.answer IS NOT NULL AND q.resume_state IN ('live','linked','pending') ORDER BY q.created_at,q.id")
        .bind(&key.run_id).fetch_all(pool).await?;
    let repair: Option<Value> =
        sqlx::query_scalar("SELECT failure FROM repair_reservation WHERE repair_run_id=$1")
            .bind(&key.run_id)
            .fetch_optional(pool)
            .await?;
    let constraints = crate::extension_recovery::constraints(pool, &key.run_id).await?;
    Ok(json!({"approved_adaptation_constraints":constraints,"reviewed_requirement":document,"confirmed_answers":answers,"repair_context":repair,"instruction":"Implement only the reviewed contract. Do not ask answered questions again. Use create_local_commit, then report_completion; report_blocker when unable to proceed. External delivery is platform-owned."}).to_string())
}

async fn ending_allowed(tx: &mut Tx<'_>, key: &RunKey, kind: &str, payload: &Value) -> Result<()> {
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM workspace_operation WHERE run_id=$1 AND status<>'complete')",
    )
    .bind(&key.run_id)
    .fetch_one(&mut **tx)
    .await?;
    require(!pending, "Broker operation must reconcile first")?;
    if kind == "completion" {
        candidate(tx, key, payload).await?;
    }
    Ok(())
}

async fn ending_blocker(tx: &mut Tx<'_>, key: &RunKey, kind: &str, payload: &Value) -> Result<()> {
    if kind == "blocker" {
        let code = if payload["requires_permission"] == true {
            "permission_expansion"
        } else {
            "agent_blocker"
        };
        sqlx::query("INSERT INTO runtime_blocker(run_id,code,detail) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(&key.run_id).bind(code).bind(payload).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn request_capacity(tx: &mut Tx<'_>, key: &RunKey) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runtime_request WHERE run_id=$1")
        .bind(&key.run_id)
        .fetch_one(&mut **tx)
        .await?;
    require(count < 256, "request count limit")?;
    Ok(())
}

async fn insert_request(tx: &mut Tx<'_>, key: &RunKey, original: &Value) -> Result<()> {
    require(allowed(tx, key).await?, "Run unavailable")?;
    validate_session(tx, key, original).await?;
    request_capacity(tx, key).await?;
    sqlx::query("INSERT INTO runtime_request(run_id,rpc_id,original) VALUES($1,$2,$3)")
        .bind(&key.run_id)
        .bind(&original["id"])
        .bind(original)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn save_ending(
    tx: &mut Tx<'_>,
    key: &RunKey,
    original: &Value,
    kind: &str,
    payload: &Value,
    result: &Value,
) -> Result<()> {
    sqlx::query("UPDATE runtime_session SET end_kind=$2,end_request=$3,end_payload=$4 WHERE run_id=$1 AND end_kind IS NULL")
        .bind(&key.run_id).bind(kind).bind(&original["id"]).bind(payload).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE runtime_request SET result=$3 WHERE run_id=$1 AND rpc_id=$2 AND result IS NULL",
    )
    .bind(&key.run_id)
    .bind(&original["id"])
    .bind(result)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn ending_session(tx: &mut Tx<'_>, key: &RunKey, original: &Value) -> Result<()> {
    require(
        allowed(tx, key).await?,
        "ending intent no longer admissible",
    )?;
    validate_session(tx, key, original).await?;
    Ok(())
}
