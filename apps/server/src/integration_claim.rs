//! Atomically freeze a reviewed version combination and acquire the global owner.
use crate::{
    budget_store::require,
    execution::{Launch, RunKey},
    git_broker::GitBroker,
    group_review::Item,
    integration::{Binding, Selection, Version},
    integration_process::Job,
    process, run_store, validation,
    validation_runner::{self, Plan},
    workspace::Workspace,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Tx<'a> = Transaction<'a, Postgres>;

struct Input {
    requirement: i64,
    revision: i64,
    authorization: i64,
    input: Value,
    facts: Vec<crate::delivered_version::Completion>,
}
pub(crate) async fn claim(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    incarnation: &str,
    plan: &Plan,
) -> Result<bool> {
    let limit = prepare(pool).await?;
    let mut tx = run_store::lock(pool).await?;
    let Some(input) = admission(&mut tx, incarnation).await? else {
        return Ok(false);
    };
    let id = format!("integration-{}", process::new_identity()?);
    let key = RunKey {
        run_id: id.clone(),
        request_id: id.clone(),
        incarnation: incarnation.into(),
    };
    let job = freeze(&mut tx, broker, &key, &input, plan, limit).await?;
    let launch = Launch {
        key,
        workspace: job.checkouts[0].to_string_lossy().into_owned(),
        workspace_identity: id.clone(),
        program: supervisor.to_string_lossy().into_owned(),
        args: Vec::from([
            "--integration-validation".into(),
            root.join(&id).to_string_lossy().into_owned(),
        ]),
    };
    persist(&mut tx, &id, &job, &launch).await?;
    tx.commit().await?;
    Ok(true)
}
async fn prepare(pool: &PgPool) -> Result<u64> {
    crate::group_queue_store::materialize(pool).await?;
    crate::storage_service::entry_limit(pool).await
}
async fn admission(tx: &mut Tx<'_>, incarnation: &str) -> Result<Option<Input>> {
    if !run_store::claim_allowed(tx, incarnation).await? {
        return Ok(None);
    }
    let Some((requirement, revision, false)) = crate::group_queue_store::head(tx).await? else {
        return Ok(None);
    };
    authorized_input(tx, requirement, revision).await
}
async fn authorized_input(
    tx: &mut Tx<'_>,
    requirement: i64,
    revision: i64,
) -> Result<Option<Input>> {
    let row: Option<(i64, Value)> = sqlx::query_as("SELECT authorization_id,input FROM group_execution_item WHERE requirement_id=$1 AND input#>>'{child,kind}'='validation_only'")
        .bind(requirement).fetch_optional(&mut **tx).await?;
    let Some((authorization, input)) = row else {
        return Ok(None);
    };
    if !crate::group_queue_store::authorized(tx, requirement).await?
        || !crate::group_budget::prepaid_fits(tx, requirement).await?
    {
        return Ok(None);
    }
    let Some(facts) = prior(tx, requirement).await? else {
        return Ok(None);
    };
    Ok(Some(Input {
        requirement,
        revision,
        authorization,
        input,
        facts,
    }))
}
fn reviewed_plan(
    item: &Item,
    plan: &Plan,
) -> Result<(
    crate::integration::Authorization,
    crate::validation::TrustedIdentity,
    Vec<String>,
)> {
    let auth = item
        .integration
        .clone()
        .ok_or("integration authorization absent")?;
    let trusted = plan.identity()?;
    require(
        auth.configuration_sha256 == trusted.config_sha256,
        "integration configuration differs from reviewed identity",
    )?;
    require(
        !item.verification.is_empty()
            && item.verification.iter().all(|v| {
                plan.steps.iter().any(|s| {
                    s.id == v.step.id && s.timeout_seconds <= v.step.timeout_seconds as u64
                })
            }),
        "required integration AC or timeout missing from pinned plan",
    )?;
    Ok((
        auth,
        trusted,
        item.verification
            .iter()
            .map(|v| v.step.id.clone())
            .collect(),
    ))
}
async fn freeze(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    key: &RunKey,
    input: &Input,
    plan: &Plan,
    limit: u64,
) -> Result<Job> {
    let item: Item = serde_json::from_value(input.input["review"].clone())?;
    let (auth, trusted, required) = reviewed_plan(&item, plan)?;
    let (versions, checkouts) = checkouts(tx, broker, key, input, auth.repositories).await?;
    let binding = Binding {
        requirement: input.requirement,
        revision: input.revision,
        authorization: input.authorization,
        input_sha256: validation::sha256(serde_json::to_vec(&input.input)?),
        versions,
        trusted,
        required,
    };
    Ok(Job {
        invocation: key.run_id.clone(),
        binding,
        plan: plan.clone(),
        checkouts,
        output_limit: limit,
    })
}
async fn checkouts(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    key: &RunKey,
    input: &Input,
    mut repositories: Vec<crate::integration::Repository>,
) -> Result<(Vec<Version>, Vec<std::path::PathBuf>)> {
    let primary = input.input["child"]["repository_id"]
        .as_i64()
        .ok_or("primary repository absent")?;
    repositories.sort_by_key(|r| (r.repository_id != primary, r.repository_id));
    let mut versions = Vec::new();
    let mut checkouts = Vec::new();
    for repository in repositories {
        let (version, path) = checkout(
            tx,
            broker,
            key,
            input.requirement,
            input.revision,
            &repository,
            &input.facts,
        )
        .await?;
        versions.push(version);
        checkouts.push(path);
    }
    Ok((versions, checkouts))
}
pub(crate) async fn persist(tx: &mut Tx<'_>, id: &str, job: &Job, launch: &Launch) -> Result<()> {
    let b = &job.binding;
    register_storage(tx, id, b).await?;
    sqlx::query("INSERT INTO integration_validation(id,requirement_id,authorization_id,revision,binding,job,launch,state) VALUES($1,$2,$3,$4,$5,$6,$7,'prepared')")
        .bind(id).bind(b.requirement).bind(b.authorization).bind(b.revision).bind(json!(b)).bind(json!(job)).bind(json!(launch)).execute(&mut **tx).await?;
    sqlx::query("UPDATE execution_control SET requirement_id=$1 WHERE id=1")
        .bind(b.requirement)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE requirement SET state='Running',version=version+1 WHERE id=$1 AND state<>'Running'",
    )
    .bind(b.requirement)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
async fn register_storage(tx: &mut Tx<'_>, id: &str, binding: &Binding) -> Result<()> {
    if crate::storage_store::deployment(tx).await?.is_some() {
        crate::storage_inventory::attempt(tx, id, binding.requirement, binding.revision).await?;
        require(
            crate::storage_service::reserve_integration(tx, id).await?,
            "integration supervisor storage budget exhausted",
        )?;
    }
    Ok(())
}
async fn checkout(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    key: &RunKey,
    requirement: i64,
    revision: i64,
    repository: &crate::integration::Repository,
    facts: &[crate::delivered_version::Completion],
) -> Result<(Version, std::path::PathBuf)> {
    let github: i64 = sqlx::query_scalar("SELECT COALESCE((document->>'github_repository_id')::bigint,0) FROM repository WHERE id=$1 AND version=$2 AND NOT (document->>'revoked')::boolean AND version>revoked_through_version")
        .bind(repository.repository_id).bind(repository.repository_version).fetch_one(&mut **tx).await?;
    let (sha, artifacts) = select_version(broker, repository, github, facts)?;
    let identity = format!("{}-repo-{}", key.run_id, repository.repository_id);
    let path = broker.path(&identity)?;
    let workspace = Workspace {
        key: RunKey {
            run_id: identity.clone(),
            request_id: identity.clone(),
            incarnation: key.incarnation.clone(),
        },
        identity: identity.clone(),
        requirement,
        revision,
        phase: "integration".into(),
        baseline: sha,
        branch: format!("ai/req-{requirement}-{identity}"),
        path: path.to_string_lossy().into_owned(),
    };
    require(
        crate::storage_service::reserve_workspace(tx, &workspace).await?,
        "integration storage budget exhausted",
    )?;
    broker.prepare(&workspace, true)?;
    let candidate = validation_runner::candidate(&path)?;
    require(
        candidate.sha == workspace.baseline,
        "integration checkout differs from frozen version",
    )?;
    Ok((
        Version {
            repository_id: repository.repository_id,
            github_repository_id: github,
            repository_version: repository.repository_version,
            candidate,
            artifacts,
        },
        path,
    ))
}

fn select_version(
    broker: &GitBroker,
    repository: &crate::integration::Repository,
    github: i64,
    facts: &[crate::delivered_version::Completion],
) -> Result<(String, Vec<String>)> {
    let mut relevant = Vec::new();
    for fact in facts {
        if fact.repository_id() == repository.repository_id {
            relevant.push(fact);
        }
    }
    let sha = match &repository.selection {
        Selection::Fixed { sha } => sha.clone(),
        Selection::CompletedDependencies => relevant
            .last()
            .ok_or("required repository dependency has no completed version")?
            .commit()
            .to_owned(),
    };
    let mut artifacts = Vec::new();
    for fact in relevant {
        require(
            fact.github_id() == github && broker.contains_commit(&sha, fact.commit()),
            "same-repository version must include completed dependencies",
        )?;
        artifacts.push(fact.artifact().to_owned());
    }
    Ok((sha, artifacts))
}
async fn prior(
    tx: &mut Tx<'_>,
    requirement: i64,
) -> Result<Option<Vec<crate::delivered_version::Completion>>> {
    if crate::group_completion::dependencies(tx, requirement)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let rows: Vec<Option<Value>> = sqlx::query_scalar("SELECT c.fact FROM group_execution_item current JOIN group_execution_item i ON i.draft_id=current.draft_id AND NOT i.removed AND COALESCE(i.queue_order,(i.input#>>'{child,order}')::bigint)<COALESCE(current.queue_order,(current.input#>>'{child,order}')::bigint) LEFT JOIN requirement r ON r.id=i.requirement_id AND NOT r.cancel_requested LEFT JOIN group_completion c ON c.requirement_id=r.id AND c.authorization_id=i.authorization_id WHERE current.requirement_id=$1 ORDER BY COALESCE(i.queue_order,(i.input#>>'{child,order}')::bigint)")
        .bind(requirement).fetch_all(&mut **tx).await?;
    let mut facts = Vec::new();
    for row in rows {
        let Some(fact) = row else {
            return Ok(None);
        };
        if fact["source"] != "platform-integration-validation/v1" {
            facts.push(crate::delivered_version::decode(tx, fact).await?);
        }
    }
    Ok(Some(facts))
}
