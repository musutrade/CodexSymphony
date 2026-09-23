//! Product storage admission/monitoring. No model calls or execution-slot claims.
use crate::{
    run_store, storage_inventory,
    storage_lifecycle::{CATEGORIES, Category},
    storage_measure,
    storage_store::{self as store, Deployment, Result},
    workspace::Workspace,
};
use serde_json::json;
use sqlx::PgPool;
use std::path::Path;

pub async fn configured(pool: &PgPool) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT policy_version IS NOT NULL FROM storage_guard WHERE id=1")
            .fetch_one(pool)
            .await?,
    )
}

pub async fn admit_workspace(pool: &PgPool, workspace: &Workspace) -> Result<bool> {
    if !configured(pool).await? {
        return Ok(true);
    }
    crate::storage_cleanup::scan(pool, crate::runtime_client::now()).await?;
    let mut tx = run_store::lock(pool).await?;
    if !reserve_workspace(&mut tx, workspace).await? {
        tx.rollback().await?;
        block(pool,"storage allocation budget exhausted; retain originals and reclaim/export or adjust policy").await?;
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}

pub async fn entry_limit(pool: &PgPool) -> Result<u64> {
    let limit:Option<i64>=sqlx::query_scalar("SELECT (p.document->>'entry_bytes')::bigint FROM storage_guard g JOIN storage_policy p ON p.version=g.policy_version WHERE g.id=1")
        .fetch_optional(pool).await?;
    Ok(limit.map_or(1_048_576, |value| value as u64).min(1_048_576))
}

pub async fn reserve_workspace(tx: &mut store::Tx<'_>, workspace: &Workspace) -> Result<bool> {
    let Some(config) = store::deployment(tx).await? else {
        return Ok(true);
    };
    storage_inventory::register_workspace(tx, workspace).await?;
    if !ready_for_work(tx).await? {
        return Ok(false);
    }
    reserve_categories(tx, workspace, &config).await
}
async fn reserve_categories(
    tx: &mut store::Tx<'_>,
    workspace: &Workspace,
    config: &Deployment,
) -> Result<bool> {
    let measured = storage_measure::measure(tx, config).await?;
    for category in CATEGORIES.into_iter().filter(|category| {
        *category != Category::Cold
            && (workspace.phase != "integration" || *category == Category::Workspace)
    }) {
        let amount = config.policy.categories[&category].reserve_bytes;
        if !store::reserve(
            tx,
            &format!("{}-{}", workspace.key.run_id, store::name(&category)?),
            &workspace.key.run_id,
            category,
            amount,
            measured.usage(category),
            &config.policy,
        )
        .await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
/// Integration checkouts do not spawn Runtime producers. Their supervisor has
/// one separate hot allocation, reserved before its directory can be discovered.
pub async fn reserve_integration(tx: &mut store::Tx<'_>, run: &str) -> Result<bool> {
    let Some(config) = store::deployment(tx).await? else {
        return Ok(true);
    };
    if !ready_for_work(tx).await? {
        return Ok(false);
    }
    let measured = storage_measure::measure(tx, &config).await?;
    store::reserve(
        tx,
        &format!("{run}-hot"),
        run,
        Category::Hot,
        config.policy.categories[&Category::Hot].reserve_bytes,
        measured.usage(Category::Hot),
        &config.policy,
    )
    .await
}

pub async fn validation(pool: &PgPool, run: &str) -> Result<bool> {
    if !configured(pool).await? {
        return Ok(true);
    }
    let mut tx = run_store::lock(pool).await?;
    let (config, measured) = validation_usage(&mut tx).await?;
    if !reserve_validation(&mut tx, &config, run, &measured).await? {
        tx.rollback().await?;
        tracing::warn!(run, "validation storage reservation unavailable");
        block(
            pool,
            "validation storage reservation unavailable; preserve candidate",
        )
        .await?;
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}

async fn validation_usage(
    tx: &mut store::Tx<'_>,
) -> Result<(Deployment, storage_measure::Measurement)> {
    let config = store::deployment(tx)
        .await?
        .ok_or("storage deployment missing")?;
    storage_inventory::discover(tx, &config, crate::runtime_client::now()).await?;
    let measured = storage_measure::measure(tx, &config).await?;
    Ok((config, measured))
}
async fn reserve_validation(
    tx: &mut store::Tx<'_>,
    config: &Deployment,
    run: &str,
    measured: &storage_measure::Measurement,
) -> Result<bool> {
    let integration: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM integration_validation WHERE id=$1)")
            .bind(run)
            .fetch_one(&mut **tx)
            .await?;
    if integration {
        return reserve_integration(tx, run).await;
    }
    if !ready_for_work(tx).await? {
        return Ok(false);
    }
    for category in [Category::Workspace, Category::Hot] {
        if !store::reserve(
            tx,
            &format!("validation-{run}-{}", store::name(&category)?),
            run,
            category,
            config.policy.categories[&category].reserve_bytes,
            measured.usage(category),
            &config.policy,
        )
        .await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn ready_for_work(tx: &mut store::Tx<'_>) -> Result<bool> {
    let blocked: bool = sqlx::query_scalar("SELECT blocked FROM storage_guard WHERE id=1")
        .fetch_one(&mut **tx)
        .await?;
    Ok(!blocked && capacity_in(tx).await?)
}

pub async fn capacity(pool: &PgPool) -> Result<bool> {
    if !configured(pool).await? {
        return Ok(true);
    }
    let mut tx = run_store::lock(pool).await?;
    let fits = capacity_in(&mut tx).await?;
    tx.commit().await?;
    Ok(fits)
}
pub async fn capacity_in(tx: &mut store::Tx<'_>) -> Result<bool> {
    let Some(config) = store::deployment(tx).await? else {
        return Ok(true);
    };
    let measured = reconciled_usage(tx, &config).await?;
    let outstanding: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(outstanding),0)::bigint FROM storage_allocation")
            .fetch_one(&mut **tx)
            .await?;
    let mut fits = measured
        .actual
        .checked_add(outstanding as u64)
        .and_then(|total| total.checked_add(config.policy.control_bytes))
        .is_some_and(|total| total <= config.policy.global_bytes);
    fits &= (outstanding as u64)
        .checked_add(config.policy.control_bytes)
        .is_some_and(|total| total <= measured.available);
    fits &= categories_fit(tx, &config, &measured).await?;
    fits &= records_current(tx, crate::runtime_client::now()).await?;
    fits &= cumulative_fit(tx, &config).await?;
    sqlx::query("UPDATE storage_guard SET measured=$1,measured_at=$2 WHERE id=1")
        .bind(json!(measured))
        .bind(crate::runtime_client::now())
        .execute(&mut **tx)
        .await?;
    Ok(fits)
}

async fn reconciled_usage(
    tx: &mut store::Tx<'_>,
    config: &Deployment,
) -> Result<storage_measure::Measurement> {
    storage_inventory::discover(tx, config, crate::runtime_client::now()).await?;
    crate::storage_cleanup::reconcile(tx, config).await?;
    let measured = storage_measure::measure(tx, config).await?;
    Ok(measured)
}
async fn categories_fit(
    tx: &mut store::Tx<'_>,
    config: &Deployment,
    measured: &storage_measure::Measurement,
) -> Result<bool> {
    let mut fits = true;
    for category in CATEGORIES {
        let reserved: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(outstanding),0)::bigint FROM storage_allocation WHERE category=$1",
        )
        .bind(store::name(&category)?)
        .fetch_one(&mut **tx)
        .await?;
        fits &= measured.categories[&category]
            .checked_add(reserved as u64)
            .is_some_and(|total| total <= config.policy.categories[&category].bytes);
    }
    Ok(fits)
}

async fn cumulative_fit(tx: &mut store::Tx<'_>, config: &Deployment) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM storage_allocation GROUP BY run_id HAVING SUM(allocated)>$1) AND NOT EXISTS(SELECT 1 FROM storage_allocation l JOIN storage_attempt a ON a.run_id=l.run_id GROUP BY a.requirement_id HAVING SUM(l.allocated)>$2)")
        .bind(config.policy.run_bytes as i64).bind(config.policy.requirement_bytes as i64).fetch_one(&mut **tx).await?)
}
async fn records_current(tx: &mut store::Tx<'_>, now: i64) -> Result<bool> {
    let overdue: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM storage_attempt WHERE expires_at<=$1)")
            .bind(now)
            .fetch_one(&mut **tx)
            .await?;
    if overdue {
        sqlx::query("UPDATE storage_guard SET blocked=true,error='decision record retention expired; export or explicitly extend storage policy; originals retained' WHERE id=1")
            .execute(&mut **tx).await?;
    }
    Ok(!overdue)
}

pub async fn block(pool: &PgPool, reason: &str) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    sqlx::query("UPDATE storage_guard SET blocked=true,error=$1 WHERE id=1")
        .bind(reason)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE agent_run SET stop_requested=true WHERE NOT quiescent")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn preparation(pool: &PgPool, workspace: &Workspace, directory: &Path) -> Result<()> {
    if !configured(pool).await? {
        return Ok(());
    }
    let mut tx = run_store::lock(pool).await?;
    let config = store::deployment(&mut tx)
        .await?
        .ok_or("storage deployment missing")?;
    storage_inventory::register_workspace(&mut tx, workspace).await?;
    let id = preparation_marker(workspace, directory)?;
    store::material(
        &mut tx,
        store::Material {
            id,
            run: &workspace.key.run_id,
            path: directory,
            kind: crate::storage_lifecycle::Kind::Retrospective,
            category: Category::Hot,
            now: crate::runtime_client::now(),
        },
        &config.policy,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn preparation_marker<'a>(workspace: &Workspace, directory: &'a Path) -> Result<&'a str> {
    crate::process::durable_write(
        &directory.join("storage-owner.json"),
        &json!({"run":workspace.key.run_id}),
    )?;
    let id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("preparation identity required")?;
    Ok(id)
}

pub async fn start(pool: &PgPool, root: &Path) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let Some(config) = load_deployment(root)? else {
        return Ok(None);
    };
    store::install(pool, &config).await?;
    let mut listener = sqlx::postgres::PgListener::connect_with(pool).await?;
    listener.listen("storage_phase_ended").await?;
    let pool = pool.clone();
    Ok(Some(tokio::spawn(worker(pool, listener))))
}
fn load_deployment(root: &Path) -> Result<Option<Deployment>> {
    let path = std::env::var_os("STORAGE_CONFIG");
    let Some(path) = path else {
        if std::env::var_os("RUNTIME_CONFIG").is_some() {
            return Err("STORAGE_CONFIG required for Runtime deployment".into());
        }
        return Ok(None);
    };
    let config: Deployment = serde_json::from_slice(&std::fs::read(path)?)?;
    if std::fs::canonicalize(root)? != config.execution.path {
        return Err("storage execution root mismatch".into());
    }
    Ok(Some(config))
}
async fn worker(pool: PgPool, mut listener: sqlx::postgres::PgListener) {
    loop {
        if let Err(error) = crate::storage_cleanup::scan(&pool, crate::runtime_client::now()).await
        {
            let detail: String = crate::operator_view::redact_text(&error.to_string())
                .chars()
                .take(1024)
                .collect();
            tracing::warn!("storage scan failed; originals retained: {}", detail);
            let _ = block(
                &pool,
                &format!(
                    "storage scan failed; preserve registered originals and reconcile: {detail}"
                ),
            )
            .await;
        }
        let delay = next_scan_delay(&pool, crate::runtime_client::now())
            .await
            .unwrap_or(std::time::Duration::from_secs(30));
        if let Ok(Err(_)) = tokio::time::timeout(delay, listener.recv()).await {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    }
}

/// Honor the persisted infrastructure retry deadline even without a phase event.
/// Healthy/exhausted scans keep the normal five-minute maintenance interval.
pub async fn next_scan_delay(pool: &PgPool, now: i64) -> Result<std::time::Duration> {
    let seconds: i64 = sqlx::query_scalar("SELECT CASE WHEN scan_retry->'last_failure' <> 'null'::jsonb THEN LEAST(300,GREATEST(1,COALESCE((scan_retry->>'next_attempt_at')::bigint-$1,300))) ELSE 300 END::bigint FROM storage_guard WHERE id=1")
        .bind(now).fetch_one(pool).await?;
    Ok(std::time::Duration::from_secs(seconds as u64))
}
