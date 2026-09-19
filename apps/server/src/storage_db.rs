//! Retire resolved raw Runtime chunks only after a verified bounded retrospective
//! and replacement reference are durable. Historical usage/results stay intact.
use crate::{
    run_store, storage_archive, storage_consumers,
    storage_files::Directory,
    storage_lifecycle::{Category, Kind},
    storage_measure,
    storage_store::{self as store, Deployment, Result, Tx},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::path::Path;

pub async fn collect(pool: &PgPool, config: &Deployment, now: i64) -> Result<()> {
    let cutoff = now.saturating_sub(config.policy.categories[&Category::Hot].seconds as i64);
    let rows:Vec<(String,String)>=sqlx::query_as("SELECT e.run_id,e.channel FROM runtime_evidence e JOIN agent_run a ON a.id=e.run_id JOIN storage_attempt s ON s.run_id=e.run_id WHERE e.expired_at IS NULL AND a.quiescent AND s.resolved_by IS NOT NULL AND extract(epoch FROM a.created_at)::bigint<=$1")
        .bind(cutoff).fetch_all(pool).await?;
    for (run, channel) in rows {
        retire(pool, config, &run, &channel, now).await?;
    }
    Ok(())
}

async fn retire(
    pool: &PgPool,
    config: &Deployment,
    run: &str,
    channel: &str,
    now: i64,
) -> Result<()> {
    if !eligible(pool, config, run).await? {
        return Ok(());
    }
    let mut retry = retry(pool, run, channel, now).await?;
    save_retry(pool, run, channel, &retry).await?;
    if !retry.begin(now, false) {
        return Ok(());
    }
    save_retry(pool, run, channel, &retry).await?;
    match export(pool, config, run, channel, now).await {
        Ok(()) => {
            retry.success();
            retry.next_attempt_at = Some(now.saturating_add(300));
        }
        Err(error) => retry.fail(
            crate::preparation::Failure {
                code: "cleanup_failed".into(),
                phase: "cleanup".into(),
                detail: crate::operator_view::redact_text(&error.to_string())
                    .chars()
                    .take(1024)
                    .collect(),
                exit_code: None,
                evidence: "runtime_evidence".into(),
            },
            now,
        ),
    }
    save_retry(pool, run, channel, &retry).await
}
async fn eligible(pool: &PgPool, config: &Deployment, run: &str) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    Ok(
        storage_consumers::protection(&mut tx, config, run, "retrospective")
            .await?
            .reason()
            .is_none(),
    )
}
async fn retry(
    pool: &PgPool,
    run: &str,
    channel: &str,
    now: i64,
) -> Result<crate::preparation::Retry> {
    let value: Option<Value> = sqlx::query_scalar(
        "SELECT cleanup_retry FROM runtime_evidence WHERE run_id=$1 AND channel=$2",
    )
    .bind(run)
    .bind(channel)
    .fetch_one(pool)
    .await?;
    let mut retry = value
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_else(|| crate::preparation::Retry::new("cleanup", now));
    if retry.active() {
        retry.fail(
            crate::preparation::Failure {
                code: "cleanup_failed".into(),
                phase: "cleanup".into(),
                detail: "interrupted retrospective export; retain raw chunks".into(),
                exit_code: None,
                evidence: "runtime_evidence".into(),
            },
            now,
        );
    }
    Ok(retry)
}
async fn save_retry(
    pool: &PgPool,
    run: &str,
    channel: &str,
    retry: &crate::preparation::Retry,
) -> Result<()> {
    sqlx::query("UPDATE runtime_evidence SET cleanup_retry=$3 WHERE run_id=$1 AND channel=$2")
        .bind(run)
        .bind(channel)
        .bind(json!(retry))
        .execute(pool)
        .await?;
    Ok(())
}
pub async fn authorize(tx: &mut Tx<'_>, requirement: i64, now: i64) -> Result<()> {
    let rows:Vec<(String,String,Value)>=sqlx::query_as("SELECT e.run_id,e.channel,e.cleanup_retry FROM runtime_evidence e JOIN agent_run a ON a.id=e.run_id WHERE a.requirement_id=$1 AND (e.cleanup_retry->>'todo')::boolean FOR UPDATE OF e")
        .bind(requirement).fetch_all(&mut **tx).await?;
    for (run, channel, value) in rows {
        let mut retry: crate::preparation::Retry = serde_json::from_value(value)?;
        retry.authorize_retry_group(now);
        sqlx::query("UPDATE runtime_evidence SET cleanup_retry=$3 WHERE run_id=$1 AND channel=$2")
            .bind(run)
            .bind(channel)
            .bind(json!(retry))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn export(
    pool: &PgPool,
    config: &Deployment,
    run: &str,
    channel: &str,
    now: i64,
) -> Result<()> {
    let Some((id, bytes)) = reserve(pool, config, run, channel).await? else {
        return Ok(());
    };
    let target = prepare(pool, config, run, &id, now).await?;
    storage_archive::write_record(&target, &bytes)?;
    storage_archive::verify_record(&target, &bytes)?;
    finalize(pool, config, run, channel, &id, &bytes, now).await
}

async fn reserve(
    pool: &PgPool,
    config: &Deployment,
    run: &str,
    channel: &str,
) -> Result<Option<(String, Vec<u8>)>> {
    let mut tx = run_store::lock(pool).await?;
    if storage_consumers::protection(&mut tx, config, run, "retrospective")
        .await?
        .reason()
        .is_some()
    {
        return Ok(None);
    }
    let bytes = record(&mut tx, run, channel, config.policy.entry_bytes).await?;
    let id = format!("runtime-archive-{}", crate::process::new_identity()?);
    if !reserve_bytes(&mut tx, config, run, &id, bytes.len()).await? {
        return Err("retrospective capacity unavailable; retain raw chunks".into());
    }
    tx.commit().await?;
    Ok(Some((id, bytes)))
}

async fn reserve_bytes(
    tx: &mut Tx<'_>,
    config: &Deployment,
    run: &str,
    id: &str,
    size: usize,
) -> Result<bool> {
    let measured = storage_measure::measure(tx, config).await?;
    store::reserve(
        tx,
        id,
        run,
        Category::Cold,
        (size as u64).saturating_add(65536),
        measured.usage(Category::Cold),
        &config.policy,
    )
    .await
}

async fn record(tx: &mut Tx<'_>, run: &str, channel: &str, limit: u64) -> Result<Vec<u8>> {
    let chunks:Vec<(i64,Vec<u8>)>=sqlx::query_as("SELECT sequence::bigint,payload FROM runtime_evidence_chunk WHERE run_id=$1 AND channel=$2 ORDER BY sequence")
        .bind(run).bind(channel).fetch_all(&mut **tx).await?;
    let values: Vec<Value> = chunks
        .iter()
        .enumerate()
        .map(|(index, (sequence, payload))| {
            chunk(*sequence, payload, index < 2 || index + 2 >= chunks.len())
        })
        .collect();
    let identity: Value =
        sqlx::query_scalar("SELECT identity FROM storage_attempt WHERE run_id=$1")
            .bind(run)
            .fetch_one(&mut **tx)
            .await?;
    let bytes = serde_json::to_vec(
        &json!({"identity":identity,"channel":channel,"chunks":values,
        "complete_replay":false,"reason":"bounded retrospective after explicit resolution; raw stream expired"}),
    )?;
    if bytes.len() as u64 > limit {
        return Err("retrospective record byte limit; retain raw chunks".into());
    }
    Ok(bytes)
}
fn chunk(sequence: i64, payload: &[u8], preview: bool) -> Value {
    let kept = if preview { payload.len().min(1024) } else { 0 };
    json!({"sequence":sequence,"bytes":payload.len(),"sha256":format!("{:x}",Sha256::digest(payload)),
        "text":crate::operator_view::redact_text(&String::from_utf8_lossy(&payload[..kept])),
        "kept_range":[0,kept],"truncated":kept<payload.len()})
}

async fn prepare(
    pool: &PgPool,
    config: &Deployment,
    run: &str,
    id: &str,
    now: i64,
) -> Result<Directory> {
    let directory = config.cold.open()?.create_directory(Path::new(id))?;
    let mut tx = run_store::lock(pool).await?;
    store::material(
        &mut tx,
        store::Material {
            id,
            run,
            path: &config.cold.path.join(id),
            kind: Kind::Retrospective,
            category: Category::Cold,
            now,
        },
        &config.policy,
    )
    .await?;
    sqlx::query("UPDATE storage_material SET protection='partial archive; reconcile' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(directory)
}

async fn finalize(
    pool: &PgPool,
    config: &Deployment,
    run: &str,
    channel: &str,
    id: &str,
    bytes: &[u8],
    now: i64,
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    sqlx::query("SELECT run_id FROM runtime_evidence WHERE run_id=$1 AND channel=$2 FOR UPDATE")
        .bind(run)
        .bind(channel)
        .fetch_one(&mut *tx)
        .await?;
    if storage_consumers::protection(&mut tx, config, run, "retrospective")
        .await?
        .reason()
        .is_some()
    {
        return Ok(());
    }
    if record(&mut tx, run, channel, config.policy.entry_bytes).await? != bytes {
        return Err("raw evidence changed during compaction".into());
    }
    save_expiration(&mut tx, config, run, channel, id, bytes, now).await?;
    tx.commit().await?;
    Ok(())
}
async fn save_expiration(
    tx: &mut Tx<'_>,
    config: &Deployment,
    run: &str,
    channel: &str,
    id: &str,
    bytes: &[u8],
    now: i64,
) -> Result<()> {
    let directory = config.cold.open()?.child(Path::new(id))?;
    storage_archive::verify_record(&directory, bytes)?;
    sqlx::query("UPDATE runtime_evidence SET expired_at=$3,replacement=$4,retrospective=$5 WHERE run_id=$1 AND channel=$2")
        .bind(run).bind(channel).bind(now).bind(id).bind(std::str::from_utf8(bytes)?).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM runtime_evidence_chunk WHERE run_id=$1 AND channel=$2")
        .bind(run)
        .bind(channel)
        .execute(&mut **tx)
        .await?;
    settle_export(tx, config, id, &directory).await
}
async fn settle_export(
    tx: &mut Tx<'_>,
    config: &Deployment,
    id: &str,
    directory: &Directory,
) -> Result<()> {
    sqlx::query("UPDATE storage_material SET protection=NULL,manifest=$2 WHERE id=$1")
        .bind(id)
        .bind(json!(directory.inventory(config.policy.entry_count)?))
        .execute(&mut **tx)
        .await?;
    store::reconcile(tx, id, directory.usage(config.policy.entry_count)?, true).await?;
    Ok(())
}
