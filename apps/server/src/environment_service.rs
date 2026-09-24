//! Environment admission reads immutable review snapshots, never Agent input.
use crate::{
    environment::Plan,
    environment_host::{Registry, Result},
    environment_probe,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;

pub async fn plan(pool: &PgPool, requirement: i64, revision: i64) -> Result<Option<Plan>> {
    let value: Value = sqlx::query_scalar("SELECT COALESCE(document->'repository','{}'::jsonb) FROM execution_revision WHERE requirement_id=$1 AND revision=$2")
        .bind(requirement).bind(revision).fetch_one(pool).await?;
    decode(&value)
}

fn decode(repository: &Value) -> Result<Option<Plan>> {
    match repository.get("environment") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(serde_json::from_str(value)?)),
        Some(_) => Err("environment configuration must be JSON text".into()),
    }
}

pub async fn enable(plan: Option<&Plan>) -> Result<()> {
    let Some(plan) = plan else {
        return Ok(());
    };
    let registry = Registry::load()?;
    for role in ["dev", "test"] {
        require(environment_probe::check(&registry, plan, "enable", role, None).await?)?;
    }
    Ok(())
}

pub async fn startup(pool: &PgPool) -> Result<()> {
    let repositories: Vec<Value> = sqlx::query_scalar(
        "SELECT document FROM repository WHERE NOT (document->>'revoked')::boolean",
    )
    .fetch_all(pool)
    .await?;
    for repository in repositories {
        if let Some(plan) = decode(&repository)? {
            let registry = Registry::load()?;
            for role in ["dev", "test"] {
                require(environment_probe::check(&registry, &plan, "startup", role, None).await?)?;
            }
        }
    }
    Ok(())
}

pub async fn admit(
    pool: &PgPool,
    requirement: i64,
    revision: i64,
    stage: &str,
    workspace: Option<&Path>,
) -> Result<()> {
    let Some(plan) = plan(pool, requirement, revision).await? else {
        return Ok(());
    };
    let role = role(stage);
    if stage == "ci" && !plan.ci {
        return Ok(());
    }
    let registry = Registry::load()?;
    let context = context(pool, requirement, revision, stage).await?;
    let mut report =
        environment_probe::check_bound(&registry, &plan, stage, role, workspace, Some(&context))
            .await?;
    record_admission(pool, requirement, revision, &plan, &mut report).await?;
    require(report)
}

async fn record_admission(
    pool: &PgPool,
    requirement: i64,
    revision: i64,
    plan: &Plan,
    report: &mut environment_probe::Report,
) -> Result<()> {
    if let Err(error) = current(pool, requirement, revision, plan).await {
        report.error = Some(error.to_string());
    }
    crate::process::durable_write(&report.evidence.join("admission.json"), report)?;
    sqlx::query("INSERT INTO environment_observation(requirement_id,revision,stage,report) VALUES($1,$2,$3,$4)")
        .bind(requirement).bind(revision).bind(&report.request.stage).bind(json!(report)).execute(pool).await?;
    Ok(())
}

async fn current(pool: &PgPool, requirement: i64, revision: i64, plan: &Plan) -> Result<()> {
    Registry::load()?.resolve(plan)?;
    let authorized: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=(v.document->>'repository_id')::bigint WHERE r.id=$1 AND r.revision=$2 AND NOT r.cancel_requested AND p.revoked_through_version<(v.document->>'repository_version')::bigint)")
        .bind(requirement).bind(revision).fetch_one(pool).await?;
    if !authorized {
        return Err("environment authorization changed during probe".into());
    }
    Ok(())
}

async fn context(
    pool: &PgPool,
    requirement: i64,
    revision: i64,
    stage: &str,
) -> Result<environment_probe::TaskContext> {
    use crate::extension_contract::{Capabilities, ExtensionConfig};
    let document: Value = sqlx::query_scalar(
        "SELECT document FROM execution_revision WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(requirement)
    .bind(revision)
    .fetch_one(pool)
    .await?;
    let repository: crate::contract::Repository =
        serde_json::from_value(document["repository"].clone())?;
    let id = document["repository_id"]
        .as_i64()
        .ok_or("environment repository identity missing")?;
    let version = document["repository_version"]
        .as_i64()
        .ok_or("environment repository version missing")?;
    if !repository
        .environment
        .as_ref()
        .is_some_and(|plan| plan.matches_repository(id, version))
    {
        return Err("environment repository binding changed".into());
    }
    let extension = ExtensionConfig::from_legacy_repository(&repository);
    let mut capabilities = Capabilities::legacy_codex(repository.model.clone());
    // Reconstruct the immutable review identity only. This does not dispatch or
    // authorize any lifecycle Hook; project_hooks checks its host allowlist.
    capabilities.hooks = repository.hooks.clone();
    let frozen = extension
        .freeze(&capabilities)
        .map_err(crate::environment::protocol)?;
    let deadline_unix_ms: Option<i64> = sqlx::query_scalar("SELECT ((retry->>'started_at')::bigint+600)*1000 FROM preparation_record WHERE requirement_id=$1 AND revision=$2 AND NOT ready AND retry->>'phase'=$3 AND retry->>'todo'='false' AND retry->'next_attempt_at'='null'::jsonb ORDER BY (retry->>'started_at')::bigint DESC LIMIT 1")
        .bind(requirement).bind(revision).bind(stage).fetch_optional(pool).await?;
    Ok(environment_probe::TaskContext {
        requirement,
        revision,
        frozen,
        policy_digest: crate::validation::sha256(serde_json::to_vec(&repository.policy)?),
        deadline_unix_ms,
    })
}

pub fn role(stage: &str) -> &'static str {
    if matches!(stage, "validation" | "delivery" | "ci") {
        "test"
    } else {
        "dev"
    }
}

pub async fn recovery(pool: &PgPool) -> Result<()> {
    let active: Option<(i64,i64)> = sqlx::query_as("SELECT r.id,r.revision FROM execution_control c JOIN requirement r ON r.id=c.requirement_id WHERE c.id=1")
        .fetch_optional(pool).await?;
    if let Some((requirement, revision)) = active {
        admit(pool, requirement, revision, "recovery", None).await?;
    }
    Ok(())
}

fn require(report: environment_probe::Report) -> Result<()> {
    if report.passed() {
        return Ok(());
    }
    Err(format!(
        "environment admission blocked: {}; evidence={}",
        serde_json::to_string(&json!({"differences":report.differences,"error":report.error}))?,
        report.evidence.display()
    )
    .into())
}

pub async fn run(pool: &PgPool, run: &str, stage: &str, workspace: &Path) -> Result<()> {
    let (requirement, revision): (i64, i64) =
        sqlx::query_as("SELECT requirement_id,revision FROM agent_run WHERE id=$1")
            .bind(run)
            .fetch_one(pool)
            .await?;
    admit(pool, requirement, revision, stage, Some(workspace)).await
}
