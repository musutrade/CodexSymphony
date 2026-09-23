//! Freeze the current target branch after checking the original reviewed scope.
use crate::{
    git_broker::GitBroker, github_http::AppClient, linked_repair::Scope,
    validation::ValidationEvidence,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(sqlx::FromRow)]
pub(crate) struct Failure {
    pub id: String,
    pub requirement_id: i64,
    pub revision: i64,
    pub merge_key: Option<String>,
    pub integration_id: Option<String>,
    pub evidence: Value,
    pub required_steps: Value,
}
struct Target {
    repository: i64,
    version: i64,
    paths: Vec<String>,
    affected: String,
}

pub(crate) async fn tick(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    now: i64,
) -> Result<()> {
    let row: Option<Failure> = sqlx::query_as("SELECT f.* FROM linked_failure f JOIN execution_control c ON c.requirement_id=f.requirement_id JOIN requirement r ON r.id=f.requirement_id WHERE f.state='observed' AND f.baseline IS NULL AND f.next_source_at<=$1 AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete ORDER BY f.created_at LIMIT 1").bind(now).fetch_optional(pool).await?;
    let Some(f) = row else {
        return Ok(());
    };
    let target = match reviewed_target(pool, &f).await {
        Ok(target) => target,
        Err(error) => return block(pool, &f.id, &error.to_string()).await,
    };
    attempt_source(pool, &mut RemoteGit { client, now }, root, now, &f, target).await
}
async fn reviewed_target(pool: &PgPool, f: &Failure) -> Result<Target> {
    let input: Option<Value> = sqlx::query_scalar("SELECT input FROM group_execution_item WHERE requirement_id=$1 AND NOT frozen AND NOT removed").bind(f.requirement_id).fetch_optional(pool).await?;
    let input = input.ok_or("original reviewed group repair scope absent")?;
    target(pool, f, &input).await
}
async fn attempt_source(
    pool: &PgPool,
    remote: &mut impl Remote,
    root: &Path,
    now: i64,
    f: &Failure,
    target: Target,
) -> Result<()> {
    if !source_attempt(pool, &f.id, now).await? {
        return Ok(());
    }
    if let Err(error) = freeze(pool, remote, root, f, target).await {
        sqlx::query("UPDATE linked_failure SET blocker=$2,source_receipts=source_receipts||jsonb_build_array(jsonb_build_object('attempt',source_attempts,'error',$2::text,'next_attempt_at',next_source_at)) WHERE id=$1 AND state='observed'")
            .bind(&f.id)
            .bind(crate::operator_view::redact_text(&error.to_string()))
            .execute(pool)
            .await?;
    }
    Ok(())
}
async fn target(pool: &PgPool, f: &Failure, input: &Value) -> Result<Target> {
    let evidence: ValidationEvidence = serde_json::from_value(f.evidence.clone())?;
    let required: Vec<String> = serde_json::from_value(f.required_steps.clone())?;
    if let Some(key) = &f.merge_key {
        let repository: i64 = sqlx::query_scalar("SELECT r.id::bigint FROM merge_operation m JOIN repository r ON (r.document->>'github_repository_id')::bigint=(m.intent#>>'{policy,repository_id}')::bigint WHERE m.action_key=$1").bind(key).fetch_one(pool).await?;
        return merged_target(repository, input, &evidence, &required);
    }
    integration_target(pool, f, input, &evidence, &required).await
}
fn merged_target(
    repository: i64,
    input: &Value,
    evidence: &ValidationEvidence,
    required: &[String],
) -> Result<Target> {
    if input["child"]["kind"] == "validation_only" {
        return merged_integration_target(repository, input, evidence, required);
    }
    crate::budget_store::require(
        input["child"]["repository_id"].as_i64() == Some(repository),
        "merged repository differs from original authorization",
    )?;
    Ok(Target {
        repository,
        version: input["review"]["repository_version"]
            .as_i64()
            .ok_or("repository version absent")?,
        paths: Scope::parse(
            input["review"]["repair_scope"]
                .as_str()
                .ok_or("scope absent")?,
        )?
        .paths(evidence, required)?,
        affected: evidence.candidate.sha.clone(),
    })
}
fn merged_integration_target(
    repository: i64,
    input: &Value,
    evidence: &ValidationEvidence,
    required: &[String],
) -> Result<Target> {
    let auth: crate::integration::Authorization =
        serde_json::from_value(input["review"]["integration"].clone())?;
    let repo = auth
        .repositories
        .iter()
        .find(|r| r.repository_id == repository)
        .ok_or("merged repository outside original integration scope")?;
    Ok(Target {
        repository,
        version: repo.repository_version,
        paths: Scope::parse(&repo.repair_scope)?.paths(evidence, required)?,
        affected: evidence.candidate.sha.clone(),
    })
}
async fn integration_target(
    pool: &PgPool,
    f: &Failure,
    input: &Value,
    evidence: &ValidationEvidence,
    required: &[String],
) -> Result<Target> {
    let auth: crate::integration::Authorization =
        serde_json::from_value(input["review"]["integration"].clone())?;
    let binding: Value = sqlx::query_scalar(
        "SELECT binding FROM integration_validation WHERE id=$1 AND quiescent AND state='failed'",
    )
    .bind(&f.integration_id)
    .fetch_one(pool)
    .await?;
    let binding: crate::integration::Binding = serde_json::from_value(binding)?;
    let repaired: Vec<i64> = sqlx::query_scalar("SELECT repository_id FROM linked_failure WHERE requirement_id=$1 AND state IN ('merged','complete') AND repository_id IS NOT NULL").bind(f.requirement_id).fetch_all(pool).await?;
    select_integration_target(auth, &binding, &repaired, evidence, required)
}
fn select_integration_target(
    auth: crate::integration::Authorization,
    binding: &crate::integration::Binding,
    repaired: &[i64],
    evidence: &ValidationEvidence,
    required: &[String],
) -> Result<Target> {
    // The reviewed check-to-repository mappings are the only target authority.
    // Stable reviewed order makes multi-repository repair globally serial.
    let mut targets = Vec::new();
    for repo in auth.repositories {
        if let Ok(paths) =
            Scope::parse(&repo.repair_scope).and_then(|s| s.paths(evidence, required))
        {
            let version = binding
                .versions
                .iter()
                .find(|v| v.repository_id == repo.repository_id)
                .ok_or("affected version absent")?;
            targets.push(Target {
                repository: repo.repository_id,
                version: repo.repository_version,
                paths,
                affected: version.candidate.sha.clone(),
            });
        }
    }
    targets.sort_by_key(|t| (repaired.contains(&t.repository), t.repository));
    targets
        .into_iter()
        .next()
        .ok_or_else(|| "failure outside authorized repair checks".into())
}
struct Frozen {
    target: Target,
    repository: Value,
    baseline: String,
    source: String,
    manifest: crate::workspace::Manifest,
}
trait Remote {
    async fn baseline(&mut self, policy: &crate::github::Policy) -> Result<String>;
    async fn fetch(&mut self, policy: &crate::github::Policy, path: &Path, sha: &str)
    -> Result<()>;
}
struct RemoteGit<'a> {
    client: &'a mut AppClient,
    now: i64,
}
impl Remote for RemoteGit<'_> {
    async fn baseline(&mut self, policy: &crate::github::Policy) -> Result<String> {
        let branch = self
            .client
            .get(
                policy,
                &format!(
                    "/repos/{}/git/ref/heads/{}",
                    policy.repository, policy.default_branch
                ),
                self.now,
            )
            .await?;
        Ok(branch["object"]["sha"]
            .as_str()
            .ok_or("target branch SHA absent")?
            .to_owned())
    }
    async fn fetch(
        &mut self,
        policy: &crate::github::Policy,
        path: &Path,
        sha: &str,
    ) -> Result<()> {
        self.client
            .fetch_commit(policy, path, sha, self.now)
            .await
            .map_err(Into::into)
    }
}
async fn repository_policy(
    pool: &PgPool,
    target: &Target,
) -> Result<(Value, crate::github::Policy)> {
    let (repository, policy): (Value, Value) = sqlx::query_as("SELECT r.document,g.policy FROM repository r JOIN github_repository g ON g.repository_id=(r.document->>'github_repository_id')::bigint WHERE r.id=$1 AND r.version=$2 AND NOT (r.document->>'revoked')::boolean AND r.version>r.revoked_through_version AND g.repository_version=r.version AND NOT g.stale").bind(target.repository).bind(target.version).fetch_one(pool).await?;
    Ok((repository, serde_json::from_value(policy)?))
}
async fn previous_source(
    pool: &PgPool,
    policy: &crate::github::Policy,
) -> Result<(String, crate::workspace::Manifest)> {
    let (source, manifest): (String, Value) = sqlx::query_as("SELECT v.source_run_id,d.manifest FROM delivery d JOIN candidate_validation v ON v.id=d.validation_id JOIN merge_operation m ON m.delivery_key=d.action_key WHERE d.repository_id=$1 AND m.merged_sha IS NOT NULL ORDER BY m.created_at DESC LIMIT 1").bind(policy.repository_id as i64).fetch_one(pool).await?;
    Ok((source, serde_json::from_value(manifest)?))
}
async fn freeze(
    pool: &PgPool,
    remote: &mut impl Remote,
    root: &Path,
    f: &Failure,
    target: Target,
) -> Result<()> {
    let (repository, policy) = repository_policy(pool, &target).await?;
    let (source, manifest) = previous_source(pool, &policy).await?;
    let baseline = remote.baseline(&policy).await?;
    let broker = GitBroker::open(&root.join("workspaces"))?;
    remote
        .fetch(&policy, &broker.delivery_repository(&manifest)?, &baseline)
        .await?;
    freeze_fetched(
        pool,
        &broker,
        f,
        Frozen {
            target,
            repository,
            baseline,
            source,
            manifest,
        },
    )
    .await
}
async fn freeze_fetched(
    pool: &PgPool,
    broker: &GitBroker,
    f: &Failure,
    frozen: Frozen,
) -> Result<()> {
    if !broker.contains_commit(&frozen.baseline, &frozen.target.affected) {
        return block(
            pool,
            &f.id,
            "target branch does not contain the affected version; explicit reconciliation required",
        )
        .await;
    }
    let mut tx = crate::run_store::lock(pool).await?;
    if !still_authorized(&mut tx, f, &frozen).await? {
        return Ok(());
    }
    persist_source(tx, broker, f, &frozen).await
}
async fn still_authorized(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    f: &Failure,
    frozen: &Frozen,
) -> Result<bool> {
    if !crate::group_queue_store::authorized(tx, f.requirement_id).await? {
        return Ok(false);
    }
    let current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_control c ON c.requirement_id=r.id JOIN repository p ON p.id=$3 WHERE r.id=$1 AND r.revision=$2 AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND p.version=$4 AND p.document=$5)")
        .bind(f.requirement_id).bind(f.revision).bind(frozen.target.repository).bind(frozen.target.version).bind(&frozen.repository).fetch_one(&mut **tx).await?;
    Ok(current)
}
async fn execution_document(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    f: &Failure,
    frozen: &Frozen,
) -> Result<Value> {
    let mut document: Value = sqlx::query_scalar(
        "SELECT document FROM requirement_revision WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(f.requirement_id)
    .bind(f.revision)
    .fetch_one(&mut **tx)
    .await?;
    document["repository_id"] = json!(frozen.target.repository);
    document["repository_version"] = json!(frozen.target.version);
    document["repository"] = frozen.repository.clone();

    Ok(document)
}
fn source_workspace(
    broker: &GitBroker,
    f: &Failure,
    frozen: &Frozen,
) -> Result<crate::workspace::Workspace> {
    let id = format!("linked-source-{}", crate::process::new_identity()?);
    let mut workspace = frozen.manifest.workspace.clone();
    workspace.key.run_id = id.clone();
    workspace.key.request_id = id.clone();
    workspace.identity = id.clone();
    workspace.requirement = f.requirement_id;
    workspace.revision = f.revision;
    workspace.baseline = frozen.baseline.clone();
    workspace.branch = format!("ai/req-{}-{id}", f.requirement_id);
    workspace.path = broker.path(&id)?.to_string_lossy().into_owned();

    Ok(workspace)
}
async fn persist_source(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    broker: &GitBroker,
    f: &Failure,
    frozen: &Frozen,
) -> Result<()> {
    let document = execution_document(&mut tx, f, frozen).await?;
    let workspace = source_workspace(broker, f, frozen)?;
    if !crate::storage_service::reserve_workspace(&mut tx, &workspace).await? {
        return Ok(());
    }
    preserve_source(&mut tx, broker, f, frozen, document, &workspace).await?;
    tx.commit().await?;
    Ok(())
}
async fn preserve_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    broker: &GitBroker,
    f: &Failure,
    frozen: &Frozen,
    document: Value,
    workspace: &crate::workspace::Workspace,
) -> Result<()> {
    broker.prepare(workspace, true)?;
    let preserved = broker.preserve(workspace)?;
    sqlx::query("UPDATE linked_failure SET repository_id=$2,document=$3,paths=$4,baseline=$5,manifest=$6,source_run=$7,blocker=NULL WHERE id=$1 AND state='observed' AND baseline IS NULL")
        .bind(&f.id).bind(frozen.target.repository).bind(document).bind(json!(frozen.target.paths)).bind(&frozen.baseline).bind(json!(preserved)).bind(&frozen.source).execute(&mut **tx).await?;

    Ok(())
}
pub(crate) async fn block(pool: &PgPool, id: &str, reason: &str) -> Result<()> {
    sqlx::query(
        "UPDATE linked_failure SET state='blocked',blocker=$2 WHERE id=$1 AND state='observed'",
    )
    .bind(id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

async fn source_attempt(pool: &PgPool, id: &str, now: i64) -> Result<bool> {
    let changed = sqlx::query("UPDATE linked_failure SET source_attempts=source_attempts+1,next_source_at=$2+CASE WHEN source_attempts=0 THEN 30 ELSE 120 END WHERE id=$1 AND state='observed' AND source_attempts<3 AND next_source_at<=$2")
        .bind(id).bind(now).execute(pool).await?;
    if changed.rows_affected() == 1 {
        return Ok(true);
    }
    block(
        pool,
        id,
        "source preparation retry budget exhausted; reconcile original failure",
    )
    .await?;
    Ok(false)
}

#[cfg(test)]
#[path = "../tests/unit/linked_repair_source.rs"]
mod tests;
