//! Durable failure identity and admission under the existing global owner lock.
use crate::{
    bounded_recovery::{Class, Failure},
    budget::Amount,
    budget_store, run_store,
    runtime_resume::Job,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;

pub async fn record(
    pool: &PgPool,
    requirement: i64,
    source: &str,
    event: &str,
    failure: &Failure,
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let old: Option<Value> =
        sqlx::query_scalar("SELECT facts FROM recovery_failure WHERE event_key=$1")
            .bind(event)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(old) = old {
        return budget_store::require(old == json!(failure), "failure event identity conflict");
    }
    let bound: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidate_validation v JOIN agent_run a ON a.id=v.source_run_id WHERE v.id=$1 AND v.requirement_id=$2 AND v.candidate_sha=$3 AND v.result=CASE WHEN $4='local' THEN 'gate_failed' ELSE 'succeeded' END AND a.state='Succeeded' AND a.quiescent)")
        .bind(source).bind(requirement).bind(&failure.candidate_sha).bind(&failure.phase).fetch_one(&mut *tx).await?;
    let repeated: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recovery_failure WHERE requirement_id=$1 AND fingerprint=$2 AND decision IN ('reserved','code')) OR EXISTS(SELECT 1 FROM repair_reservation WHERE requirement_id=$1 AND source_validation_id=$3)")
        .bind(requirement).bind(failure.fingerprint()).bind(source).fetch_one(&mut *tx).await?;
    let decision = classify(failure, bound, repeated);
    persist_failure(&mut tx, requirement, source, event, failure, decision).await?;
    tx.commit().await
}
fn classify(failure: &Failure, bound: bool, repeated: bool) -> (&'static str, &'static str) {
    if !bound {
        return (
            "blocked",
            "candidate evidence or execution identity is not valid for this failure phase",
        );
    }
    match failure.classify() {
        Class::Code if !repeated => ("code", "classified authorized check failure"),
        Class::Code => (
            "covered",
            "duplicate failure already accounted; preserve original repair intent",
        ),
        Class::Infrastructure => ("infrastructure", "retry stage without a coding model"),
        _ => (
            "blocked",
            "unknown, configuration, authorization or security failure; operator action required",
        ),
    }
}
async fn persist_failure(
    tx: &mut Tx<'_>,
    requirement: i64,
    source: &str,
    event: &str,
    failure: &Failure,
    (decision, reason): (&str, &str),
) -> Result<()> {
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT $1,$2,$3,$4,$5,$6,$7,$8 WHERE EXISTS(SELECT 1 FROM candidate_validation WHERE id=$3 AND requirement_id=$2 AND candidate_sha=$9)")
        .bind(event).bind(requirement).bind(source).bind(&failure.phase).bind(json!(failure)).bind(failure.fingerprint()).bind(decision).bind(reason).bind(&failure.candidate_sha).execute(&mut **tx).await?;
    continue_predecessor(tx, event).await?;
    if matches!(decision, "code" | "infrastructure") {
        sqlx::query("UPDATE recovery_failure SET decision='superseded',reason='new native evidence retained as '||$1 WHERE source_validation_id=$2 AND event_key<>$1 AND decision='blocked' AND facts->>'native_code'='unknown' AND facts->>'log_ref'=$3")
            .bind(event).bind(source).bind(&failure.log_ref).execute(&mut **tx).await?;
    }
    if decision == "infrastructure" {
        crate::recovery_retry::schedule(tx, event, requirement, failure).await?;
    }
    if decision == "blocked" {
        sqlx::query("UPDATE agent_run SET stop_requested=true,blocker='recovery requires operator review; preserve work' WHERE requirement_id=$1 AND NOT quiescent")
            .bind(requirement).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn continue_predecessor(tx: &mut Tx<'_>, event: &str) -> Result<()> {
    sqlx::query("UPDATE recovery_failure old SET decision='continued',reason='successor validation failure retained as '||$1 FROM recovery_failure incoming JOIN candidate_validation v ON v.id=incoming.source_validation_id JOIN repair_reservation p ON p.repair_run_id=v.source_run_id WHERE incoming.event_key=$1 AND old.event_key=p.event_key AND old.event_key<>incoming.event_key AND p.status='failed' AND old.decision='reserved'")
        .bind(event).execute(&mut **tx).await?;
    Ok(())
}

pub async fn unchanged_candidate(pool: &PgPool, run: &str, sha: &str) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let source:Option<(String,String)> = sqlx::query_as("SELECT p.source_validation_id,p.event_key FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id WHERE p.repair_run_id=$1 AND v.candidate_sha=$2 AND p.event_key IS NOT NULL")
        .bind(run).bind(sha).fetch_optional(&mut *tx).await?;
    let Some((source, event)) = source else {
        return Ok(false);
    };
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT $1,requirement_id,$2,phase,facts,fingerprint,'blocked','repair produced unchanged candidate; preserve work and review the original failure' FROM recovery_failure WHERE event_key=$3 ON CONFLICT DO NOTHING")
        .bind(format!("no-progress:{run}")).bind(source).bind(event).execute(&mut *tx).await?;
    sqlx::query("UPDATE repair_reservation SET status='failed' WHERE repair_run_id=$1")
        .bind(run)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

pub(crate) async fn allowed(
    tx: &mut Tx<'_>,
    id: i64,
    revision: i64,
    incarnation: &str,
) -> Result<bool> {
    if !crate::group_queue_store::authorized(tx, id).await? {
        return Ok(false);
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_control c ON c.requirement_id=r.id JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=COALESCE((v.document->>'repository_id')::bigint,1) WHERE r.id=$1 AND r.revision=$2 AND r.state IN ('Running','Submitted') AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND c.incarnation=$3 AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE requirement_id=r.id AND NOT quiescent) AND NOT EXISTS(SELECT 1 FROM runtime_blocker b JOIN agent_run a ON a.id=b.run_id WHERE a.requirement_id=r.id AND NOT b.resolved))")
        .bind(id).bind(revision).bind(incarnation).fetch_one(&mut **tx).await
}

/// Ordinal, initial model resources, workspace allocation and concrete launch
/// intent commit together. An unknown launch retains this exact reservation.
pub async fn reserve(pool: &PgPool, event: &str, job: &Job, resources: Amount) -> Result<bool> {
    validate_job(job, resources)?;
    let mut tx = run_store::lock(pool).await?;
    let Some(reservation) = prepare_reservation(&mut tx, event, job, resources).await? else {
        tx.commit().await?;
        return Ok(false);
    };
    commit_reservation(tx, event, job, resources, reservation).await
}
fn validate_job(job: &Job, resources: Amount) -> Result<()> {
    budget_store::require(
        resources.positive() && resources.turns == 1,
        "one positive repair turn reservation required",
    )?;
    budget_store::require(
        job.launch.key == job.workspace.key
            && job.launch.workspace == job.workspace.path
            && job.launch.workspace_identity == job.workspace.identity,
        "repair launch/workspace identity mismatch",
    )?;
    Ok(())
}
type Reservation = (String, Value, i64, i64);
async fn prepare_reservation(
    tx: &mut Tx<'_>,
    event: &str,
    job: &Job,
    resources: Amount,
) -> Result<Option<Reservation>> {
    let id = job.workspace.requirement;
    if !allowed(tx, id, job.workspace.revision, &job.launch.key.incarnation).await? {
        return Ok(None);
    }
    let row: Option<(String, Value)> = sqlx::query_as("SELECT f.source_validation_id,f.facts FROM recovery_failure f JOIN candidate_validation v ON v.id=f.source_validation_id WHERE f.event_key=$1 AND f.requirement_id=$2 AND f.decision='code' AND NOT EXISTS(SELECT 1 FROM recovery_failure b WHERE b.source_validation_id=f.source_validation_id AND b.decision IN ('blocked','infrastructure')) AND v.source_run_id=$3 AND v.candidate_sha=$4 AND v.revision=$5 AND NOT EXISTS(SELECT 1 FROM repair_reservation p WHERE p.requirement_id=$2 AND p.status IN ('reserved','started'))")
        .bind(event).bind(id).bind(&job.source).bind(&job.workspace.baseline).bind(job.workspace.revision).fetch_optional(&mut **tx).await?;
    let Some((source, failure)) = row else {
        return Ok(None);
    };
    if !current_head(tx, id, &failure).await? {
        return Ok(None);
    }
    let ordinal: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(ordinal),0)+1 FROM repair_reservation WHERE requirement_id=$1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let Some(limit) = capacity(tx, event, id, ordinal, resources).await? else {
        return Ok(None);
    };
    Ok(Some((source, failure, ordinal, limit)))
}
async fn current_head(tx: &mut Tx<'_>, id: i64, failure: &Value) -> Result<bool> {
    if failure["phase"] == "local" {
        return Ok(true);
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM github_pr_observation WHERE requirement_id=$1 AND NOT stale AND observation->>'head'=$2 AND observation->>'merge'='Unmerged' AND NOT (observation->>'closed')::boolean)")
        .bind(id).bind(failure["pr_head"].as_str()).fetch_one(&mut **tx).await
}
pub(crate) async fn launch_allowed(
    tx: &mut Tx<'_>,
    id: i64,
    launch: &crate::execution::Launch,
) -> Result<bool> {
    let failure:Option<(Value,bool)>=sqlx::query_as("SELECT f.facts,NOT EXISTS(SELECT 1 FROM recovery_failure b WHERE b.source_validation_id=f.source_validation_id AND b.decision IN ('blocked','infrastructure')) FROM repair_reservation p JOIN recovery_failure f ON f.event_key=p.event_key WHERE p.launch=$1").bind(json!(launch)).fetch_optional(&mut **tx).await?;
    let Some((failure, safe)) = failure else {
        return Ok(true);
    };
    Ok(safe
        && current_head(tx, id, &failure).await?
        && crate::group_budget::prepaid_fits(tx, id).await?)
}
async fn capacity(
    tx: &mut Tx<'_>,
    event: &str,
    id: i64,
    ordinal: i64,
    resources: Amount,
) -> Result<Option<i64>> {
    let limit: Option<i64> =
        sqlx::query_scalar("SELECT repair_limit FROM repair_authorization WHERE requirement_id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    let within_limit = match limit {
        Some(limit) => ordinal <= limit,
        None => false,
    };
    if !within_limit || !budget_store::reservation_allowed(tx, id, resources).await? {
        sqlx::query("UPDATE recovery_failure SET decision='blocked',reason='budget_exhausted: preserve current owner and work' WHERE event_key=$1")
            .bind(event).execute(&mut **tx).await?;
        return Ok(None);
    }
    Ok(limit)
}
async fn commit_reservation(
    mut tx: Tx<'_>,
    event: &str,
    job: &Job,
    resources: Amount,
    reservation: Reservation,
) -> Result<bool> {
    let id = job.workspace.requirement;
    if !crate::storage_service::reserve_workspace(&mut tx, &job.workspace)
        .await
        .map_err(|error| {
            // Retain the native closure body as a separately measured source owner.
            sqlx::Error::Protocol(error.to_string())
        })?
    {
        return Ok(false);
    }
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=$1 AND state='Submitted'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let context =
        repair_context(&mut tx, id, job.workspace.revision, &reservation, resources).await?;
    let (source, _, ordinal, _) = reservation;
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status,launch,workspace,resources,event_key) VALUES($1,$2,$3,$4,'reserved',$5,$6,$7,$8)")
        .bind(id).bind(ordinal).bind(source).bind(context).bind(json!(job.launch)).bind(json!(job.workspace)).bind(json!(resources)).bind(event).execute(&mut *tx).await?;
    sqlx::query("UPDATE recovery_failure SET decision=CASE WHEN event_key=$1 THEN 'reserved' ELSE 'covered' END WHERE source_validation_id=(SELECT source_validation_id FROM recovery_failure WHERE event_key=$1) AND decision='code'")
        .bind(event)
        .execute(&mut *tx)
        .await?;
    crate::group_budget::sync(&mut tx, id).await?;
    tx.commit().await?;
    Ok(true)
}

async fn repair_context(
    tx: &mut Tx<'_>,
    id: i64,
    revision: i64,
    reservation: &Reservation,
    resources: Amount,
) -> Result<Value> {
    let resolution: Option<Value> = sqlx::query_scalar("SELECT resolution FROM recovery_failure WHERE source_validation_id=$1 AND resolution IS NOT NULL ORDER BY created_at LIMIT 1").bind(&reservation.0).fetch_optional(&mut **tx).await?.flatten();
    let balance = budget_store::balance(tx, id).await?;
    let (source, failure, ordinal, limit) = reservation;
    let contract: Value = sqlx::query_scalar(
        "SELECT document FROM execution_revision WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(id)
    .bind(revision)
    .fetch_one(&mut **tx)
    .await?;
    let group:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('item_id',b.item_id,'limits',b.limits,'used',b.used,'reserved',b.reserved) FROM group_budget b JOIN group_execution_item i ON i.draft_id=b.draft_id AND (b.item_id='' OR b.item_id=i.child_id) WHERE i.requirement_id=$1 ORDER BY b.item_id")
        .bind(id).fetch_all(&mut **tx).await?;
    Ok(
        json!({"resolution":resolution,"failure":failure,"ordinal":ordinal,"repair_limit":limit,"budget":balance,"group_budget":group,"budget_snapshot":"before_this_reservation","reserved_resources":resources,"source_validation":source,"remaining_acceptance":contract["contract"]["acceptance_criteria"]}),
    )
}

pub(crate) async fn resources(tx: &mut Tx<'_>, id: i64) -> Result<Amount> {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT resources FROM repair_reservation WHERE requirement_id=$1 AND resources IS NOT NULL AND NOT resources_transferred")
        .bind(id).fetch_all(&mut **tx).await?;
    let mut total = Amount::default();
    for row in rows {
        total = total
            .checked_add(budget_store::decode(row)?)
            .ok_or_else(|| {
                // Do not wrap overflowing persisted reservations into a balance.
                sqlx::Error::Protocol("repair reservation overflow".into())
            })?;
    }
    Ok(total)
}

pub(crate) async fn transfer(tx: &mut Tx<'_>, intent: &budget_store::CallIntent) -> Result<bool> {
    let reserved: Option<Value> = sqlx::query_scalar("SELECT resources FROM repair_reservation WHERE repair_run_id=$1 AND NOT resources_transferred AND resources IS NOT NULL AND status='started'")
        .bind(&intent.key.run_id).fetch_optional(&mut **tx).await?;
    let Some(reserved) = reserved else {
        return Ok(false);
    };
    budget_store::require(
        reserved == json!(intent.reserve),
        "repair resources changed; reconcile reservation",
    )?;
    sqlx::query("UPDATE repair_reservation SET resources_transferred=true WHERE repair_run_id=$1")
        .bind(&intent.key.run_id)
        .execute(&mut **tx)
        .await?;
    Ok(true)
}

/// The Runtime has already admitted the successor under the shared owner lock.
/// Preserve the original association before redirecting completion/first-call
/// settlement to this successor. No new ordinal or resource hold is created.
pub(crate) async fn link_resume(tx: &mut Tx<'_>, run: &str) -> Result<()> {
    sqlx::query("INSERT INTO linked_run_input(run_id,failure_id,document) SELECT $1,i.failure_id,i.document FROM run_workspace w JOIN linked_run_input i ON i.run_id=w.restored_from WHERE w.run_id=$1 ON CONFLICT DO NOTHING")
        .bind(run).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO repair_run_history(requirement_id,ordinal,source_run,successor_run) SELECT p.requirement_id,p.ordinal,p.repair_run_id,$1 FROM repair_reservation p JOIN run_workspace w ON w.restored_from=p.repair_run_id WHERE w.run_id=$1 AND p.status='started' ON CONFLICT DO NOTHING")
        .bind(run).execute(&mut **tx).await?;
    sqlx::query("UPDATE repair_reservation p SET repair_run_id=$1 FROM run_workspace w WHERE w.run_id=$1 AND w.restored_from=p.repair_run_id AND p.status='started'")
        .bind(run).execute(&mut **tx).await?;
    Ok(())
}
