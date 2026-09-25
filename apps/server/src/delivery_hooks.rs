//! Additional blocking hooks. Complete validation remains authoritative.
use crate::{
    controlled_contract::{Operation, Registration},
    delivery_hook_process::Job,
    validation_context::Context,
    validation_runner::Plan,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = crate::delivery_extension::Result<T>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    BeforeDeliver,
    BeforePublish,
    BeforeMerge,
}
impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::BeforeDeliver => "before_deliver",
            Self::BeforePublish => "before_publish",
            Self::BeforeMerge => "before_merge",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    registration: Registration,
    stages: Vec<Stage>,
    plan: Plan,
}

pub async fn run(pool: &PgPool, action: &str, operation: &str, stage: Stage) -> Result<()> {
    let (requirement, revision, validation, source, context): (i64,i64,String,String,Option<Value>) = sqlx::query_as("SELECT d.requirement_id,d.revision,d.validation_id,v.source_run_id,v.hook_context FROM delivery d JOIN candidate_validation v ON v.id=d.validation_id WHERE d.action_key=$1")
        .bind(action).fetch_one(pool).await?;
    let Some(environment) = crate::environment_service::plan(pool, requirement, revision).await?
    else {
        return Ok(());
    };
    let selected = selected_registrations(&environment);
    if selected.is_empty() {
        return Ok(());
    }
    let (context, installed, scope) = load_registry(&environment, context)?;
    for registration in selected {
        let entry = checked_entry(&installed, registration, scope, &context.checkout)?;
        execute(
            pool,
            action,
            operation,
            &source,
            &validation,
            stage,
            entry,
            &context,
        )
        .await?;
    }
    Ok(())
}

fn selected_registrations(environment: &crate::environment::Plan) -> Vec<&Registration> {
    let mut selected = Vec::new();
    for entry in &environment.controlled.extensions {
        if entry.operations.contains(&Operation::BeforeDeliver) {
            selected.push(entry);
        }
    }
    selected
}

fn load_registry(
    environment: &crate::environment::Plan,
    context: Option<Value>,
) -> Result<(Context, Vec<Installed>, &str)> {
    let path = std::env::var_os("DELIVERY_HOOK_REGISTRY")
        .ok_or("reviewed delivery hook registry missing")?;
    let installed = serde_json::from_slice(&std::fs::read(path)?)?;
    let context =
        serde_json::from_value(context.ok_or("delivery hook needs original validation context")?)?;
    let scope = environment_scope(environment)?;
    Ok((context, installed, scope))
}

fn checked_entry<'a>(
    installed: &'a [Installed],
    registration: &Registration,
    scope: &str,
    checkout: &Path,
) -> Result<&'a Installed> {
    let entry = installed_entry(installed, registration, scope)?;
    check_entry(entry, checkout)?;
    Ok(entry)
}

fn environment_scope(environment: &crate::environment::Plan) -> Result<&str> {
    for entry in &environment.controlled.extensions {
        if entry.id == environment.extension_id {
            return Ok(&entry.scope_ref);
        }
    }
    Err("environment scope registration missing".into())
}

fn installed_entry<'a>(
    installed: &'a [Installed],
    registration: &Registration,
    scope: &str,
) -> Result<&'a Installed> {
    if registration.scope_ref != scope {
        return Err("delivery hook repository scope differs".into());
    }
    for entry in installed {
        if &entry.registration == registration {
            return Ok(entry);
        }
    }
    Err("selected delivery hook is not installed".into())
}

fn check_entry(entry: &Installed, checkout: &Path) -> Result<()> {
    let identity = entry.plan.identity()?;
    if entry.registration.credential_provider_ref.is_some()
        || entry.registration.implementation_digest != identity.protected_entry_sha256
        || entry.registration.config_ref
            != crate::validation::sha256(serde_json::to_vec(&(&entry.stages, &entry.plan))?)
        || entry
            .plan
            .entry
            .starts_with(std::fs::canonicalize(checkout)?)
        || entry.stages.is_empty()
    {
        return Err("delivery hook deployment identity or credential boundary differs".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    pool: &PgPool,
    action: &str,
    operation: &str,
    source: &str,
    validation: &str,
    stage: Stage,
    entry: &Installed,
    context: &Context,
) -> Result<()> {
    if !entry.stages.contains(&stage) {
        return Ok(());
    }
    crate::validation_context::delivery(pool, action).await?;
    let identity = crate::validation::sha256(format!(
        "{action}:{operation}:{validation}:{}:{}",
        stage.name(),
        entry.registration.id
    ));
    let call = prepare_call(pool, action, operation, stage, entry, context, &identity).await?;
    let directory = context
        .directory
        .parent()
        .ok_or("validation directory parent missing")?
        .join("delivery-hooks")
        .join(&identity);
    let proposed = Job {
        call,
        checkout: context.checkout.clone(),
        directory,
        candidate: crate::validation_runner::candidate(&context.checkout)?,
        plan: entry.plan.clone(),
    };
    let (job, claimed) = claim(pool, action, operation, source, stage, &proposed).await?;
    let result = crate::delivery_hook_process::execute(pool, &job, claimed).await;
    retain(pool, &identity, &job, &result).await?;
    admit_result(pool, action, &job, result).await
}

async fn admit_result(
    pool: &PgPool,
    action: &str,
    job: &Job,
    result: Result<crate::controlled_contract::Evaluation>,
) -> Result<()> {
    let evaluation = result?;
    evaluation
        .check_pass(&job.call, crate::runtime_client::now().saturating_mul(1000))
        .map_err(crate::environment::protocol)?;
    crate::environment_service::admit(
        pool,
        job.call.identity.requirement_id,
        job.call.identity.revision,
        "delivery",
        None,
    )
    .await?;
    crate::validation_context::delivery(pool, action).await
}

async fn claim(
    pool: &PgPool,
    action: &str,
    operation: &str,
    source: &str,
    stage: Stage,
    proposed: &Job,
) -> Result<(Job, bool)> {
    let identity = &proposed.call.identity.invocation_id;
    let claimed = sqlx::query("INSERT INTO project_hook_invocation(invocation_id,run_id,resource_id,event,hook_name,status,output_dir,input) VALUES($1,$2,$3,$4,$5,'intent',$6,$7) ON CONFLICT DO NOTHING")
        .bind(identity).bind(source).bind(format!("{action}:{operation}")).bind(stage.name()).bind(&proposed.call.extension_id)
        .bind(proposed.directory.to_string_lossy().as_ref()).bind(json!(proposed)).execute(pool).await?.rows_affected() == 1;
    let saved: Value =
        sqlx::query_scalar("SELECT input FROM project_hook_invocation WHERE invocation_id=$1")
            .bind(identity)
            .fetch_one(pool)
            .await?;
    let job: Job = serde_json::from_value(saved)?;
    if json!(job) != json!(proposed) {
        return Err("frozen delivery hook input changed".into());
    }
    Ok((job, claimed))
}

async fn prepare_call(
    pool: &PgPool,
    action: &str,
    operation: &str,
    stage: Stage,
    entry: &Installed,
    context: &Context,
    identity: &str,
) -> Result<crate::controlled_contract::Call> {
    let mut call = context.call.clone();
    call.identity.invocation_id = identity.to_owned();
    call.identity.attempt = 1;
    call.identity.resource_id = action.into();
    call.policy_digest = crate::validation::sha256(serde_json::to_vec(&(
        &context.call.policy_digest,
        action,
        operation,
        stage,
    ))?);
    call.operation = Operation::BeforeDeliver;
    call.extension_id = entry.registration.id.clone();
    call.implementation_digest = entry.registration.implementation_digest.clone();
    call.required_checks.clear();
    for step in &entry.plan.steps {
        call.required_checks.push(step.id.clone());
    }
    let environment = crate::environment_service::plan(
        pool,
        call.identity.requirement_id,
        call.identity.revision,
    )
    .await?
    .ok_or("delivery environment binding removed")?;
    let controlled = crate::controlled_contract::ControlledConfig {
        protocol_version: 1,
        environment: environment.controlled.environment,
        extensions: Vec::from([entry.registration.clone()]),
    };
    let approved = std::slice::from_ref(&entry.registration);
    call.controlled_config_digest = controlled
        .freeze(approved)
        .map_err(crate::environment::protocol)?;
    let task = crate::environment_service::context(
        pool,
        call.identity.requirement_id,
        call.identity.revision,
        "delivery",
    )
    .await?;
    call.validate(&task.frozen, &controlled, approved)
        .map_err(crate::environment::protocol)?;
    Ok(call)
}

async fn retain(
    pool: &PgPool,
    identity: &str,
    job: &Job,
    result: &Result<crate::controlled_contract::Evaluation>,
) -> Result<()> {
    let (status, value) = match result {
        Ok(evaluation) => {
            let status = match evaluation.verdict {
                crate::controlled_contract::Verdict::Fail => "failed",
                crate::controlled_contract::Verdict::Pass
                    if evaluation
                        .check_pass(&job.call, crate::runtime_client::now().saturating_mul(1000))
                        .is_ok() =>
                {
                    "success"
                }
                _ => "unknown",
            };
            (status, json!(evaluation))
        }
        Err(error) => (
            "unknown",
            json!({"error":crate::operator_view::redact_text(&error.to_string())}),
        ),
    };
    let key = crate::execution::RunKey {
        run_id: identity.into(),
        request_id: identity.into(),
        incarnation: identity.into(),
    };
    let stopped = crate::validation_supervisor::quiescent(&job.directory, &key).unwrap_or(false);
    sqlx::query("UPDATE project_hook_invocation SET status=$2,result=$3,stop_confirmed=$4 WHERE invocation_id=$1")
        .bind(identity).bind(status).bind(value).bind(stopped).execute(pool).await?;
    Ok(())
}

/// Read/stop reconciliation remains available after cancellation, revocation or
/// validation expiry. It never launches an invocation or grants publication.
pub async fn reconcile(pool: &PgPool) -> Result<()> {
    let rows: Vec<(String,Value)> = sqlx::query_as("SELECT invocation_id,input FROM project_hook_invocation WHERE input IS NOT NULL AND event IN ('before_deliver','before_publish','before_merge') AND status IN ('intent','running','unknown') ORDER BY created_at LIMIT 1").fetch_all(pool).await?;
    for (identity, input) in rows {
        reconcile_invocation(pool, &identity, input).await?;
    }
    Ok(())
}

async fn reconcile_invocation(pool: &PgPool, identity: &str, input: Value) -> Result<()> {
    let job: Job = serde_json::from_value(input)?;
    if identity != job.call.identity.invocation_id {
        return Err("delivery hook recovery identity mismatch".into());
    }
    let key = crate::execution::RunKey {
        run_id: identity.to_owned(),
        request_id: identity.to_owned(),
        incarnation: identity.to_owned(),
    };
    if !crate::validation_supervisor::quiescent(&job.directory, &key)? {
        std::fs::create_dir_all(&job.directory)?;
        crate::process::durable_write(&job.directory.join("stop.json"), &key)?;
    }
    let result = crate::delivery_hook_process::execute(pool, &job, false).await;
    retain(pool, identity, &job, &result).await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/delivery_hooks.rs"]
mod tests;
