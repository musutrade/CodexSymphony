//! Reconcile real occupied bytes, including PostgreSQL indexes/WAL and backups.
use crate::{
    storage,
    storage_files::{Directory, FileIdentity},
    storage_lifecycle::{Category, Usage},
    storage_store::{Deployment, Result, Tx},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement {
    pub categories: BTreeMap<Category, u64>,
    pub available: u64,
    pub actual: u64,
    pub classification_todo: u64,
}
impl Measurement {
    pub fn usage(&self, category: Category) -> Usage {
        Usage {
            actual: self.actual,
            category_actual: self.categories[&category],
            available: self.available,
            ..Usage::default()
        }
    }
}

pub async fn measure(tx: &mut Tx<'_>, config: &Deployment) -> Result<Measurement> {
    let (execution, cold, available, extras) = roots(config)?;
    let database: i64 = sqlx::query_scalar("SELECT pg_database_size(current_database()) + COALESCE((SELECT SUM(size) FROM pg_ls_waldir()),0)::bigint")
        .fetch_one(&mut **tx).await?;
    let records: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(pg_total_relation_size(c.oid)),0)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=current_schema() AND c.relkind='r' AND c.relname NOT IN ('runtime_evidence_chunk')")
        .fetch_one(&mut **tx).await?;
    let hot = hot_bytes(tx, config).await?;
    let categories = BTreeMap::from([
        (Category::Workspace, execution.saturating_sub(hot)),
        (Category::Hot, hot),
        (Category::Cold, cold),
        (
            Category::Database,
            (database as u64)
                .saturating_sub(records as u64)
                .checked_add(extras)
                .ok_or("database size overflow")?,
        ),
        (Category::Record, records as u64),
    ]);
    let actual = categories
        .values()
        .try_fold(0u64, |sum, bytes| sum.checked_add(*bytes))
        .ok_or("storage size overflow")?;
    Ok(Measurement {
        categories,
        available,
        actual,
        classification_todo: unclassified(tx, config).await?,
    })
}

fn root_usage(root: &crate::storage_store::Root, limit: u64) -> Result<(u64, u64)> {
    Ok((root.open()?.usage(limit)?, storage::available(&root.path)?))
}
fn roots(config: &Deployment) -> Result<(u64, u64, u64, u64)> {
    let (execution, exec_free) = root_usage(&config.execution, config.policy.entry_count)?;
    let (cold, cold_free) = root_usage(&config.cold, config.policy.entry_count)?;
    config.database_filesystem.open()?;
    let mut available = exec_free
        .min(cold_free)
        .min(storage::available(&config.database_filesystem.path)?);
    let extras = extra_usage(config, &mut available)?;
    Ok((execution, cold, available, extras))
}
fn extra_usage(config: &Deployment, available: &mut u64) -> Result<u64> {
    let mut total = 0u64;
    for root in &config.database_extras {
        let (bytes, free) = root_usage(root, config.policy.entry_count)?;
        total = total.checked_add(bytes).ok_or("database size overflow")?;
        *available = (*available).min(free);
    }
    Ok(total)
}

async fn hot_bytes(tx: &mut Tx<'_>, config: &Deployment) -> Result<u64> {
    let materials: Vec<(String, Value)> = sqlx::query_as("SELECT path,directory_identity FROM storage_material WHERE category='hot' AND status<>'deleted'")
        .fetch_all(&mut **tx).await?;
    let mut bytes = 0u64;
    for (path, identity) in materials {
        let path = Path::new(&path);
        if !path.starts_with(&config.execution.path) {
            return Err("material outside registered root".into());
        }
        let directory = Directory::open(path)?;
        directory.matches(&serde_json::from_value::<FileIdentity>(identity)?)?;
        bytes = bytes
            .checked_add(directory.usage(config.policy.entry_count)?)
            .ok_or("material size overflow")?;
    }
    Ok(bytes)
}

async fn unclassified(tx: &mut Tx<'_>, config: &Deployment) -> Result<u64> {
    let paths: Vec<String> = sqlx::query_scalar("SELECT path FROM storage_material")
        .fetch_all(&mut **tx)
        .await?;
    let mut count = 0;
    for root in [&config.execution, &config.cold] {
        for entry in root.open()?.listing(config.policy.entry_count)? {
            let path = root.path.join(entry.path);
            if !paths
                .iter()
                .any(|known| Path::new(known).starts_with(&path) || path.starts_with(known))
            {
                count += 1;
            }
        }
    }
    Ok(count)
}
