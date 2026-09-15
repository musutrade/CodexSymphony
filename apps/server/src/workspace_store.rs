//! Authorized local operations under the same lock as pause and revocation.
//! Pending/partial work is never automatically replayed or cleaned up.
use crate::{
    execution::RunKey,
    git_broker::GitBroker,
    run_store,
    workspace::{Manifest, Recovery, Workspace, recovery},
    workspace_files::{Result, require},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};

type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Prepare { baseline: String },
    Commit { message: String },
    Preserve,
    Restore { source: String },
}

/// Internal platform entrypoint. Only Commit.message may originate at a dynamic
/// tool. Runtime wiring and remote delivery remain disabled by their own gates.
pub async fn execute(
    pool: &PgPool,
    broker: &GitBroker,
    key: &RunKey,
    request: &str,
    operation: Operation,
) -> Result<Value> {
    if let Some(saved) = intent(pool, key, request, &operation).await? {
        if operation == Operation::Preserve {
            broker.verify(&serde_json::from_value(saved.clone())?)?;
        }
        return Ok(saved);
    }
    let result = perform(pool, broker, key, request, &operation).await;
    if result.is_err() {
        // Do not expose command stderr or credential-bearing paths in errors.
        // DB outage leaves the already committed pending intent intact.
        let _ = sqlx::query("UPDATE workspace_operation SET status='partial',error='local operation failed; preserve original directory and reconcile' WHERE run_id=$1 AND request_id=$2 AND status='pending'")
            .bind(&key.run_id).bind(request).execute(pool).await;
        let _ = sqlx::query("UPDATE agent_run SET blocker='workspace operation incomplete; retain originals',stop_requested=true WHERE id=$1")
            .bind(&key.run_id).execute(pool).await;
    }
    result
}

async fn intent(
    pool: &PgPool,
    key: &RunKey,
    request: &str,
    operation: &Operation,
) -> Result<Option<Value>> {
    require(
        !request.is_empty() && request.len() <= 256,
        "invalid tool request",
    )?;
    let mut tx = run_store::lock(pool).await?;
    authorize(&mut tx, key, operation).await?;
    if let Some(value) = saved_operation(&mut tx, key, request, operation).await? {
        return Ok(Some(value));
    }
    insert_intent(&mut tx, key, request, operation).await?;
    tx.commit().await?;
    Ok(None)
}

async fn insert_intent(
    tx: &mut Tx<'_>,
    key: &RunKey,
    request: &str,
    operation: &Operation,
) -> Result<()> {
    sqlx::query("INSERT INTO workspace_operation(run_id,request_id,command,status) VALUES ($1,$2,$3,'pending')")
        .bind(&key.run_id).bind(request).bind(serde_json::to_value(operation)?).execute(&mut **tx).await?;
    Ok(())
}

async fn saved_operation(
    tx: &mut Tx<'_>,
    key: &RunKey,
    request: &str,
    operation: &Operation,
) -> Result<Option<Value>> {
    let existing: Option<(Value, String, Option<Value>)> = sqlx::query_as(
        "SELECT command,status,result FROM workspace_operation WHERE run_id=$1 AND request_id=$2",
    )
    .bind(&key.run_id)
    .bind(request)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((command, status, value)) = existing {
        require(
            command == serde_json::to_value(operation)?,
            "request reused with different arguments",
        )?;
        require(
            status == "complete",
            "pending/partial operation requires reconciliation",
        )?;
        return Ok(value);
    }
    Ok(None)
}

async fn authorize(tx: &mut Tx<'_>, key: &RunKey, operation: &Operation) -> Result<()> {
    // Preservation is allowed after pause/revocation so paid work survives.
    // It still requires the exact Run identity, current owner and stop proof.
    let facts: (bool, bool, String) = sqlx::query_as("SELECT a.quiescent, c.incarnation=a.incarnation AND c.recovery_complete AND NOT c.paused AND NOT r.paused AND NOT a.stop_requested AND NOT a.quiescent AND a.blocker IS NULL AND r.revision=a.revision AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version, a.state FROM agent_run a JOIN execution_control c ON c.requirement_id=a.requirement_id JOIN requirement r ON r.id=a.requirement_id JOIN requirement_revision v ON v.requirement_id=a.requirement_id AND v.revision=a.revision CROSS JOIN repository p WHERE a.id=$1 AND a.request_id=$2 AND a.incarnation=$3 AND p.id=1 AND NOT EXISTS (SELECT 1 FROM agent_run newer WHERE newer.requirement_id=a.requirement_id AND newer.run_sequence>a.run_sequence)")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).fetch_one(&mut **tx).await?;
    let allowed = match operation {
        Operation::Preserve => facts.0,
        Operation::Commit { .. } => facts.1 && facts.2 == "Running",
        _ => facts.1 && facts.2 == "Created",
    };
    require(allowed, "Run is not authorized for workspace operation")?;
    let unresolved_other: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspace_operation WHERE run_id<>$1 AND status<>'complete')",
    )
    .bind(&key.run_id)
    .fetch_one(&mut **tx)
    .await?;
    require(
        !unresolved_other,
        "another workspace operation requires reconciliation",
    )
}

/// Cold-start/stop reconciliation uses only registered workspaces. Existing
/// partial attempts remain blocked without repeating filesystem operations.
pub async fn recover_stopped(pool: &PgPool, root: &std::path::Path) -> Result<bool> {
    let runs: Vec<(String,String,String)> = sqlx::query_as("SELECT a.id,a.request_id,a.incarnation FROM agent_run a JOIN run_workspace w ON w.run_id=a.id LEFT JOIN workspace_snapshot s ON s.run_id=a.id WHERE a.quiescent AND s.run_id IS NULL")
        .fetch_all(pool).await?;
    for (run_id, request_id, incarnation) in runs {
        let incomplete: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM workspace_operation WHERE run_id=$1 AND status<>'complete')")
            .bind(&run_id).fetch_one(pool).await?;
        if incomplete {
            return Ok(false);
        }
        let broker = GitBroker::open(root)?;
        let key = RunKey {
            run_id,
            request_id,
            incarnation,
        };
        execute(
            pool,
            &broker,
            &key,
            "platform-preserve",
            Operation::Preserve,
        )
        .await?;
    }
    Ok(true)
}

async fn perform(
    pool: &PgPool,
    broker: &GitBroker,
    key: &RunKey,
    request: &str,
    operation: &Operation,
) -> Result<Value> {
    let mut tx = run_store::lock(pool).await?;
    authorize(&mut tx, key, operation).await?;
    let pending: bool = sqlx::query_scalar("SELECT status='pending' FROM workspace_operation WHERE run_id=$1 AND request_id=$2 FOR UPDATE")
        .bind(&key.run_id).bind(request).fetch_one(&mut *tx).await?;
    require(pending, "operation is not pending")?;
    let result = apply_operation(&mut tx, broker, key, operation).await?;
    sqlx::query("UPDATE workspace_operation SET status='complete',result=$3 WHERE run_id=$1 AND request_id=$2 AND status='pending'")
        .bind(&key.run_id).bind(request).bind(&result).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result)
}

async fn apply_operation(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    key: &RunKey,
    operation: &Operation,
) -> Result<Value> {
    match operation {
        Operation::Prepare { baseline } => prepare(tx, broker, key, baseline).await,
        Operation::Commit { message } => commit(tx, broker, key, message).await,
        Operation::Preserve => preserve(tx, broker, key).await,
        Operation::Restore { source } => restore(tx, broker, key, source).await,
    }
}

async fn identity(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    key: &RunKey,
    baseline: &str,
) -> Result<Workspace> {
    let (requirement, revision, phase, path, identity): (i64,i64,String,String,String) = sqlx::query_as("SELECT requirement_id,revision,phase,workspace,workspace_identity FROM agent_run WHERE id=$1")
        .bind(&key.run_id).fetch_one(&mut **tx).await?;
    require(
        broker.path(&key.run_id)?.to_str() == Some(&path),
        "reserved cwd does not match Run worktree",
    )?;
    Ok(Workspace {
        key: key.clone(),
        identity,
        requirement,
        revision,
        phase,
        baseline: baseline.into(),
        branch: format!("ai/req-{}-{}", requirement, key.run_id),
        path,
    })
}

async fn bind(tx: &mut Tx<'_>, workspace: &Workspace, source: Option<&str>) -> Result<()> {
    sqlx::query("INSERT INTO run_workspace(run_id,identity,restored_from) VALUES ($1,$2,$3)")
        .bind(&workspace.key.run_id)
        .bind(serde_json::to_value(workspace)?)
        .bind(source)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn bound(tx: &mut Tx<'_>, broker: &GitBroker, key: &RunKey) -> Result<Workspace> {
    let value: Value = sqlx::query_scalar("SELECT identity FROM run_workspace WHERE run_id=$1")
        .bind(&key.run_id)
        .fetch_one(&mut **tx)
        .await?;
    let saved: Workspace = serde_json::from_value(value)?;
    let current = identity(tx, broker, key, &saved.baseline).await?;
    // Phase belongs to execution facts; it may advance without changing cwd.
    let mut expected = saved.clone();
    expected.phase = current.phase.clone();
    require(expected == current, "workspace ownership changed")?;
    Ok(current)
}

async fn prepare(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    key: &RunKey,
    baseline: &str,
) -> Result<Value> {
    let workspace = identity(tx, broker, key, baseline).await?;
    let worker = broker.clone();
    let copy = workspace.clone();
    tokio::task::spawn_blocking(move || worker.prepare(&copy, true)).await??;
    bind(tx, &workspace, None).await?;
    Ok(serde_json::to_value(workspace)?)
}

async fn commit(tx: &mut Tx<'_>, broker: &GitBroker, key: &RunKey, message: &str) -> Result<Value> {
    let workspace = bound(tx, broker, key).await?;
    require(
        workspace.phase == "execution",
        "commit outside execution phase",
    )?;
    let worker = broker.clone();
    let message = message.to_owned();
    let sha = tokio::task::spawn_blocking(move || worker.commit(&workspace, &message)).await??;
    sqlx::query("UPDATE run_workspace SET candidate_sha=$2 WHERE run_id=$1")
        .bind(&key.run_id)
        .bind(&sha)
        .execute(&mut **tx)
        .await?;
    Ok(json!({"sha":sha}))
}

async fn preserve(tx: &mut Tx<'_>, broker: &GitBroker, key: &RunKey) -> Result<Value> {
    let workspace = bound(tx, broker, key).await?;
    let worker = broker.clone();
    let manifest = tokio::task::spawn_blocking(move || worker.preserve(&workspace)).await??;
    let candidate: Option<String> =
        sqlx::query_scalar("SELECT candidate_sha FROM run_workspace WHERE run_id=$1")
            .bind(&key.run_id)
            .fetch_one(&mut **tx)
            .await?;
    let valid = candidate.as_deref() == Some(&manifest.head);
    let value = serde_json::to_value(&manifest)?;
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES ($1,$2,$3)")
        .bind(&key.run_id)
        .bind(&value)
        .bind(valid)
        .execute(&mut **tx)
        .await?;
    Ok(value)
}

async fn source(tx: &mut Tx<'_>, target: &Workspace, run: &str) -> Result<Manifest> {
    let (value, candidate): (Value,bool) = sqlx::query_as("SELECT s.manifest,s.candidate AND COALESCE(w.candidate_sha=s.manifest->>'head',false) FROM workspace_snapshot s JOIN agent_run a ON a.id=s.run_id JOIN run_workspace w ON w.run_id=a.id WHERE a.id=$1 AND a.quiescent AND a.requirement_id=$2 AND a.revision=$3 AND a.phase=$4 AND NOT EXISTS (SELECT 1 FROM agent_run newer WHERE newer.requirement_id=a.requirement_id AND newer.id<>$5 AND newer.run_sequence>a.run_sequence)")
        .bind(run).bind(target.requirement).bind(target.revision).bind(&target.phase).bind(&target.key.run_id).fetch_one(&mut **tx).await?;
    let manifest: Manifest = serde_json::from_value(value)?;
    require(
        manifest.workspace.phase == target.phase,
        "historical phase is invalid",
    )?;
    let selection = recovery(&target.phase, candidate, false, true);
    // Handoff requires independently identity-bound validation, implemented by
    // the validation/delivery issues. Never invent that evidence here.
    require(
        matches!(selection, Recovery::Work | Recovery::Validate),
        "recovery stage requires independent evidence",
    )?;
    Ok(manifest)
}

async fn restore(tx: &mut Tx<'_>, broker: &GitBroker, key: &RunKey, run: &str) -> Result<Value> {
    let mut workspace = identity(tx, broker, key, "").await?;
    let manifest = source(tx, &workspace, run).await?;
    workspace.baseline = manifest.head.clone();
    let worker = broker.clone();
    let copy = workspace.clone();
    tokio::task::spawn_blocking(move || {
        if copy.phase == "validation" {
            worker.restore_candidate(&copy, &manifest)
        } else {
            worker.restore(&copy, &manifest)
        }
    })
    .await??;
    bind(tx, &workspace, Some(run)).await?;
    Ok(serde_json::to_value(workspace)?)
}
