//! Explicit adapter registration, without directory discovery or format-specific
//! report scraping. Supervisor failures retain facts even without valid JSON.
use crate::{
    diagnostic_store,
    diagnostics::{Binding, Captured, Result},
    extension_contract::{HookOutcome, InvocationIdentity},
    validation::Candidate,
};
use sqlx::PgPool;
use std::path::Path;

pub fn call_binding(call: &crate::controlled_contract::Call, phase: &str) -> Binding {
    let candidate = call.candidate.as_ref().map(call_candidate);
    Binding {
        identity: call.identity.clone(),
        phase: phase.into(),
        candidate,
        validation_id: None,
        generation: call.identity.attempt as i64,
        implementation_digest: call.implementation_digest.clone(),
        environment_digest: call.environment_digest.clone(),
        policy_digest: call.policy_digest.clone(),
    }
}

fn call_candidate(source: &crate::controlled_contract::SourceIdentity) -> Candidate {
    Candidate {
        sha: source.commit.clone(),
        tree: source.tree.clone(),
        immutable: true,
    }
}

pub async fn validation(
    pool: &PgPool,
    r: &crate::validation_service::Request<'_>,
    context: Option<&crate::validation_context::Context>,
) -> Result<()> {
    let document: serde_json::Value = sqlx::query_scalar("SELECT trusted FROM candidate_validation WHERE id=$1 AND source_run_id=$2 AND requirement_id=$3 AND revision=$4").bind(r.id).bind(r.source_run).bind(r.requirement).bind(r.revision).fetch_one(pool).await?;
    let trusted: crate::validation::TrustedIdentity = serde_json::from_value(document)?;
    let mut binding = match context {
        Some(context) => call_binding(&context.call, "validation"),
        None => Binding {
            identity: InvocationIdentity {
                protocol_version: crate::extension_contract::PROTOCOL_VERSION,
                requirement_id: r.requirement,
                revision: r.revision,
                run_id: Some(r.source_run.into()),
                resource_id: r.id.into(),
                invocation_id: r.id.into(),
                attempt: 1,
                config_id: trusted.config_sha256.clone(),
            },
            phase: "validation".into(),
            candidate: Some(r.candidate.clone()),
            validation_id: None,
            generation: 1,
            implementation_digest: trusted.protected_entry_sha256,
            environment_digest: "unknown".into(),
            policy_digest: trusted.config_sha256,
        },
    };
    binding.validation_id = Some(r.id.into());
    binding.generation = sqlx::query_scalar("WITH RECURSIVE generations AS (SELECT id,retry_of FROM candidate_validation WHERE id=$1 UNION ALL SELECT v.id,v.retry_of FROM candidate_validation v JOIN generations g ON v.id=g.retry_of) SELECT count(*)::bigint FROM generations").bind(r.id).fetch_one(pool).await?;
    plan(pool, r.source_run, r.directory, &binding, r.plan).await
}

pub async fn plan(
    pool: &PgPool,
    source: &str,
    directory: &Path,
    binding: &Binding,
    plan: &crate::validation_runner::Plan,
) -> Result<()> {
    let limits = diagnostic_store::limits(pool).await?;
    let mut captures = Vec::new();
    for (index, step) in plan.steps.iter().enumerate() {
        if captures.len() >= crate::diagnostics::MAX_FILES - 1 {
            omitted(binding, &limits, &mut captures)?;
            break;
        }
        let mut step_binding = binding.clone();
        step_binding.phase = format!("{}/step:{}", binding.phase, step.id);
        let path = format!("step-{index}.log");
        let partial = directory
            .join(format!("step-{index}.truncated.json"))
            .exists()
            || directory.join("stop.json").exists();
        let captured = crate::diagnostic_capture::capture(
            directory,
            &path,
            &step_binding,
            limits.file,
            partial,
            limits.expires_at,
        )?;
        let raw = captured.raw.clone();
        captures.push(captured);
        if let Some(raw) = raw {
            feedback_artifacts(directory, &step_binding, step, &raw, &limits, &mut captures)?;
        }
    }
    diagnostic_store::persist(pool, source, captures, &limits).await
}

/// Integration invocations have no coding Run. Keep their own source identity
/// so an explicitly linked repair can read the original version-set failure.
pub async fn integration(
    pool: &PgPool,
    directory: &Path,
    job: &crate::integration_process::Job,
) -> Result<()> {
    let primary = job
        .binding
        .versions
        .first()
        .ok_or("empty integration version set")?;
    let binding = Binding {
        identity: InvocationIdentity {
            protocol_version: crate::extension_contract::PROTOCOL_VERSION,
            requirement_id: job.binding.requirement,
            revision: job.binding.revision,
            run_id: None,
            resource_id: job.invocation.clone(),
            invocation_id: job.invocation.clone(),
            attempt: 1,
            config_id: job.binding.trusted.config_sha256.clone(),
        },
        phase: "integration".into(),
        candidate: Some(primary.candidate.clone()),
        validation_id: Some(job.invocation.clone()),
        generation: 1,
        implementation_digest: job.binding.trusted.protected_entry_sha256.clone(),
        environment_digest: "unknown".into(),
        policy_digest: job.binding.trusted.config_sha256.clone(),
    };
    plan(
        pool,
        &job.invocation,
        &directory.join("checks"),
        &binding,
        &job.plan,
    )
    .await
}

fn feedback_artifacts(
    directory: &Path,
    binding: &Binding,
    step: &crate::validation_runner::Step,
    raw: &[u8],
    limits: &diagnostic_store::Limits,
    captures: &mut Vec<Captured>,
) -> Result<()> {
    if step.command.get(1).map(String::as_str) != Some(crate::extension_feedback::SELECTOR) {
        return Ok(());
    }
    let feedback: crate::extension_feedback::Feedback = match serde_json::from_slice(raw) {
        Ok(feedback) => feedback,
        Err(_) => return Ok(()),
    };
    if feedback.protocol_version != crate::extension_contract::PROTOCOL_VERSION
        || feedback.check_id != step.id
    {
        return Ok(());
    }
    if crate::extension_contract::validate_artifact_refs(&feedback.artifacts).is_err() {
        return Ok(());
    }
    append(directory, binding, &feedback.artifacts, limits, captures)
}

fn append(
    directory: &Path,
    binding: &Binding,
    artifacts: &[crate::extension_contract::ArtifactRef],
    limits: &diagnostic_store::Limits,
    captures: &mut Vec<Captured>,
) -> Result<()> {
    if artifacts.len() > crate::diagnostics::MAX_FILES {
        return Err("diagnostic artifact count exceeds protocol limit".into());
    }
    for artifact in artifacts {
        if captures.len() >= crate::diagnostics::MAX_FILES - 1 {
            omitted(binding, limits, captures)?;
            break;
        }
        captures.push(crate::diagnostic_capture::capture(
            directory,
            &artifact.path,
            binding,
            limits.file,
            false,
            limits.expires_at,
        )?);
    }
    Ok(())
}

pub async fn lifecycle(
    pool: &PgPool,
    source: &str,
    directory: &Path,
    identity: &InvocationIdentity,
    hook: &crate::extension_contract::HookConfig,
) -> Result<()> {
    let candidate = workspace_candidate(pool, source).await?;
    let binding = Binding {
        identity: identity.clone(),
        phase: lifecycle_phase(&hook.event, &hook.name),
        candidate,
        validation_id: None,
        generation: identity.attempt as i64,
        implementation_digest: hook.script_identity.clone(),
        environment_digest: "unknown".into(),
        policy_digest: identity.config_id.clone(),
    };
    let limits = diagnostic_store::limits(pool).await?;
    let partial = directory.join("truncated.json").exists() || directory.join("stop.json").exists();
    let stdout = crate::diagnostic_capture::capture(
        directory,
        "stdout.json",
        &binding,
        limits.file,
        partial,
        limits.expires_at,
    )?;
    let stderr = crate::diagnostic_capture::capture(
        directory,
        "stderr.log",
        &binding,
        limits.file,
        partial,
        limits.expires_at,
    )?;
    let raw = stdout.raw.clone();
    let mut captures = Vec::from([stdout, stderr]);
    if let Some(raw) = raw {
        lifecycle_artifacts(directory, identity, &raw, &binding, &limits, &mut captures)?;
    }
    diagnostic_store::persist(pool, source, captures, &limits).await
}

fn lifecycle_artifacts(
    directory: &Path,
    identity: &InvocationIdentity,
    raw: &[u8],
    binding: &Binding,
    limits: &diagnostic_store::Limits,
    captures: &mut Vec<Captured>,
) -> Result<()> {
    let result = match crate::extension_contract::parse_hook_result(raw, identity) {
        Ok(result) => result,
        Err(_) => return Ok(()),
    };
    let artifacts = match &result.outcome {
        HookOutcome::Success { artifacts } | HookOutcome::Failed { artifacts, .. } => artifacts,
    };
    append(directory, binding, artifacts, limits, captures)
}

async fn workspace_candidate(pool: &PgPool, source: &str) -> Result<Option<Candidate>> {
    let row: Option<(Option<String>,Option<String>,bool)> = sqlx::query_as("SELECT s.manifest->>'head',s.manifest->>'index_tree',s.candidate AND COALESCE(w.candidate_sha=s.manifest->>'head',false) FROM run_workspace w JOIN workspace_snapshot s ON s.run_id=w.run_id WHERE w.run_id=$1").bind(source).fetch_optional(pool).await?;
    if let Some((Some(sha), Some(tree), immutable)) = row {
        return Ok(Some(Candidate {
            sha,
            tree,
            immutable,
        }));
    }
    Ok(None)
}

pub async fn expire(pool: &PgPool, now: i64) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    diagnostic_store::expire_in(&mut tx, now).await?;
    tx.commit().await?;
    Ok(())
}

fn omitted(
    binding: &Binding,
    limits: &diagnostic_store::Limits,
    captures: &mut Vec<Captured>,
) -> Result<()> {
    if captures.len() < crate::diagnostics::MAX_FILES {
        captures.push(crate::diagnostic_capture::missing(
            binding,
            "diagnostic-manifest-limit",
            "manifest quota reached; additional producer evidence was not collected",
            limits.expires_at,
        )?);
    }
    Ok(())
}

fn lifecycle_phase(event: &crate::extension_contract::HookEvent, name: &str) -> String {
    use crate::extension_contract::HookEvent;
    let phase = match event {
        HookEvent::AfterCreate => "after_create",
        HookEvent::BeforeRun => "before_run",
        HookEvent::AfterRun => "after_run",
        HookEvent::BeforeRemove => "before_remove",
    };
    format!("{phase}/hook:{name}")
}

#[cfg(test)]
#[path = "../tests/unit/diagnostic_service.rs"]
mod tests;
