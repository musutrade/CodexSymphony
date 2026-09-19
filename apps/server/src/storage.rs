//! Control-plane storage admission and runtime watchdog. Never delete originals.
use crate::{preparation::RESERVE_BYTES, process};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, io, path::Path, process::Command};

// Covers the gap where PostgreSQL itself cannot persist the stop latch.
// On process restart the existing incarnation/recovery barrier remains closed.
static FAILED: AtomicBool = AtomicBool::new(false);

pub fn available(path: &Path) -> io::Result<u64> {
    let output = Command::new("/usr/bin/stat")
        .args(["-f", "--format=%a %S", "--"])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("storage filesystem unavailable"));
    }
    parse_available(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_available(value: &str) -> io::Result<u64> {
    let mut fields = value.split_whitespace();
    let blocks = number(fields.next())?;
    let size = number(fields.next())?;
    blocks
        .checked_mul(size)
        .ok_or_else(|| io::Error::other("invalid disk capacity"))
}

fn number(value: Option<&str>) -> io::Result<u64> {
    value
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| io::Error::other("invalid disk capacity"))
}

pub fn check(path: &Path) -> io::Result<()> {
    if available(path)? < RESERVE_BYTES {
        return Err(io::Error::from_raw_os_error(28));
    }
    let probe = path.join(format!(".storage-{}", process::new_identity()?));
    process::durable_write(&probe, &"storage write/read/fsync probe")?;
    let _: String = process::read(&probe)?;
    fs::remove_file(&probe)?;
    fs::File::open(path)?.sync_all()
}

/// The database probe includes a committed write; SELECT 1 cannot prove durability.
pub async fn persistence(pool: &PgPool) -> Result<(), sqlx::Error> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("UPDATE storage_guard SET error=error WHERE id=1").execute(pool),
    )
    .await;
    let Ok(result) = result else {
        return Err(sqlx::Error::PoolTimedOut);
    };
    result?;
    Ok(())
}

pub async fn latch(pool: &PgPool) {
    FAILED.store(true, Ordering::SeqCst);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), sqlx::raw_sql("UPDATE storage_guard SET blocked=true,error=COALESCE(error,'storage_unavailable; retain originals and reconcile') WHERE id=1")
        .execute(pool)).await;
}

pub async fn write(pool: &PgPool, path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let result = process::durable_write(path, value);
    if result.is_err() {
        latch(pool).await;
    }
    result
}

/// Recheck before every model continuation or remote write. Unknown is refused.
pub async fn permit(pool: &PgPool, root: &Path) -> bool {
    if FAILED.load(Ordering::SeqCst) {
        return false;
    }
    if check(root).is_err()
        || persistence(pool).await.is_err()
        || !crate::storage_service::capacity(pool)
            .await
            .inspect_err(|error| {
                let diagnostic = crate::operator_view::redact_text(&error.to_string());
                tracing::warn!("storage capacity check failed; originals retained: {diagnostic}")
            })
            .unwrap_or(false)
    {
        latch(pool).await;
        return false;
    }
    sqlx::query_scalar::<_, bool>("SELECT NOT blocked FROM storage_guard WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap_or(false)
}

/// Explicit operator recovery, after stop and preservation reconciliation.
/// Never clears either global or Requirement pause.
pub async fn recover(
    pool: &PgPool,
    root: &Path,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    check(root)?;
    persistence(pool).await?;
    let mut tx = crate::run_store::lock(pool).await?;
    let result = recover_in(&mut tx, root).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn recover_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &Path,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    check(root)?;
    if !crate::storage_service::capacity_in(tx).await? {
        return Ok(false);
    }
    let result = sqlx::query("UPDATE storage_guard SET blocked=false,error=NULL WHERE id=1 AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent)")
        .execute(&mut **tx).await?;
    if result.rows_affected() == 1 {
        FAILED.store(false, Ordering::SeqCst);
    }
    Ok(result.rows_affected() == 1)
}
