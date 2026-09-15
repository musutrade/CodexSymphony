//! PostgreSQL execution facts and one global Requirement owner.
use crate::execution::{Launch, ProcessIdentity, Receipt, RunKey, accepts_event, receipt_matches};
use sqlx::{PgPool, Postgres, Transaction};

type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Run {
    pub id: String,
    pub requirement_id: i64,
    pub incarnation: String,
    pub request_id: String,
    pub workspace: String,
    pub workspace_identity: String,
    pub phase: String,
    pub process_identity: Option<serde_json::Value>,
    pub stop_requested: bool,
    pub quiescent: bool,
    pub blocker: Option<String>,
}

impl Run {
    pub fn key(&self) -> RunKey {
        RunKey {
            run_id: self.id.clone(),
            request_id: self.request_id.clone(),
            incarnation: self.incarnation.clone(),
        }
    }
    pub fn process(&self) -> Option<ProcessIdentity> {
        serde_json::from_value(self.process_identity.clone()?).ok()
    }
}

pub(crate) async fn lock(pool: &PgPool) -> Result<Tx<'_>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '500ms'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '2s'")
        .execute(&mut *tx)
        .await?;
    // Also serializes with review/revocation/withdrawal in the existing adapter.
    sqlx::query("SELECT pg_advisory_xact_lock(13002)")
        .execute(&mut *tx)
        .await?;
    sqlx::query_scalar::<_, i32>("SELECT id FROM execution_control WHERE id=1 FOR UPDATE")
        .fetch_one(&mut *tx)
        .await?;
    Ok(tx)
}

pub async fn begin_incarnation(pool: &PgPool, incarnation: &str) -> Result<()> {
    sqlx::query("INSERT INTO execution_control(id) SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM agent_run) ON CONFLICT DO NOTHING")
        .execute(pool)
        .await?;
    let mut tx = lock(pool).await?;
    sqlx::query("UPDATE execution_control SET incarnation=$1,recovery_complete=false WHERE id=1")
        .bind(incarnation)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Internal reservation boundary. The product tick deliberately does not call
/// this until downstream preparation capabilities exist. No coding HTTP API.
pub async fn reserve_prepared(pool: &PgPool, launch: &Launch) -> Result<bool> {
    let mut tx = lock(pool).await?;
    if !claim_allowed(&mut tx, &launch.key.incarnation).await? {
        return Ok(false);
    }
    let Some((id, revision)) = queued(&mut tx).await? else {
        return Ok(false);
    };
    commit_claim(tx, id, revision, launch).await
}

async fn queued(tx: &mut Tx<'_>) -> Result<Option<(i64, i64)>> {
    let next: Option<(i64,i64,bool)> = sqlx::query_as(
        "SELECT id,revision,paused FROM requirement WHERE state='Ready' ORDER BY id LIMIT 1 FOR UPDATE")
        .fetch_optional(&mut **tx).await?;
    let Some((id, revision, false)) = next else {
        return Ok(None);
    };
    if !authorized(tx, id, revision).await? {
        return Ok(None);
    }
    Ok(Some((id, revision)))
}

async fn commit_claim(mut tx: Tx<'_>, id: i64, revision: i64, launch: &Launch) -> Result<bool> {
    insert_run(&mut tx, id, revision, launch).await?;
    sqlx::query("UPDATE execution_control SET requirement_id=$1 WHERE id=1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE requirement SET state='Running',version=version+1 WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

async fn claim_allowed(tx: &mut Tx<'_>, incarnation: &str) -> Result<bool> {
    sqlx::query_scalar("SELECT requirement_id IS NULL AND NOT paused AND recovery_complete AND incarnation=$1 FROM execution_control WHERE id=1")
        .bind(incarnation).fetch_one(&mut **tx).await
}

async fn authorized(tx: &mut Tx<'_>, id: i64, revision: i64) -> Result<bool> {
    let allowed: Option<bool> = sqlx::query_scalar("SELECT NOT (r.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint > r.revoked_through_version FROM requirement_revision v CROSS JOIN repository r WHERE v.requirement_id=$1 AND v.revision=$2 AND r.id=1")
        .bind(id).bind(revision).fetch_optional(&mut **tx).await?;
    Ok(allowed == Some(true))
}

async fn insert_run(tx: &mut Tx<'_>, id: i64, revision: i64, launch: &Launch) -> Result<()> {
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'Created')")
        .bind(&launch.key.run_id).bind(id).bind(revision).bind(&launch.key.incarnation)
        .bind(&launch.key.request_id).bind(&launch.workspace).bind(&launch.workspace_identity)
        .bind(sqlx::types::Json(launch))
        .execute(&mut **tx).await?;
    Ok(())
}

pub async fn unresolved(pool: &PgPool) -> Result<Vec<Run>> {
    sqlx::query_as("SELECT * FROM agent_run WHERE NOT quiescent ORDER BY created_at,id")
        .fetch_all(pool)
        .await
}

pub async fn attach_process(pool: &PgPool, receipt: &Receipt) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_run SET process_identity=$4,state='Running' WHERE id=$1 AND request_id=$2 AND incarnation=$3 AND process_identity IS NULL AND NOT quiescent")
        .bind(&receipt.key.run_id).bind(&receipt.key.request_id).bind(&receipt.key.incarnation)
        .bind(sqlx::types::Json(&receipt.process))
        .execute(pool).await?;
    Ok(result.rows_affected() == 1)
}

pub async fn actions_allowed(pool: &PgPool, key: &RunKey) -> Result<bool> {
    let result: Option<bool> = sqlx::query_scalar("SELECT c.incarnation=a.incarnation AND c.recovery_complete AND NOT c.paused AND NOT r.paused AND NOT a.stop_requested AND NOT a.quiescent AND a.state IN ('Created','Running') AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint > p.revoked_through_version FROM agent_run a JOIN requirement r ON r.id=a.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=a.revision CROSS JOIN repository p WHERE a.id=$1 AND a.request_id=$2 AND a.incarnation=$3 AND p.id=1")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).fetch_optional(pool).await?;
    Ok(result == Some(true))
}

pub async fn reserved_launch(pool: &PgPool, launch: &Launch) -> Result<bool> {
    let matches: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM agent_run WHERE id=$1 AND launch=$2 AND process_identity IS NULL AND state='Created')")
        .bind(&launch.key.run_id).bind(sqlx::types::Json(launch)).fetch_one(pool).await?;
    Ok(matches && actions_allowed(pool, &launch.key).await?)
}

/// Persist pause before the coordinator attempts any OS stop. Repeated pause
/// never clears an earlier intent, owner, process identity or saved phase.
pub async fn pause(pool: &PgPool, requirement: Option<i64>) -> Result<()> {
    let mut tx = lock(pool).await?;
    match requirement {
        Some(id) => {
            sqlx::query("UPDATE requirement SET paused=true WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        None => {
            sqlx::query("UPDATE execution_control SET paused=true WHERE id=1")
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("UPDATE agent_run SET stop_requested=true WHERE NOT quiescent AND ($1::bigint IS NULL OR requirement_id=$1)")
        .bind(requirement).execute(&mut *tx).await?;
    tx.commit().await
}

pub async fn block(pool: &PgPool, id: &str, reason: &str) -> Result<()> {
    sqlx::query(
        "UPDATE agent_run SET blocker=$2,stop_requested=true WHERE id=$1 AND NOT quiescent",
    )
    .bind(id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn confirm_quiescent(pool: &PgPool, run: &Run, receipt: &Receipt) -> Result<bool> {
    let Some(process) = run.process() else {
        return Ok(false);
    };
    if !receipt_matches(&run.key(), &process, receipt) {
        return Ok(false);
    }
    // No candidate preservation exists yet, so stop proof means Interrupted,
    // never Succeeded, Requirement completion, or permission to free the slot.
    sqlx::query("UPDATE agent_run SET quiescent=true,state='Interrupted',blocker=NULL WHERE id=$1 AND process_identity=$2")
        .bind(&run.id).bind(&run.process_identity).execute(pool).await?;
    Ok(true)
}

pub async fn finish_recovery(pool: &PgPool, incarnation: &str) -> Result<bool> {
    let result = sqlx::query("UPDATE execution_control SET recovery_complete=true WHERE id=1 AND incarnation=$1 AND NOT EXISTS (SELECT 1 FROM agent_run WHERE NOT quiescent)")
        .bind(incarnation).execute(pool).await?;
    Ok(result.rows_affected() == 1)
}

pub async fn archive_event(
    pool: &PgPool,
    incoming: &RunKey,
    payload: serde_json::Value,
) -> Result<bool> {
    let mut tx = lock(pool).await?;
    let key: Option<(String,String,String)> = sqlx::query_as("SELECT a.id,a.request_id,a.incarnation FROM agent_run a JOIN execution_control c ON c.requirement_id=a.requirement_id AND c.incarnation=a.incarnation JOIN requirement r ON r.id=a.requirement_id WHERE a.id=$1 AND a.state='Running' AND NOT a.stop_requested AND NOT a.quiescent AND NOT c.paused AND NOT r.paused AND c.recovery_complete")
        .bind(&incoming.run_id).fetch_optional(&mut *tx).await?;
    let accepted = key.is_some_and(|(run_id, request_id, incarnation)| {
        accepts_event(
            &RunKey {
                run_id,
                request_id,
                incarnation,
            },
            incoming,
            true,
        )
    });
    sqlx::query("INSERT INTO run_event(run_id,request_id,incarnation,payload,accepted) VALUES ($1,$2,$3,$4,$5)")
        .bind(&incoming.run_id).bind(&incoming.request_id).bind(&incoming.incarnation).bind(payload).bind(accepted)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(accepted)
}
