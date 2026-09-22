//! Independently execute the reviewed plan on an actual fetched merge commit.
use crate::{
    automatic_merge::Intent,
    git_broker::GitBroker,
    github_contract::PostMerge,
    github_http::AppClient,
    validation::{self, ValidationEvidence},
    validation_runner::{self, Plan},
    workspace::{Manifest, Workspace},
};
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) async fn plan(pool: &PgPool, intent: &Intent) -> Result<Plan> {
    let contract = intent
        .policy
        .delivery
        .as_ref()
        .ok_or("delivery policy absent")?;
    match &contract.post_merge {
        PostMerge::FixedValidation {
            plan_id,
            configuration_sha256,
            ..
        } => {
            let plan: Plan = serde_json::from_slice(&std::fs::read(plan_id)?)?;
            if plan.identity()?.config_sha256 != *configuration_sha256 {
                return Err("post_merge plan identity changed".into());
            }
            Ok(plan)
        }
        PostMerge::Checks { .. } => source_plan(pool, intent).await,
    }
}

pub(crate) async fn source_plan(pool: &PgPool, intent: &Intent) -> Result<Plan> {
    let (plan,trusted):(Option<serde_json::Value>,serde_json::Value)=sqlx::query_as("SELECT approved_plan,trusted FROM candidate_validation WHERE id=$1 AND result='succeeded' AND candidate_sha=$2")
        .bind(&intent.validation_id).bind(&intent.head).fetch_one(pool).await?;
    let plan: Plan = serde_json::from_value(
        plan.ok_or("approved source plan unavailable; retain original authorization")?,
    )?;
    if serde_json::json!(plan.identity()?) != trusted {
        return Err("approved source validation plan changed".into());
    }
    Ok(plan)
}

pub(crate) async fn checkout(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    intent: &Intent,
    sha: &str,
    now: i64,
) -> Result<PathBuf> {
    let broker = GitBroker::open(&root.join("workspaces"))?;
    let manifest: serde_json::Value =
        sqlx::query_scalar("SELECT manifest FROM delivery WHERE action_key=$1")
            .bind(&intent.delivery_key)
            .fetch_one(pool)
            .await?;
    let manifest: Manifest = serde_json::from_value(manifest)?;
    let repository = broker.delivery_repository(&manifest)?;
    if !broker.contains_commit(sha, sha) {
        client
            .fetch_commit(&intent.policy, &repository, sha, now)
            .await?;
    }
    let id = format!("merge-{}-{}", intent.action_key(), sha);
    let path = broker.path(&id)?;
    let workspace = Workspace {
        key: crate::execution::RunKey {
            run_id: id.clone(),
            request_id: id.clone(),
            incarnation: "merge-validation".into(),
        },
        identity: id.clone(),
        requirement: intent.requirement,
        revision: intent.revision,
        phase: "post_merge".into(),
        baseline: sha.into(),
        branch: format!("ai/req-{}-{id}", intent.requirement),
        path: path.to_string_lossy().into_owned(),
    };
    if path.exists() {
        if broker.head(&workspace)? != sha {
            return Err("saved merge checkout identity differs".into());
        }
    } else {
        broker.prepare(&workspace, true)?;
    }
    Ok(path)
}

pub(crate) async fn execute(
    pool: &PgPool,
    intent: &Intent,
    checkout: PathBuf,
    directory: PathBuf,
    plan: Plan,
    required: Vec<String>,
) -> Result<ValidationEvidence> {
    let stop = StopOnDrop(Arc::new(AtomicBool::new(false)));
    let flag = stop.0.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let candidate = validation_runner::candidate(&checkout)?;
        let trusted = plan.identity()?;
        let steps = validation_runner::execute_cancellable(
            &checkout, &directory, &candidate, &plan, &flag,
        )?;
        let evidence = ValidationEvidence {
            source_before: candidate.tree.clone(),
            source_after: validation_runner::candidate(&checkout)?.tree,
            entry_before: trusted.protected_entry_sha256.clone(),
            entry_after: plan.identity()?.protected_entry_sha256,
            candidate,
            trusted,
            steps,
        };
        validation::verify(&evidence, &evidence.candidate, &evidence.trusted, &required)
            .map_err(|error| format!("post_merge validation failed: {error:?}"))?;
        Ok(evidence)
    });
    loop {
        tokio::select! {
            result = &mut task => return result?,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                // A read failure is conservative: stop local work and preserve
                // its incomplete invocation, rather than continue unauthorised.
                if !execution_allowed(pool, intent).await.unwrap_or(false) {
                    stop.0.store(true, Ordering::Release);
                }
            }
        }
    }
}
async fn execution_allowed(pool: &PgPool, intent: &Intent) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    Ok(crate::merge_store::allowed(&mut tx, intent).await?)
}
struct StopOnDrop(Arc<AtomicBool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
