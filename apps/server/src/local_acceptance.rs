//! Independent verification of the delivered version under the existing
//! subreaper, with an immutable invocation and the original approved plan.
use crate::{
    delivery_extension::Result,
    git_broker::GitBroker,
    local_delivery_store::{self as store, Job},
    validation::{self, ValidationEvidence},
    validation_runner::Plan,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;

pub async fn accept(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    job: &Job,
    hooks: &Value,
) -> Result<()> {
    let task = prepare(pool, root, broker, job, hooks).await?;
    crate::plugin_scope::admit(
        pool,
        "validation:native",
        &task.invocation,
        job.requirement_id,
        job.revision,
    )
    .await?;
    let claimed = begin(pool, job, &task).await?;
    let outcome = crate::local_acceptance_process::execute(pool, root, job, &task, claimed).await?;
    crate::validation_worker::finish_validation_hook(
        pool,
        root,
        broker,
        &task.invocation,
        &serde_json::from_value(job.manifest.clone())?,
    )
    .await?;
    let evidence = outcome
        .evidence
        .ok_or("local acceptance did not produce complete evidence")?;
    let passed = validation::verify(
        &evidence,
        &task.binding.versions[0].candidate,
        &task.binding.trusted,
        &task.binding.required,
    )
    .is_ok();
    finish(pool, job, &evidence, passed).await
}

async fn prepare(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    job: &Job,
    hooks: &Value,
) -> Result<crate::integration_process::Job> {
    crate::environment_service::admit(
        pool,
        job.requirement_id,
        job.revision,
        "post_delivery_validate",
        None,
    )
    .await?;
    crate::validation_context::delivery(pool, &job.action_key).await?;
    let manifest = serde_json::from_value(job.manifest.clone())?;
    let checkout = restore(pool, broker, job, &manifest).await?;
    let id = format!("local-acceptance-{}", job.action_key);
    if !crate::validation_worker::prepare_validation_hook(
        pool, root, &id, &manifest, &checkout, hooks,
    )
    .await?
    {
        return Err("local acceptance preparation hook incomplete".into());
    }
    approved_task(pool, job, id, checkout).await
}

async fn approved_task(
    pool: &PgPool,
    job: &Job,
    invocation: String,
    checkout: std::path::PathBuf,
) -> Result<crate::integration_process::Job> {
    let (plan, required, trusted, tree): (Value, Value, Value, String) = sqlx::query_as("SELECT approved_plan,required_steps,trusted,candidate_tree FROM candidate_validation WHERE id=$1 AND result='succeeded' AND candidate_sha=$2")
        .bind(&job.validation_id).bind(&job.head_sha).fetch_one(pool).await?;
    let plan: Plan = serde_json::from_value(plan)?;
    let trusted: validation::TrustedIdentity = serde_json::from_value(trusted)?;
    let candidate = crate::validation_runner::candidate(&checkout)?;
    check_input(&plan, &trusted, &candidate, &job.head_sha, &tree)?;
    task(
        pool,
        job,
        invocation,
        checkout,
        plan,
        trusted,
        serde_json::from_value(required)?,
        candidate,
    )
    .await
}

fn check_input(
    plan: &Plan,
    trusted: &validation::TrustedIdentity,
    candidate: &validation::Candidate,
    head: &str,
    tree: &str,
) -> Result<()> {
    if plan.identity()? != *trusted || candidate.sha != head || candidate.tree != tree {
        return Err("local acceptance source or approved plan changed".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn task(
    pool: &PgPool,
    job: &Job,
    invocation: String,
    checkout: std::path::PathBuf,
    plan: Plan,
    trusted: validation::TrustedIdentity,
    required: Vec<String>,
    candidate: validation::Candidate,
) -> Result<crate::integration_process::Job> {
    let authorization: Option<i64> = sqlx::query_scalar(
        "SELECT authorization_id FROM group_execution_item WHERE requirement_id=$1",
    )
    .bind(job.requirement_id)
    .fetch_optional(pool)
    .await?;
    Ok(crate::integration_process::Job {
        invocation,
        plan,
        checkouts: Vec::from([checkout]),
        output_limit: crate::storage_service::entry_limit(pool).await?,
        binding: crate::integration::Binding {
            requirement: job.requirement_id,
            revision: job.revision,
            authorization: authorization.unwrap_or(0),
            input_sha256: validation::sha256(serde_json::to_vec(&(
                &job.action_key,
                &job.policy,
                &job.local_binding,
            ))?),
            versions: Vec::from([crate::integration::Version {
                repository_id: job.binding()?.target.repository_id,
                github_repository_id: 0,
                repository_version: job.binding()?.target.repository_version,
                candidate,
                artifacts: Vec::from([format!("local-delivery:{}", job.action_key)]),
            }]),
            trusted,
            required,
        },
    })
}

async fn begin(pool: &PgPool, job: &Job, task: &crate::integration_process::Job) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !store::allowed(&mut tx, job).await? {
        return Err("local acceptance is not authorized".into());
    }
    reserve_storage(&mut tx, task).await?;
    let changed = sqlx::query("UPDATE delivery SET local_acceptance_started=true,local_acceptance_job=$2 WHERE action_key=$1 AND NOT local_acceptance_started")
        .bind(&job.action_key).bind(json!(task)).execute(&mut *tx).await?.rows_affected()==1;
    let saved: Value =
        sqlx::query_scalar("SELECT local_acceptance_job FROM delivery WHERE action_key=$1")
            .bind(&job.action_key)
            .fetch_one(&mut *tx)
            .await?;
    if saved != json!(task) {
        return Err("local acceptance invocation identity changed".into());
    }
    tx.commit().await?;
    Ok(changed)
}

async fn finish(
    pool: &PgPool,
    job: &Job,
    evidence: &ValidationEvidence,
    passed: bool,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let authorized = retain_authorized(&mut tx, job, evidence, passed).await?;
    if authorized {
        finish_current(&mut tx, job, evidence).await?;
    }
    tx.commit().await?;
    if !authorized {
        return Err("local acceptance failed or authority changed".into());
    }
    Ok(())
}
async fn retain_authorized(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: &Job,
    evidence: &ValidationEvidence,
    passed: bool,
) -> Result<bool> {
    retain(tx, job, evidence, passed).await?;
    if !passed {
        crate::local_repair::failed(tx, job, evidence).await?;
        return Ok(false);
    }
    store::allowed(tx, job).await
}

async fn finish_current(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: &Job,
    evidence: &ValidationEvidence,
) -> Result<()> {
    if crate::local_repair::accepted(tx, job, evidence).await? {
        return Ok(());
    }
    record_child(tx, job, evidence).await?;
    complete(tx, job).await
}

async fn complete(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, job: &Job) -> Result<()> {
    sqlx::query("UPDATE delivery SET released=true WHERE action_key=$1")
        .bind(&job.action_key)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE requirement SET state='Done',version=version+1 WHERE id=$1 AND state='Submitted'",
    )
    .bind(job.requirement_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE execution_control SET requirement_id=NULL WHERE requirement_id=$1")
        .bind(job.requirement_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE candidate_validation SET stage='done' WHERE id=$1")
        .bind(&job.validation_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn retain(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: &Job,
    evidence: &ValidationEvidence,
    passed: bool,
) -> Result<()> {
    let result = json!({"passed":passed,"evidence":evidence});
    sqlx::query(
        "UPDATE delivery SET local_acceptance=$2 WHERE action_key=$1 AND local_acceptance IS NULL",
    )
    .bind(&job.action_key)
    .bind(&result)
    .execute(&mut **tx)
    .await?;
    let saved: Value =
        sqlx::query_scalar("SELECT local_acceptance FROM delivery WHERE action_key=$1")
            .bind(&job.action_key)
            .fetch_one(&mut **tx)
            .await?;
    if saved != result {
        return Err("local acceptance evidence conflict".into());
    }
    Ok(())
}

async fn record_child(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: &Job,
    evidence: &ValidationEvidence,
) -> Result<()> {
    let group: Option<(i64, Value)> = sqlx::query_as(
        "SELECT authorization_id,input FROM group_execution_item WHERE requirement_id=$1",
    )
    .bind(job.requirement_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((authorization, input)) = group {
        crate::budget_store::require(
            input["child"]["repository_id"] == job.policy["repository_id"]
                && input["review"]["repository_version"] == job.policy["repository_version"],
            "local completion repository differs from group authorization",
        )?;
        let fact = json!({"source":"platform-local-delivery/v1","requirement_id":job.requirement_id,"authorization_id":authorization,"child_revision":input["review"]["revision"],"repository_id":job.policy["repository_id"],"delivery_version":job.head_sha,"acceptance_sha":evidence.candidate.sha,"acceptance_plan":input["review"]["verification"],"action_key":job.action_key,"target":job.local_binding,"evidence_sha256":validation::sha256(serde_json::to_vec(evidence)?),"artifact":format!("local-delivery:{}",job.action_key)});
        sqlx::query("INSERT INTO group_completion(requirement_id,authorization_id,fact) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(job.requirement_id).bind(authorization).bind(&fact).execute(&mut **tx).await?;
        let saved: Value =
            sqlx::query_scalar("SELECT fact FROM group_completion WHERE requirement_id=$1")
                .bind(job.requirement_id)
                .fetch_one(&mut **tx)
                .await?;
        crate::budget_store::require(saved == fact, "local completion identity conflict")?;
    }
    Ok(())
}

async fn restore(
    pool: &PgPool,
    broker: &GitBroker,
    job: &Job,
    manifest: &crate::workspace::Manifest,
) -> Result<std::path::PathBuf> {
    let mut workspace = manifest.workspace.clone();
    let id = format!("local-checkout-{}", job.action_key);
    workspace.key.run_id = id.clone();
    workspace.key.request_id = id.clone();
    workspace.identity = id.clone();
    workspace.path = broker.path(&id)?.to_string_lossy().into_owned();
    workspace.branch = format!("ai/req-{}-{id}", job.requirement_id);
    workspace.baseline = job.head_sha.clone();
    workspace.phase = "integration".into();
    if !crate::storage_service::admit_workspace(pool, &workspace).await? {
        return Err("local acceptance workspace storage unavailable".into());
    }
    if !Path::new(&workspace.path).exists() {
        broker.restore_candidate(&workspace, manifest)?;
    }
    Ok(workspace.path.into())
}

async fn reserve_storage(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &crate::integration_process::Job,
) -> Result<()> {
    if crate::storage_store::deployment(tx).await?.is_some() {
        crate::storage_inventory::attempt(
            tx,
            &task.invocation,
            task.binding.requirement,
            task.binding.revision,
        )
        .await?;
        crate::budget_store::require(
            crate::storage_service::reserve_integration(tx, &task.invocation).await?,
            "local acceptance evidence storage unavailable",
        )?;
    }
    Ok(())
}
