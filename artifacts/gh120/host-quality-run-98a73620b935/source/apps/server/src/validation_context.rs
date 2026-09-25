//! P9 binding for reviewed project validation. Reuses the validation ledger and
//! the environment adapter's independently observed facts, including recovery.
use crate::{
    controlled_contract::{
        Call, ControlledConfig, Evaluation, Operation, Registration, SourceIdentity,
    },
    extension_contract::InvocationIdentity,
    validation::sha256,
    validation_service::Request,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::PathBuf;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Context {
    pub call: Call,
    pub environment_contract: String,
    pub checkout: PathBuf,
    pub directory: PathBuf,
}

pub async fn prepare(pool: &PgPool, r: &Request<'_>) -> Result<Option<Context>> {
    let Some(environment) =
        crate::environment_service::plan(pool, r.requirement, r.revision).await?
    else {
        return Ok(None);
    };
    let saved: Option<Value> =
        sqlx::query_scalar("SELECT hook_context FROM candidate_validation WHERE id=$1")
            .bind(r.id)
            .fetch_one(pool)
            .await?;
    if let Some(saved) = saved {
        let context: Context = serde_json::from_value(saved)?;
        check_request(&context, r)?;
        check_current(pool, &context, "validation").await?;
        return Ok(Some(context));
    }
    freeze_context(pool, r, &environment).await.map(Some)
}

async fn freeze_context(
    pool: &PgPool,
    r: &Request<'_>,
    environment: &crate::environment::Plan,
) -> Result<Context> {
    let task =
        crate::environment_service::context(pool, r.requirement, r.revision, "validation").await?;
    let trusted = r.plan.identity()?;
    current_plan(pool, r.requirement, &trusted.config_sha256).await?;
    // The Plan was selected by the operator-owned repository Runtime route.
    // This adapter registration carries no vendor credential or Gate semantics.
    let registration = Registration {
        id: "reviewed-process-validation".into(),
        implementation_digest: trusted.protected_entry_sha256.clone(),
        operations: Vec::from([Operation::Validate]),
        scope_ref: environment.controlled.environment.host_profile_ref.clone(),
        config_ref: trusted.config_sha256.clone(),
        credential_provider_ref: None,
    };
    let controlled = ControlledConfig {
        protocol_version: 1,
        environment: environment.controlled.environment.clone(),
        extensions: Vec::from([registration.clone()]),
    };
    let call = build_call(pool, r, &task, &registration, &controlled).await?;
    call.validate(
        &task.frozen,
        &controlled,
        std::slice::from_ref(&registration),
    )
    .map_err(crate::environment::protocol)?;
    let context = Context {
        call,
        environment_contract: environment.contract_digest(),
        checkout: r.checkout.into(),
        directory: r.directory.into(),
    };
    sqlx::query("UPDATE candidate_validation SET hook_context=$2,hook_required=true WHERE id=$1 AND hook_context IS NULL AND result='pending'")
        .bind(r.id).bind(json!(context)).execute(pool).await?;
    Ok(context)
}

async fn build_call(
    pool: &PgPool,
    r: &Request<'_>,
    task: &crate::environment_probe::TaskContext,
    registration: &Registration,
    controlled: &ControlledConfig,
) -> Result<Call> {
    let required: Value =
        sqlx::query_scalar("SELECT required_steps FROM candidate_validation WHERE id=$1")
            .bind(r.id)
            .fetch_one(pool)
            .await?;
    Ok(Call {
        identity: InvocationIdentity {
            protocol_version: 1,
            requirement_id: r.requirement,
            revision: r.revision,
            run_id: Some(r.source_run.into()),
            resource_id: registration.scope_ref.clone(),
            invocation_id: r.id.into(),
            attempt: 1,
            config_id: task.frozen.config_id.clone(),
        },
        controlled_config_digest: controlled
            .freeze(std::slice::from_ref(registration))
            .map_err(crate::environment::protocol)?,
        operation: Operation::Validate,
        extension_id: registration.id.clone(),
        implementation_digest: registration.implementation_digest.clone(),
        candidate: Some(SourceIdentity {
            commit: r.candidate.sha.clone(),
            tree: r.candidate.tree.clone(),
        }),
        environment_digest: observed(pool, r.requirement, r.revision, "validation").await?,
        policy_digest: policy(
            pool,
            r.requirement,
            r.revision,
            r.source_run,
            &registration.config_ref,
        )
        .await?,
        deadline_unix_ms: task
            .deadline_unix_ms
            .unwrap_or(i64::MAX)
            .min(crate::runtime_client::now().saturating_mul(1000) + 3_600_000),
        required_checks: serde_json::from_value(required)?,
    })
}

fn check_request(context: &Context, request: &Request<'_>) -> Result<()> {
    let identity = &context.call.identity;
    if (
        identity.requirement_id,
        identity.revision,
        identity.run_id.as_deref(),
    ) != (
        request.requirement,
        request.revision,
        Some(request.source_run),
    ) || context.checkout != request.checkout
        || context.directory != request.directory
    {
        return Err("saved validation task or resource identity differs".into());
    }
    Ok(())
}

async fn policy(
    pool: &PgPool,
    requirement: i64,
    revision: i64,
    source: &str,
    plan: &str,
) -> Result<String> {
    let document: Value = sqlx::query_scalar("SELECT jsonb_build_object('revision',v.document,'manifest',s.manifest) FROM execution_revision v JOIN workspace_snapshot s ON s.run_id=$3 WHERE v.requirement_id=$1 AND v.revision=$2")
        .bind(requirement).bind(revision).bind(source).fetch_one(pool).await?;
    Ok(sha256(serde_json::to_vec(&(document, plan))?))
}

async fn observed(pool: &PgPool, requirement: i64, revision: i64, stage: &str) -> Result<String> {
    let report: Value = sqlx::query_scalar("SELECT report FROM environment_observation WHERE requirement_id=$1 AND revision=$2 AND stage=$3 ORDER BY id DESC LIMIT 1")
        .bind(requirement).bind(revision).bind(stage).fetch_one(pool).await?;
    let report: crate::environment_probe::Report = serde_json::from_value(report)?;
    if !report.passed() {
        return Err("current validation environment is not admitted".into());
    }
    Ok(report
        .actual_digest
        .ok_or("actual validation environment identity missing")?)
}

pub async fn check_current(pool: &PgPool, context: &Context, stage: &str) -> Result<()> {
    let id = &context.call.identity;
    let invalidated: bool =
        sqlx::query_scalar("SELECT hook_invalidated FROM candidate_validation WHERE id=$1")
            .bind(&id.invocation_id)
            .fetch_one(pool)
            .await?;
    if invalidated {
        return Err("validation invalidated by control interruption".into());
    }
    let environment = crate::environment_service::plan(pool, id.requirement_id, id.revision)
        .await?
        .ok_or("validation environment binding removed")?;
    if environment.contract_digest() != context.environment_contract
        || observed(pool, id.requirement_id, id.revision, stage).await?
            != context.call.environment_digest
    {
        return Err("validation environment changed".into());
    }
    if !allowed(pool, id.requirement_id, id.revision).await? {
        return Err("validation authorization is no longer active".into());
    }
    Ok(())
}

pub async fn allowed(pool: &PgPool, requirement: i64, revision: i64) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_control c ON c.requirement_id=r.id JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=(v.document->>'repository_id')::bigint WHERE r.id=$1 AND r.revision=$2 AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT (p.document->>'revoked')::boolean AND p.revoked_through_version<(v.document->>'repository_version')::bigint AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=r.id AND NOT a.quiescent))")
        .bind(requirement).bind(revision).fetch_one(pool).await?)
}

pub async fn record(pool: &PgPool, context: &Context, evaluation: &Evaluation) -> Result<()> {
    sqlx::query("UPDATE candidate_validation SET hook_evaluation=$2 WHERE id=$1 AND hook_context=$3 AND result='pending'")
        .bind(&context.call.identity.invocation_id).bind(json!(evaluation)).bind(json!(context)).execute(pool).await?;
    check_current(pool, context, "validation").await?;
    if evaluation.verdict == crate::controlled_contract::Verdict::Pass {
        evaluation
            .check_pass(
                &context.call,
                crate::runtime_client::now().saturating_mul(1000),
            )
            .map_err(crate::environment::protocol)?;
    }
    Ok(())
}

pub async fn unknown(pool: &PgPool, context: &Context, error: &str) -> Result<()> {
    let evaluation = Evaluation {
        call: context.call.clone(),
        verdict: crate::controlled_contract::Verdict::Unknown,
        checks: Vec::new(),
    };
    let failure = json!({"class":"external_result_unknown","phase":"validation","detail":crate::operator_view::redact_text(error),"evidence":context.directory,"resume":"reconcile original supervisor and retain all evidence before an authorized new attempt"});
    sqlx::query("UPDATE candidate_validation SET result='blocked',hook_evaluation=$2,failure=$3 WHERE id=$1 AND hook_context=$4 AND result='pending'")
        .bind(&context.call.identity.invocation_id).bind(json!(evaluation)).bind(failure.to_string()).bind(json!(context)).execute(pool).await?;
    Ok(())
}

/// Existing GitHub outbox adapter. Other delivery adapters call `admit` with
/// their own frozen task/source identity, without inventing PR or CI facts.
pub async fn delivery(pool: &PgPool, action: &str) -> Result<()> {
    let (validation, requirement, revision, commit, tree): (String, i64, i64, String, String) = sqlx::query_as("SELECT d.validation_id,d.requirement_id,d.revision,d.head_sha,v.candidate_tree FROM delivery d JOIN candidate_validation v ON v.id=d.validation_id WHERE d.action_key=$1")
        .bind(action).fetch_one(pool).await?;
    admit(
        pool,
        &validation,
        requirement,
        revision,
        &SourceIdentity { commit, tree },
    )
    .await
}

/// Host-independent admission. The target adapter supplies its frozen expected
/// source; result identity and provenance remain owned by the core ledger.
pub async fn admit(
    pool: &PgPool,
    validation: &str,
    requirement: i64,
    revision: i64,
    expected: &SourceIdentity,
) -> Result<()> {
    let (context, evaluation, plan, required): (Option<Value>, Option<Value>, Option<Value>, bool) = sqlx::query_as("SELECT hook_context,hook_evaluation,approved_plan,hook_required FROM candidate_validation WHERE id=$1 AND requirement_id=$2 AND revision=$3 AND candidate_sha=$4 AND candidate_tree=$5 AND result='succeeded'")
        .bind(validation).bind(requirement).bind(revision).bind(&expected.commit).bind(&expected.tree).fetch_one(pool).await?;
    let Some(context) = context else {
        if required {
            return Err("environment-bound delivery lacks validation invocation evidence".into());
        }
        return Ok(());
    };
    let (context, evaluation, plan) = decode_retained(context, evaluation, plan)?;
    admit_context(
        pool,
        requirement,
        revision,
        expected,
        &context,
        &evaluation,
        &plan,
    )
    .await
}

async fn admit_context(
    pool: &PgPool,
    requirement: i64,
    revision: i64,
    expected: &SourceIdentity,
    context: &Context,
    evaluation: &Evaluation,
    plan: &crate::validation_runner::Plan,
) -> Result<()> {
    current_plan(pool, requirement, &plan.identity()?.config_sha256).await?;
    if context.call.candidate.as_ref() != Some(expected) {
        return Err("delivery source differs from validation call".into());
    }
    check_current(pool, context, "delivery").await?;
    check_policy(pool, context, requirement, revision, plan).await?;
    retained(context, evaluation, plan)
}

fn decode_retained(
    context: Value,
    evaluation: Option<Value>,
    plan: Option<Value>,
) -> Result<(Context, Evaluation, crate::validation_runner::Plan)> {
    let context: Context = serde_json::from_value(context)?;
    let evaluation: Evaluation =
        serde_json::from_value(evaluation.ok_or("validation evaluation missing")?)?;
    let plan: crate::validation_runner::Plan =
        serde_json::from_value(plan.ok_or("approved validation plan missing")?)?;
    Ok((context, evaluation, plan))
}

async fn check_policy(
    pool: &PgPool,
    context: &Context,
    requirement: i64,
    revision: i64,
    plan: &crate::validation_runner::Plan,
) -> Result<()> {
    let identity = &context.call.identity;
    if identity.requirement_id != requirement
        || identity.revision != revision
        || policy(
            pool,
            requirement,
            revision,
            identity.run_id.as_deref().ok_or("source Run missing")?,
            &plan.identity()?.config_sha256,
        )
        .await?
            != context.call.policy_digest
    {
        return Err("validation policy or baseline changed".into());
    }
    Ok(())
}

fn retained(
    context: &Context,
    evaluation: &Evaluation,
    plan: &crate::validation_runner::Plan,
) -> Result<()> {
    let key = crate::execution::RunKey {
        run_id: context.call.identity.invocation_id.clone(),
        request_id: context.call.identity.invocation_id.clone(),
        incarnation: context.call.identity.invocation_id.clone(),
    };
    crate::validation_supervisor::require_complete(&context.directory, &key)?;
    evaluation
        .check_pass(
            &context.call,
            crate::runtime_client::now().saturating_mul(1000),
        )
        .map_err(crate::environment::protocol)?;
    if !context.directory.join("binding.json").is_file() {
        return Err("validation process evidence unavailable; do not rerun at delivery".into());
    }
    let candidate = crate::validation_runner::candidate(&context.checkout)?;
    let (retained, _) = crate::validation_hook::execute(
        &context.checkout,
        &context.directory,
        &candidate,
        plan,
        &context.call,
    )?;
    if &retained != evaluation {
        return Err("validation evidence changed after acceptance".into());
    }
    Ok(())
}

async fn current_plan(pool: &PgPool, requirement: i64, expected: &str) -> Result<()> {
    let Some(path) = std::env::var_os("RUNTIME_CONFIG") else {
        return Ok(());
    };
    let routes = crate::runtime_routes::Deployment::load(std::path::Path::new(&path))?;
    let Some(((selected, _), runtime)) = routes.selected(pool).await? else {
        return Err("approved validation route no longer available".into());
    };
    let plan = runtime
        .validation
        .as_ref()
        .ok_or("approved validation route removed")?;
    if selected != requirement || plan.identity()?.config_sha256 != expected {
        return Err("approved validation route replaced".into());
    }
    Ok(())
}
