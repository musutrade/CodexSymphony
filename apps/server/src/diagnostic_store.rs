//! One ledger for user and Agent reads. The existing execution lock serializes
//! authorization, content admission, revocation and expiration.
use crate::{
    diagnostics::{Artifact, Availability, Captured, Page, Result},
    execution::RunKey,
};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
type Tx<'a> = Transaction<'a, Postgres>;

pub struct Limits {
    pub file: u64,
    pub call: u64,
    pub task: u64,
    pub count: i64,
    pub expires_at: i64,
}

pub async fn limits(pool: &PgPool) -> Result<Limits> {
    let policy: Option<serde_json::Value> = sqlx::query_scalar("SELECT p.document FROM storage_guard g JOIN storage_policy p ON p.version=g.policy_version WHERE g.id=1").fetch_optional(pool).await?;
    if let Some(policy) = policy {
        let policy: crate::storage_lifecycle::Policy = serde_json::from_value(policy)?;
        policy.validate()?;
        return Ok(Limits {
            file: policy.entry_bytes.min(crate::diagnostics::MAX_FILE),
            call: policy.run_bytes.min(16 * crate::diagnostics::MAX_FILE),
            task: policy.requirement_bytes,
            count: policy.entry_count.min(4096) as i64,
            expires_at: crate::runtime_client::now().saturating_add(
                policy.categories[&crate::storage_lifecycle::Category::Hot].seconds as i64,
            ),
        });
    }
    Ok(Limits {
        file: crate::diagnostics::MAX_FILE,
        call: 8 * crate::diagnostics::MAX_FILE,
        task: 32 * crate::diagnostics::MAX_FILE,
        count: 256,
        expires_at: crate::runtime_client::now() + 86400,
    })
}

pub async fn persist(
    pool: &PgPool,
    source: &str,
    captures: Vec<Captured>,
    limits: &Limits,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let size = total_bytes(&captures);
    let requirement = captures.first().map(captured_requirement);
    let capacity = capacity(&mut tx, source, requirement, size).await?;
    for mut captured in captures {
        if !capacity {
            discard(&mut captured);
        }
        persist_one(&mut tx, source, &mut captured, limits).await?;
    }
    tx.commit().await?;
    Ok(())
}

fn captured_requirement(captured: &Captured) -> i64 {
    captured.artifact.binding.identity.requirement_id
}

fn total_bytes(captures: &[Captured]) -> u64 {
    let mut total = 0u64;
    for captured in captures {
        total = total
            .saturating_add(captured.artifact.retained_bytes)
            .saturating_add(captured.artifact.export_bytes)
            .saturating_add(4096);
    }
    total
}

async fn capacity(
    tx: &mut Tx<'_>,
    source: &str,
    requirement: Option<i64>,
    size: u64,
) -> Result<bool> {
    let Some(config) = crate::storage_store::deployment(tx).await? else {
        return Ok(true);
    };
    let measured = crate::storage_measure::measure(tx, &config).await?;
    let mut usage = measured.usage(crate::storage_lifecycle::Category::Record);
    let totals: (i64,i64,i64,i64) = sqlx::query_as("SELECT COALESCE(sum(outstanding),0)::bigint,COALESCE(sum(outstanding) FILTER(WHERE category='record'),0)::bigint,(COALESCE(sum(allocated) FILTER(WHERE run_id=$1),0)+COALESCE((SELECT sum(allocated_bytes) FROM diagnostic_artifact WHERE source_run=$1),0))::bigint,(COALESCE(sum(allocated) FILTER(WHERE run_id IN(SELECT run_id FROM storage_attempt WHERE requirement_id=$2)),0)+COALESCE((SELECT sum(allocated_bytes) FROM diagnostic_artifact WHERE requirement_id=$2),0))::bigint FROM storage_allocation").bind(source).bind(requirement).fetch_one(&mut **tx).await?;
    usage.reserved = totals.0 as u64;
    usage.category_reserved = totals.1 as u64;
    usage.run_allocated = totals.2 as u64;
    usage.requirement_allocated = totals.3 as u64;
    Ok(crate::storage_lifecycle::admit(
        &config.policy,
        &usage,
        crate::storage_lifecycle::Category::Record,
        size,
    ))
}

async fn persist_one(
    tx: &mut Tx<'_>,
    source: &str,
    captured: &mut Captured,
    limits: &Limits,
) -> Result<()> {
    let artifact = &captured.artifact;
    if prior(tx, artifact).await? {
        return Ok(());
    }
    let size = artifact
        .retained_bytes
        .saturating_add(artifact.export_bytes);
    let (task, call, count): (i64, i64, i64) = sqlx::query_as("SELECT COALESCE(sum(allocated_bytes),0)::bigint,COALESCE(sum(allocated_bytes) FILTER (WHERE invocation_id=$2 AND attempt=$3),0)::bigint,count(*)::bigint FROM diagnostic_artifact WHERE requirement_id=$1")
        .bind(artifact.binding.identity.requirement_id).bind(&artifact.binding.identity.invocation_id).bind(artifact.binding.identity.attempt as i32).fetch_one(&mut **tx).await?;
    if count >= limits.count {
        return Err("diagnostic manifest count quota exhausted; retain original files".into());
    }
    if (task as u64).saturating_add(size) > limits.task
        || (call as u64).saturating_add(size) > limits.call
    {
        discard(captured);
    }
    insert(tx, source, captured).await
}

async fn prior(tx: &mut Tx<'_>, artifact: &Artifact) -> Result<bool> {
    let saved: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT manifest FROM diagnostic_artifact WHERE artifact_id=$1")
            .bind(&artifact.artifact_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(saved) = saved {
        let saved: Artifact = serde_json::from_value(saved)?;
        if saved.binding != artifact.binding || saved.purpose != artifact.purpose {
            return Err("diagnostic identity changed".into());
        }
        return Ok(true); // Keep first captured bytes/status, including missing/expired.
    }
    Ok(false)
}

fn discard(captured: &mut Captured) {
    captured.artifact.availability = Availability::Missing;
    captured.artifact.reason = Some(
        "diagnostic storage quota exhausted; content not retained in diagnostic storage".into(),
    );
    captured.artifact.retained_bytes = 0;
    captured.artifact.export_bytes = 0;
    captured.artifact.raw_sha256 = None;
    captured.artifact.export_sha256 = None;
    captured.raw = None;
    captured.export = None;
}

async fn insert(tx: &mut Tx<'_>, source: &str, captured: &Captured) -> Result<()> {
    let a = &captured.artifact;
    sqlx::query("INSERT INTO diagnostic_artifact(artifact_id,requirement_id,revision,source_run,invocation_id,attempt,manifest,raw_payload,export_payload,allocated_bytes,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(&a.artifact_id).bind(a.binding.identity.requirement_id).bind(a.binding.identity.revision).bind(source).bind(&a.binding.identity.invocation_id).bind(a.binding.identity.attempt as i32).bind(json!(a)).bind(&captured.raw).bind(&captured.export).bind(a.retained_bytes.saturating_add(a.export_bytes) as i64).bind(a.expires_at).execute(&mut **tx).await?;
    Ok(())
}

pub async fn user_allowed(tx: &mut Tx<'_>, requirement: i64) -> Result<()> {
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=COALESCE((v.document->>'repository_id')::bigint,r.repository_id) WHERE r.id=$1 AND NOT r.cancel_requested AND NOT COALESCE((p.document->>'revoked')::boolean,false) AND p.revoked_through_version<COALESCE((v.document->>'repository_version')::bigint,p.version))")
        .bind(requirement).fetch_one(&mut **tx).await?;
    if !allowed {
        return Err("diagnostic task authorization unavailable".into());
    }
    Ok(())
}

pub async fn agent_allowed(tx: &mut Tx<'_>, key: &RunKey) -> Result<i64> {
    if !crate::runtime_store::allowed(tx, key).await? {
        return Err("diagnostic Run unavailable".into());
    }
    let requirement = sqlx::query_scalar("SELECT requirement_id FROM agent_run WHERE id=$1")
        .bind(&key.run_id)
        .fetch_one(&mut **tx)
        .await?;
    user_allowed(tx, requirement).await?;
    Ok(requirement)
}

pub async fn list_in(
    tx: &mut Tx<'_>,
    requirement: i64,
    source: Option<&str>,
    after: i64,
) -> Result<Page> {
    if after < 0 {
        return Err("invalid diagnostic cursor".into());
    }
    expire_in(tx, crate::runtime_client::now()).await?;
    let rows: Vec<(i64,serde_json::Value)> = sqlx::query_as("SELECT d.sequence,d.manifest FROM diagnostic_artifact d WHERE d.requirement_id=$1 AND d.sequence>$2 AND diagnostic_revision_allows(d.requirement_id,d.revision) AND diagnostic_integration_allows(d.requirement_id,d.revision,d.source_run,d.invocation_id,d.manifest#>>'{binding,phase}') AND ($3::text IS NULL OR diagnostic_run_allows($3,d.source_run,d.invocation_id)) ORDER BY d.sequence LIMIT 9")
        .bind(requirement).bind(after).bind(source).fetch_all(&mut **tx).await?;
    let mut artifacts = Vec::new();
    let mut next = None;
    let more = rows.len() > 8;
    for (sequence, manifest) in rows.into_iter().take(8) {
        artifacts.push(serde_json::from_value(manifest)?);
        if more {
            next = Some(sequence);
        }
    }
    Ok(Page { artifacts, next })
}

pub async fn load_in(
    tx: &mut Tx<'_>,
    requirement: i64,
    source: Option<&str>,
    id: &str,
) -> Result<(Artifact, Vec<u8>)> {
    expire_in(tx, crate::runtime_client::now()).await?;
    let row: Option<(serde_json::Value,Option<Vec<u8>>)> = sqlx::query_as("SELECT manifest,export_payload FROM diagnostic_artifact d WHERE d.requirement_id=$1 AND d.artifact_id=$2 AND diagnostic_revision_allows(d.requirement_id,d.revision) AND diagnostic_integration_allows(d.requirement_id,d.revision,d.source_run,d.invocation_id,d.manifest#>>'{binding,phase}') AND ($3::text IS NULL OR diagnostic_run_allows($3,d.source_run,d.invocation_id))")
        .bind(requirement).bind(id).bind(source).fetch_optional(&mut **tx).await?;
    let (manifest, payload) = row.ok_or("diagnostic artifact unavailable or unauthorized")?;
    let mut artifact: Artifact = serde_json::from_value(manifest)?;
    let bytes = payload.ok_or("diagnostic content missing or expired; inspect manifest")?;
    if artifact.export_sha256.as_deref() != Some(&crate::validation::sha256(&bytes)) {
        artifact.availability = Availability::Corrupt;
        artifact.reason = Some("export digest differs; content denied".into());
        sqlx::query("UPDATE diagnostic_artifact SET manifest=$2 WHERE artifact_id=$1")
            .bind(id)
            .bind(json!(artifact))
            .execute(&mut **tx)
            .await?;
        // Caller commits the failure state; never return tampered content.
        return Ok((artifact, Vec::new()));
    }
    Ok((artifact, bytes))
}

pub async fn expire_in(tx: &mut Tx<'_>, now: i64) -> Result<()> {
    let consumer = crate::storage_consumers::consumer_predicate("d.requirement_id", "d.source_run");
    let unreconciled = crate::storage_consumers::unreconciled_predicate("d.source_run");
    let query = format!(
        "UPDATE diagnostic_artifact d SET raw_payload=NULL,export_payload=NULL,retired_at=$1,manifest=jsonb_set(jsonb_set(manifest,'{{availability}}','\"expired\"'::jsonb),'{{reason}}','\"retention expired; content deleted\"'::jsonb)
         WHERE d.expires_at<=$1 AND d.retired_at IS NULL
           AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=d.requirement_id AND NOT a.quiescent)
           AND NOT EXISTS(SELECT 1 FROM integration_validation v WHERE v.requirement_id=d.requirement_id AND NOT v.quiescent)
           AND NOT ({consumer}) AND NOT ({unreconciled})
           AND NOT EXISTS(SELECT 1 FROM project_hook_invocation h JOIN project_hook_run r ON r.run_id=h.run_id WHERE r.requirement_id=d.requirement_id AND h.status IN ('intent','running','unknown') AND (NOT h.stop_confirmed OR h.event IN ('before_deliver','before_publish','before_merge')))"
    );
    sqlx::query(&query).bind(now).execute(&mut **tx).await?;
    Ok(())
}
