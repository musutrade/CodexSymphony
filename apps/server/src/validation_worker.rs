//! Runtime worker bridge: validate preserved completion candidates before any
//! downstream delivery. Missing deployment validation config leaves them pending.
use crate::{
    git_broker::GitBroker,
    validation_runner::{self, Plan},
    validation_service::{self, Request},
    workspace::{Manifest, Workspace},
};
use serde_json::Value;
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn tick(pool: &PgPool, root: &Path, broker: &GitBroker, plan: &Plan) -> Result<bool> {
    let row:Option<(String,i64,i64,Value)>=sqlx::query_as("SELECT a.id,a.requirement_id,a.revision,s.manifest FROM agent_run a JOIN workspace_snapshot s ON s.run_id=a.id JOIN requirement r ON r.id=a.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_revision rev ON rev.requirement_id=r.id AND rev.revision=a.revision CROSS JOIN repository repo WHERE a.state='Succeeded' AND a.quiescent AND a.phase='validation' AND s.candidate AND r.state='Running' AND r.revision=a.revision AND NOT r.paused AND NOT c.paused AND c.recovery_complete AND repo.id=COALESCE((rev.document->>'repository_id')::bigint,1) AND NOT (repo.document->>'revoked')::boolean AND (rev.document->>'repository_version')::bigint>repo.revoked_through_version AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run live WHERE live.requirement_id=r.id AND NOT live.quiescent) AND NOT EXISTS(SELECT 1 FROM candidate_validation v WHERE v.source_run_id=a.id AND v.result<>'pending') ORDER BY a.run_sequence LIMIT 1").fetch_optional(pool).await?;
    let Some((source, requirement, revision, manifest)) = row else {
        return Ok(false);
    };
    let manifest: Manifest = serde_json::from_value(manifest)?;
    if !crate::storage_service::validation(pool, &source).await? {
        return Ok(false);
    }
    let checkout = restore(broker, &source, &manifest)?;
    let candidate = validation_runner::candidate(&checkout)?;
    let id = format!("validation-{source}");
    let directory = root.join(&id);
    validation_service::validate(
        pool,
        Request {
            id: &id,
            source_run: &source,
            requirement,
            revision,
            checkout: &checkout,
            directory: &directory,
            candidate: &candidate,
            plan,
        },
    )
    .await?;
    Ok(true)
}
fn restore(broker: &GitBroker, source: &str, manifest: &Manifest) -> Result<std::path::PathBuf> {
    let id = format!("validation-{source}");
    let mut target: Workspace = manifest.workspace.clone();
    target.key.run_id = id.clone();
    target.key.request_id = id.clone();
    target.identity = id.clone();
    target.branch = format!("ai/req-{}-{id}", target.requirement);
    target.baseline = manifest.head.clone();
    target.path = broker.path(&id)?.to_string_lossy().into_owned();
    target.phase = "validation".into();
    if !Path::new(&target.path).exists() {
        broker.restore_candidate(&target, manifest)?;
    }
    if broker.head(&target)? != manifest.head {
        return Err("preserved candidate changed".into());
    }
    Ok(target.path.into())
}
