//! Register existing producers without requiring a successful final report.
use crate::{
    storage_lifecycle::{Category, Identity, Kind},
    storage_store::{self as store, Deployment, Material, Result, Tx},
    workspace::Workspace,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub async fn attempt(
    tx: &mut Tx<'_>,
    run: &str,
    requirement: i64,
    revision: i64,
) -> Result<Identity> {
    let existing: Option<Value> =
        sqlx::query_scalar("SELECT identity FROM storage_attempt WHERE run_id=$1")
            .bind(run)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(existing) = existing {
        return Ok(serde_json::from_value(existing)?);
    }
    let review: Value = sqlx::query_scalar(
        "SELECT document FROM requirement_revision WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(requirement)
    .bind(revision)
    .fetch_one(&mut **tx)
    .await?;
    let ordinal: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sequence),0)+1 FROM storage_attempt")
            .fetch_one(&mut **tx)
            .await?;
    let identity = Identity {
        repository: review["repository"]["remote"].as_str().unwrap_or("").into(),
        requirement,
        revision,
        run: run.into(),
        attempt: ordinal as u64,
        candidate: None,
        stage: "execution".into(),
        policy: format!("{:x}", Sha256::digest(serde_json::to_vec(&review)?)),
        pr: None,
    };
    store::register(tx, &identity, &json!({"review":review})).await?;
    Ok(identity)
}

pub async fn discover(tx: &mut Tx<'_>, config: &Deployment, now: i64) -> Result<()> {
    let attempts: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT id,requirement_id,revision FROM agent_run ORDER BY run_sequence")
            .fetch_all(&mut **tx)
            .await?;
    for (run, requirement, revision) in attempts {
        attempt(tx, &run, requirement, revision).await?;
    }
    preparing(tx).await?;
    let runs: Vec<String> =
        sqlx::query_scalar("SELECT run_id FROM storage_attempt ORDER BY sequence")
            .fetch_all(&mut **tx)
            .await?;
    for run in runs {
        directories(tx, config, &run, now).await?;
    }
    bind_candidates(tx).await?;
    summaries(tx, config.policy.entry_bytes).await
}

async fn preparing(tx: &mut Tx<'_>) -> Result<()> {
    let preparing: Vec<(String,i64,i64)> = sqlx::query_as(
        "SELECT launch#>>'{key,run_id}',requirement_id,revision FROM initial_run UNION SELECT run_id,requirement_id,revision FROM preparation_record UNION SELECT job#>>'{launch,key,run_id}',(job#>>'{workspace,requirement}')::bigint,(job#>>'{workspace,revision}')::bigint FROM runtime_resume UNION SELECT launch#>>'{key,run_id}',requirement_id,(workspace->>'revision')::bigint FROM repair_reservation WHERE launch IS NOT NULL")
        .fetch_all(&mut **tx).await?;
    for (run, requirement, revision) in preparing {
        attempt(tx, &run, requirement, revision).await?;
    }
    Ok(())
}

async fn bind_candidates(tx: &mut Tx<'_>) -> Result<()> {
    sqlx::query("UPDATE storage_attempt s SET identity=s.identity || jsonb_build_object('candidate',v.candidate_sha,'pr',d.pr_number) FROM candidate_validation v LEFT JOIN delivery d ON d.validation_id=v.id WHERE v.source_run_id=s.run_id AND v.id=(SELECT current.id FROM candidate_validation current WHERE current.source_run_id=s.run_id AND current.superseded_by IS NULL ORDER BY current.started_at DESC NULLS LAST,current.id DESC LIMIT 1) AND (s.identity->'candidate' IS DISTINCT FROM to_jsonb(v.candidate_sha) OR s.identity->'pr' IS DISTINCT FROM COALESCE(to_jsonb(d.pr_number),'null'::jsonb))")
        .execute(&mut **tx).await?;
    Ok(())
}

async fn directories(tx: &mut Tx<'_>, config: &Deployment, run: &str, now: i64) -> Result<()> {
    crate::workspace_files::component(run)?;
    for (suffix, path, kind, category) in [
        (
            "runtime",
            config.execution.path.join(run),
            Kind::Retrospective,
            Category::Hot,
        ),
        (
            "work",
            config.execution.path.join("workspaces/runs").join(run),
            Kind::Recovery,
            Category::Workspace,
        ),
        (
            "snapshot",
            config.execution.path.join("workspaces/archives").join(run),
            Kind::Recovery,
            Category::Workspace,
        ),
        (
            "validation",
            config.execution.path.join(format!("validation-{run}")),
            Kind::Retrospective,
            Category::Hot,
        ),
        (
            "checkout",
            config
                .execution
                .path
                .join("workspaces/runs")
                .join(format!("validation-{run}")),
            Kind::Recovery,
            Category::Workspace,
        ),
    ] {
        register_path(
            tx,
            config,
            Material {
                id: &format!("{run}-{suffix}"),
                run,
                path: &path,
                kind,
                category,
                now,
            },
        )
        .await?;
    }
    caches(tx, config, run, now).await?;
    preparations(tx, config, run, now).await?;
    merge_directories(tx, config, run, now).await?;
    Ok(())
}

async fn merge_directories(
    tx: &mut Tx<'_>,
    config: &Deployment,
    run: &str,
    now: i64,
) -> Result<()> {
    let merges: Vec<(String,Option<String>,Option<String>)> = sqlx::query_as("SELECT m.action_key,m.intent->>'checkout_sha',m.merged_sha FROM merge_operation m JOIN delivery d ON d.action_key=m.delivery_key JOIN candidate_validation v ON v.id=d.validation_id WHERE v.source_run_id=$1")
        .bind(run).fetch_all(&mut **tx).await?;
    for (key, pre, post) in merges {
        for (phase, sha) in [("pre", pre), ("post", post)] {
            let Some(sha) = sha else { continue };
            let evidence = if phase == "pre" {
                format!("pre-merge-{key}-{sha}")
            } else {
                format!("post-merge-{key}")
            };
            for (suffix, path, kind, category) in [
                (
                    "evidence",
                    config.execution.path.join("validations").join(evidence),
                    Kind::Retrospective,
                    Category::Hot,
                ),
                (
                    "checkout",
                    config
                        .execution
                        .path
                        .join("workspaces/runs")
                        .join(format!("merge-{key}-{sha}")),
                    Kind::Recovery,
                    Category::Workspace,
                ),
            ] {
                register_path(
                    tx,
                    config,
                    Material {
                        id: &format!("merge-{key}-{phase}-{suffix}"),
                        run,
                        path: &path,
                        kind,
                        category,
                        now,
                    },
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn preparations(tx: &mut Tx<'_>, config: &Deployment, run: &str, now: i64) -> Result<()> {
    let prefix = format!(".preparation-{run}-");
    for entry in std::fs::read_dir(&config.execution.path)? {
        let entry = entry?;
        let filename = entry.file_name();
        let name = filename.to_str().ok_or("unknown preparation filename")?;
        if !name.starts_with(&prefix) {
            continue;
        }
        preparation_entry(tx, config, run, now, name).await?;
    }
    Ok(())
}

async fn preparation_entry(
    tx: &mut Tx<'_>,
    config: &Deployment,
    run: &str,
    now: i64,
    name: &str,
) -> Result<()> {
    let deleted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM storage_material WHERE id=$1 AND status='deleted')",
    )
    .bind(name)
    .fetch_one(&mut **tx)
    .await?;
    if deleted {
        return Ok(());
    }
    let path = config.execution.path.join(name);
    store::material(
        tx,
        Material {
            id: name,
            run,
            path: &path,
            kind: Kind::Retrospective,
            category: Category::Hot,
            now,
        },
        &config.policy,
    )
    .await?;
    let owner = crate::workspace_files::read(&path.join("storage-owner.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    if owner.as_ref().and_then(|owner| owner["run"].as_str()) != Some(run) {
        sqlx::query(
            "UPDATE storage_material SET protection='unknown preparation identity' WHERE id=$1",
        )
        .bind(name)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn caches(tx: &mut Tx<'_>, config: &Deployment, run: &str, now: i64) -> Result<()> {
    let saved:Option<Value>=sqlx::query_scalar("SELECT identity FROM run_workspace WHERE run_id=$1 UNION ALL SELECT workspace FROM initial_run WHERE launch#>>'{key,run_id}'=$1 LIMIT 1")
        .bind(run).fetch_optional(&mut **tx).await?;
    let Some(saved) = saved else {
        return Ok(());
    };
    let workspace: Workspace = serde_json::from_value(saved)?;
    if !Path::new(&workspace.path).try_exists()? {
        return Ok(());
    }
    register_caches(tx, config, &workspace, now).await
}
async fn register_caches(
    tx: &mut Tx<'_>,
    config: &Deployment,
    workspace: &Workspace,
    now: i64,
) -> Result<()> {
    let run = &workspace.key.run_id;
    let broker = crate::git_broker::GitBroker::open(&config.execution.path.join("workspaces"))?;
    for name in crate::workspace_files::CACHE_ROOTS {
        let path = Path::new(&workspace.path).join(name);
        if path.try_exists()? && broker.cache_rebuildable(workspace, name)? {
            register_path(
                tx,
                config,
                Material {
                    id: &format!("{run}-cache-{name}"),
                    run,
                    path: &path,
                    kind: Kind::Rebuildable,
                    category: Category::Workspace,
                    now,
                },
            )
            .await?;
        }
    }
    Ok(())
}

async fn register_path(tx: &mut Tx<'_>, config: &Deployment, material: Material<'_>) -> Result<()> {
    if !material.path.try_exists()? {
        return Ok(());
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM storage_material WHERE id=$1)")
            .bind(material.id)
            .fetch_one(&mut **tx)
            .await?;
    if !exists {
        store::material(tx, material, &config.policy).await?;
    }
    Ok(())
}

async fn summaries(tx: &mut Tx<'_>, limit: u64) -> Result<()> {
    // Bound long-lived summaries at ingress. Original results are not rewritten.
    let values: Vec<(String,Value)> = sqlx::query_as("SELECT s.run_id,s.summary || jsonb_build_object('run',to_jsonb(a)-'launch','validation',(SELECT to_jsonb(v)-'trusted'-'required_steps' FROM candidate_validation v WHERE v.source_run_id=s.run_id AND v.superseded_by IS NULL ORDER BY v.started_at DESC NULLS LAST,v.id DESC LIMIT 1),'validations',(SELECT COALESCE(jsonb_agg(to_jsonb(v)-'trusted'-'required_steps' ORDER BY v.started_at NULLS FIRST,v.id),'[]'::jsonb) FROM candidate_validation v WHERE v.source_run_id=s.run_id),'preparation',(SELECT jsonb_build_object('retry',retry,'ready',ready) FROM preparation_record WHERE run_id=s.run_id),'resolved_by',s.resolved_by) FROM storage_attempt s LEFT JOIN agent_run a ON a.id=s.run_id")
        .fetch_all(&mut **tx).await?;
    for (run, mut value) in values {
        crate::operator_view::redact(&mut value);
        if serde_json::to_vec(&value)?.len() as u64 > limit {
            return Err(
                "decision record byte limit; preserve originals and export or adjust policy".into(),
            );
        }
        sqlx::query("UPDATE storage_attempt SET summary=summary || $2 WHERE run_id=$1 AND summary<>summary || $2")
            .bind(run)
            .bind(value)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn register_workspace(tx: &mut Tx<'_>, workspace: &Workspace) -> Result<Identity> {
    attempt(
        tx,
        &workspace.key.run_id,
        workspace.requirement,
        workspace.revision,
    )
    .await
}
