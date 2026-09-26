//! Versioned operator decisions reuse the failure ledger and repair worker.
use crate::{
    budget_store::{decode, require},
    development_constraints::Constraint,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub request_id: String,
    pub version: i64,
    pub revision: i64,
    pub validation_id: String,
    pub reason: String,
    pub action: Action,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Revalidate {
        plan_digest: String,
        resume_condition: String,
    },
    RevalidateDelivery {
        plan_digest: String,
        resume_condition: String,
        policy_digest: String,
    },
    AdaptCode {
        constraints: Vec<Constraint>,
    },
}

pub async fn decide(pool: &PgPool, id: i64, command: &Decision) -> Result<Value> {
    validate(command)?;
    let mut tx = crate::run_store::lock(pool).await?;
    let input = json!({"extension_recovery":id,"command":command});
    if let Some(saved) = replay(&mut tx, &command.request_id, &input).await? {
        return Ok(saved);
    }
    let event = eligible(&mut tx, id, command).await?;
    let state = match command.action {
        Action::Revalidate { .. } | Action::RevalidateDelivery { .. } => "pending",
        Action::AdaptCode { .. } => "adaptation",
    };
    let result = persist_decision(&mut tx, id, command, &event, state).await?;
    tx.commit().await?;
    Ok(result)
}

async fn persist_decision(
    tx: &mut Tx<'_>,
    id: i64,
    command: &Decision,
    event: &str,
    state: &str,
) -> Result<Value> {
    let input = json!({"extension_recovery":id,"command":command});
    sqlx::query("UPDATE recovery_failure SET resolution=$2,resolution_state=$3,decision=CASE WHEN $3='adaptation' THEN 'code' ELSE 'blocked' END,reason='operator authorized scoped recovery; original fault retained' WHERE event_key=$1 AND (resolution IS NULL OR (resolution_state='blocked' AND successor_validation IS NULL))")
        .bind(event).bind(json!({"actor":"authenticated_operator","command":command})).bind(state).execute(&mut **tx).await?;
    sqlx::query("UPDATE recovery_failure SET decision='covered',reason='included in validation recovery decision '||$2 WHERE source_validation_id=$1 AND event_key<>$2 AND decision='blocked'")
        .bind(&command.validation_id).bind(event).execute(&mut **tx).await?;
    let version: i64 = sqlx::query_scalar(
        "UPDATE requirement SET version=version+1 WHERE id=$1 RETURNING version",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let result = json!({"accepted":true,"started":false,"version":version,"event_key":event,"resolution_state":state});
    sqlx::query("INSERT INTO business_request(request_id,input,result) VALUES($1,$2,$3)")
        .bind(&command.request_id)
        .bind(input)
        .bind(&result)
        .execute(&mut **tx)
        .await?;
    Ok(result)
}

fn validate(command: &Decision) -> Result<()> {
    require(
        crate::contract::validate_request_id(&command.request_id).is_ok(),
        "invalid request identity",
    )?;
    require(
        !command.reason.trim().is_empty() && command.reason.len() <= 4096,
        "recovery reason required",
    )?;
    match &command.action {
        Action::AdaptCode { constraints } => {
            require(
                !constraints.is_empty(),
                "explicit adaptation scope required",
            )?;
            crate::development_constraints::validate(constraints).map_err(protocol)
        }
        Action::Revalidate {
            plan_digest,
            resume_condition,
        }
        | Action::RevalidateDelivery {
            plan_digest,
            resume_condition,
            ..
        } => {
            require(digest_valid(plan_digest), "approved plan digest required")?;
            require(
                !resume_condition.trim().is_empty() && resume_condition.len() <= 4096,
                "recovery condition required",
            )
        }
    }
}
fn digest_valid(value: &str) -> bool {
    if value.len() != 64 {
        return false;
    }
    for byte in value.bytes() {
        if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}
fn protocol(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}

async fn replay(tx: &mut Tx<'_>, request: &str, input: &Value) -> Result<Option<Value>> {
    let old: Option<(Value, Value)> =
        sqlx::query_as("SELECT input,result FROM business_request WHERE request_id=$1")
            .bind(request)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some((old, result)) = old {
        require(old == *input, "recovery request identity conflict")?;
        return Ok(Some(result));
    }
    Ok(None)
}

async fn eligible(tx: &mut Tx<'_>, id: i64, command: &Decision) -> Result<String> {
    let revision = prepare_authorized_failure(tx, id, command).await?;
    let row: (String, Option<Value>) = sqlx::query_as("SELECT f.event_key,v.hook_context FROM recovery_failure f JOIN candidate_validation v ON v.id=f.source_validation_id WHERE f.requirement_id=$1 AND v.revision=$2 AND v.id=$3 AND (v.result IN ('gate_failed','blocked') OR ($4 AND v.result='succeeded' AND v.hook_invalidated)) AND v.superseded_by IS NULL AND NOT EXISTS(SELECT 1 FROM agent_run newer JOIN agent_run original ON original.id=v.source_run_id WHERE newer.requirement_id=v.requirement_id AND newer.run_sequence>original.run_sequence) AND f.decision='blocked' AND (f.resolution IS NULL OR (f.resolution_state='blocked' AND f.successor_validation IS NULL)) AND NOT EXISTS(SELECT 1 FROM recovery_failure other WHERE other.source_validation_id=v.id AND other.event_key<>f.event_key AND other.resolution IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM candidate_validation newer WHERE newer.retry_of=v.id) ORDER BY f.event_key LIMIT 1")
        .bind(id).bind(revision).bind(&command.validation_id).bind(matches!(&command.action, Action::RevalidateDelivery { .. })).fetch_one(&mut **tx).await?;
    stopped(row.1)?;
    let hooks_stopped: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM project_hook_invocation WHERE run_id=$1 AND status IN ('intent','running','unknown') AND NOT stop_confirmed)")
        .bind(&command.validation_id).fetch_one(&mut **tx).await?;
    require(
        hooks_stopped,
        "original project hook requires reconciliation",
    )?;
    check_limits(tx, id).await?;
    Ok(row.0)
}

async fn prepare_authorized_failure(tx: &mut Tx<'_>, id: i64, command: &Decision) -> Result<i64> {
    let revision = authorized_revision(tx, id, command).await?;
    crate::extension_delivery_recovery::authorize(tx, id, revision, &command.action).await?;
    crate::extension_delivery_recovery::prepare_pending(
        tx,
        id,
        &command.validation_id,
        &command.action,
    )
    .await?;
    crate::extension_failure::reconcile_legacy(tx, id, &command.validation_id).await?;
    Ok(revision)
}

async fn authorized_revision(tx: &mut Tx<'_>, id: i64, command: &Decision) -> Result<i64> {
    let (version, revision, incarnation): (i64,i64,String) = sqlx::query_as("SELECT r.version,r.revision,c.incarnation FROM requirement r JOIN execution_control c ON c.requirement_id=r.id WHERE r.id=$1")
        .bind(id).fetch_one(&mut **tx).await?;
    require(
        (version, revision) == (command.version, command.revision),
        "stale recovery version",
    )?;
    require(
        crate::recovery_store::allowed(tx, id, revision, &incarnation).await?,
        "recovery authority unavailable",
    )?;
    Ok(revision)
}
async fn check_limits(tx: &mut Tx<'_>, id: i64) -> Result<()> {
    let balance = crate::budget_store::balance(tx, id).await?;
    require(!balance.exhausted, "cumulative budget exhausted")?;
    let used: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM business_request WHERE input->>'extension_recovery'=($1::bigint)::text",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    require(used < 3, "explicit recovery attempt budget exhausted")?;
    Ok(())
}

pub(crate) fn stopped(context: Option<Value>) -> Result<()> {
    if let Some(value) = context {
        let context: crate::validation_context::Context = decode(value)?;
        let id = context.call.identity.invocation_id;
        let key = crate::execution::RunKey {
            run_id: id.clone(),
            request_id: id.clone(),
            incarnation: id,
        };
        let stopped =
            crate::validation_supervisor::quiescent(&context.directory, &key).map_err(external)?;
        require(stopped, "original validation process not confirmed stopped")?;
    }
    Ok(())
}
pub(crate) fn external(error: Box<dyn std::error::Error + Send + Sync>) -> sqlx::Error {
    sqlx::Error::Protocol(error.to_string())
}

pub async fn view(pool: &PgPool, id: i64) -> Result<Value> {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('event_key',f.event_key,'validation_id',f.source_validation_id,'facts',f.facts,'decision',f.decision,'reason',f.reason,'resolution',f.resolution,'resolution_state',f.resolution_state,'successor_validation',f.successor_validation) FROM recovery_failure f WHERE requirement_id=$1 ORDER BY created_at,event_key").bind(id).fetch_all(pool).await?;
    let mut result = json!({"failures":rows});
    crate::operator_view::redact(&mut result);
    Ok(result)
}

pub async fn constraints(pool: &PgPool, run: &str) -> Result<Vec<Value>> {
    sqlx::query_scalar("SELECT f.resolution#>'{command,action,constraints}' FROM recovery_failure f JOIN candidate_validation v ON v.id=f.source_validation_id JOIN agent_run a ON a.requirement_id=f.requirement_id AND a.revision=v.revision WHERE a.id=$1 AND f.resolution_state='adaptation' ORDER BY f.created_at,f.event_key")
        .bind(run).fetch_all(pool).await
}

pub async fn check_scope(
    pool: &PgPool,
    broker: &crate::git_broker::GitBroker,
    source: &str,
    manifest: &crate::workspace::Manifest,
) -> std::result::Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let row: Option<(String, Value)> = sqlx::query_as("SELECT v.candidate_sha,f.resolution#>'{command,action,constraints}' FROM repair_reservation p JOIN recovery_failure f ON f.event_key=p.event_key JOIN candidate_validation v ON v.id=p.source_validation_id WHERE p.repair_run_id=$1 AND f.resolution_state='adaptation'").bind(source).fetch_optional(pool).await?;
    let Some((baseline, value)) = row else {
        return Ok(true);
    };
    let constraints: Vec<Constraint> = serde_json::from_value(value)?;
    let paths = broker.changed_paths(&manifest.workspace, &baseline, &manifest.head)?;
    for path in paths {
        if !path_allowed(&path, &constraints) {
            sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT 'scope:'||$1,p.requirement_id,p.source_validation_id,'local',f.facts,f.fingerprint,'blocked','adaptation changed a path outside the approved scope' FROM repair_reservation p JOIN recovery_failure f ON f.event_key=p.event_key WHERE p.repair_run_id=$1 ON CONFLICT DO NOTHING").bind(source).execute(pool).await?;
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn path_allowed(path: &str, constraints: &[Constraint]) -> bool {
    for constraint in constraints {
        for scope in &constraint.paths {
            if path == scope || path.starts_with(&format!("{scope}/")) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "../tests/unit/extension_recovery.rs"]
mod tests;
