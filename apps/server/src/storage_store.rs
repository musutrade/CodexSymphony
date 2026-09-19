//! Storage ledger under the existing consumer/admission transaction lock.
use crate::{
    run_store,
    storage_files::{Directory, FileIdentity},
    storage_lifecycle::{self as domain, Category, Identity, Kind, Policy, Usage},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::path::PathBuf;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub path: PathBuf,
    pub identity: FileIdentity,
}
impl Root {
    pub fn open(&self) -> Result<Directory> {
        if std::fs::canonicalize(&self.path)? != self.path {
            return Err("canonical storage root required".into());
        }
        let directory = Directory::open(&self.path)?;
        directory.matches(&self.identity)?;
        Ok(directory)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub policy: Policy,
    pub execution: Root,
    pub cold: Root,
    pub database_filesystem: Root,
    /// Existing backup/log directories outside PostgreSQL's own database/WAL.
    pub database_extras: Vec<Root>,
}

pub async fn install(pool: &PgPool, deployment: &Deployment) -> Result<()> {
    validate_deployment(deployment)?;
    let mut tx = run_store::lock(pool).await?;
    let version = &deployment.policy.version;
    let document = json!(deployment.policy);
    let config = json!(deployment);
    sqlx::query("INSERT INTO storage_policy(version,document,deployment) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(version).bind(&document).bind(&config).execute(&mut *tx).await?;
    let saved: Value = sqlx::query_scalar("SELECT deployment FROM storage_policy WHERE version=$1")
        .bind(version)
        .fetch_one(&mut *tx)
        .await?;
    if saved != config {
        return Err("storage policy version reused with different configuration".into());
    }
    sqlx::query("UPDATE storage_guard SET policy_version=$1 WHERE id=1")
        .bind(version)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE storage_attempt SET expires_at=created_at+$1 WHERE expires_at<created_at+$1",
    )
    .bind(deployment.policy.categories[&Category::Record].seconds as i64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn validate_deployment(deployment: &Deployment) -> Result<()> {
    deployment.policy.validate()?;
    for root in [
        &deployment.execution,
        &deployment.cold,
        &deployment.database_filesystem,
    ]
    .into_iter()
    .chain(deployment.database_extras.iter())
    {
        root.open()?;
    }
    if deployment.cold.path.starts_with(&deployment.execution.path)
        || deployment.execution.path.starts_with(&deployment.cold.path)
    {
        return Err("execution and cold roots must be disjoint".into());
    }
    Ok(())
}

pub async fn deployment(tx: &mut Tx<'_>) -> Result<Option<Deployment>> {
    let value: Option<Value> = sqlx::query_scalar("SELECT p.deployment FROM storage_policy p JOIN storage_guard g ON g.policy_version=p.version WHERE g.id=1")
        .fetch_optional(&mut **tx).await?;
    Ok(value.map(serde_json::from_value).transpose()?)
}

pub async fn register(tx: &mut Tx<'_>, identity: &Identity, summary: &Value) -> Result<()> {
    sqlx::query("INSERT INTO storage_attempt(run_id,requirement_id,revision,identity,summary,expires_at) VALUES($1,$2,$3,$4,$5,extract(epoch FROM now())::bigint+(SELECT (p.document#>>'{categories,record,seconds}')::bigint FROM storage_guard g JOIN storage_policy p ON p.version=g.policy_version WHERE g.id=1)) ON CONFLICT DO NOTHING")
        .bind(&identity.run).bind(identity.requirement).bind(identity.revision)
        .bind(json!(identity)).bind(summary).execute(&mut **tx).await?;
    let saved: Value = sqlx::query_scalar("SELECT identity FROM storage_attempt WHERE run_id=$1")
        .bind(&identity.run)
        .fetch_one(&mut **tx)
        .await?;
    if saved != json!(identity) {
        return Err("storage attempt identity conflict".into());
    }
    Ok(())
}

pub fn name(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_value(value)?
        .as_str()
        .ok_or("storage category required")?
        .into())
}

pub struct Material<'a> {
    pub id: &'a str,
    pub run: &'a str,
    pub path: &'a std::path::Path,
    pub kind: Kind,
    pub category: Category,
    pub now: i64,
}
pub async fn material(tx: &mut Tx<'_>, item: Material<'_>, policy: &Policy) -> Result<()> {
    let (identity, path, expiry) = material_identity(&item, policy)?;
    let retry = crate::preparation::Retry::new("cleanup", item.now);
    let allocation = allocation_name(&item)?;
    sqlx::query("INSERT INTO storage_material(id,run_id,path,directory_identity,kind,category,policy_version,created_at,expires_at,retry,allocation_request) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING")
        .bind(item.id).bind(item.run).bind(&path).bind(&identity).bind(name(&item.kind)?)
        .bind(name(&item.category)?).bind(&policy.version).bind(item.now).bind(expiry)
        .bind(json!(retry)).bind(allocation).execute(&mut **tx).await?;
    let saved: (String, String, Value) =
        sqlx::query_as("SELECT run_id,path,directory_identity FROM storage_material WHERE id=$1")
            .bind(item.id)
            .fetch_one(&mut **tx)
            .await?;
    if saved != (item.run.into(), path, identity) {
        return Err("material instance changed".into());
    }
    Ok(())
}

fn material_identity(item: &Material<'_>, policy: &Policy) -> Result<(Value, String, i64)> {
    let directory = Directory::open(item.path)?;
    let identity = json!(directory.identity()?);
    let path = item.path.to_str().ok_or("UTF-8 material path required")?;
    let expiry = item
        .now
        .checked_add(policy.categories[&item.category].seconds as i64)
        .ok_or("storage retention overflow")?;
    Ok((identity, path.into(), expiry))
}

fn allocation_name(item: &Material<'_>) -> Result<String> {
    if item.category == Category::Cold {
        return Ok(item.id.into());
    }
    let prefix = if item.id == format!("{}-validation", item.run)
        || item.id == format!("{}-checkout", item.run)
    {
        format!("validation-{}", item.run)
    } else {
        item.run.into()
    };
    Ok(format!("{prefix}-{}", name(&item.category)?))
}

pub async fn reserve(
    tx: &mut Tx<'_>,
    request: &str,
    run: &str,
    category: Category,
    bytes: u64,
    mut usage: Usage,
    policy: &Policy,
) -> Result<bool> {
    let category = name(&category)?;
    let existing: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT run_id,category,requested FROM storage_allocation WHERE request_id=$1",
    )
    .bind(request)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(existing) = existing {
        return Ok(existing == (run.into(), category, bytes as i64));
    }
    let totals: (i64,i64,i64,i64) = sqlx::query_as("SELECT COALESCE(SUM(outstanding),0)::bigint,COALESCE(SUM(outstanding) FILTER(WHERE category=$2),0)::bigint,COALESCE(SUM(allocated) FILTER(WHERE run_id=$1),0)::bigint,COALESCE(SUM(allocated) FILTER(WHERE run_id IN(SELECT run_id FROM storage_attempt WHERE requirement_id=(SELECT requirement_id FROM storage_attempt WHERE run_id=$1))),0)::bigint FROM storage_allocation")
        .bind(run).bind(&category).fetch_one(&mut **tx).await?;
    (
        usage.reserved,
        usage.category_reserved,
        usage.run_allocated,
        usage.requirement_allocated,
    ) = (
        totals.0 as u64,
        totals.1 as u64,
        totals.2 as u64,
        totals.3 as u64,
    );
    let kind = serde_json::from_value(json!(category))?;
    if !domain::admit(policy, &usage, kind, bytes) {
        return Ok(false);
    }
    sqlx::query("INSERT INTO storage_allocation(request_id,run_id,category,policy_version,requested,allocated,outstanding) VALUES($1,$2,$3,$4,$5,$5,$5)")
        .bind(request).bind(run).bind(category).bind(&policy.version).bind(bytes as i64)
        .execute(&mut **tx).await?;
    Ok(true)
}

/// Settlement never refunds cumulative allocations, including across revisions.
/// Unknown or interrupted producers retain the outstanding reservation.
pub async fn reconcile(
    tx: &mut Tx<'_>,
    request: &str,
    actual: u64,
    complete: bool,
) -> Result<bool> {
    let (allocated, settled, previous): (i64, bool, i64) = sqlx::query_as(
        "SELECT allocated,settled,actual FROM storage_allocation WHERE request_id=$1 FOR UPDATE",
    )
    .bind(request)
    .fetch_one(&mut **tx)
    .await?;
    if settled {
        return Ok(complete && previous as u64 == actual);
    }
    let outstanding = if complete {
        0
    } else {
        (allocated as u64).saturating_sub(actual)
    };
    sqlx::query(
        "UPDATE storage_allocation SET actual=$2,outstanding=$3,settled=$4,allocated=GREATEST(allocated,$2) WHERE request_id=$1",
    )
    .bind(request)
    .bind(i64::try_from(actual)?)
    .bind(outstanding as i64)
    .bind(complete)
    .execute(&mut **tx)
    .await?;
    Ok(actual <= allocated as u64)
}
