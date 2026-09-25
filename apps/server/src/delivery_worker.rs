//! One sequential sender; HTTP never waits for GitHub. Replayed jobs observe
//! remote facts before taking another bounded write attempt.
use crate::{
    delivery::PrFact,
    delivery_store::{self as store, Pending},
    github_http::{Error, invalid},
};
use serde_json::Value;
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[allow(async_fn_in_trait)]
pub trait Remote {
    async fn capability_check(&mut self, job: &Pending) -> std::result::Result<Value, Error>;
    async fn find(&mut self, job: &Pending) -> std::result::Result<Option<Value>, Error>;
    async fn head(&mut self, job: &Pending) -> std::result::Result<Option<String>, Error>;
    async fn push(&mut self, job: &Pending) -> std::result::Result<Value, Error>;
    async fn create(&mut self, job: &Pending) -> std::result::Result<Value, Error>;
    async fn close(&mut self, job: &Pending, number: u64) -> std::result::Result<Value, Error>;
}
pub async fn tick(pool: &PgPool, root: &Path, remote: &mut impl Remote, now: i64) -> Result<()> {
    for job in store::due(pool, now).await? {
        reconcile(pool, root, remote, &job, now).await?;
    }
    Ok(())
}
async fn reconcile(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
) -> Result<()> {
    store::observed(pool, job, now).await?;
    let found = match crate::github_publication_adapter::observe(pool, remote, job, now).await {
        Ok(pr) => pr,
        Err(error) => {
            let error = error.downcast_ref::<Error>().ok_or(error.to_string())?;
            store::failed(pool, job, now, error).await?;
            return Ok(());
        }
    };
    if let Some(pr) = found {
        return reconcile_pr(pool, root, remote, job, now, pr).await;
    }
    missing_pr(pool, root, remote, job, now).await
}
async fn missing_pr(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
) -> Result<()> {
    if job.pr_number.is_some() || job.kind == "close" {
        store::failed(pool, job, now, &invalid()).await?;
        return Ok(());
    }
    let head = match remote.head(job).await {
        Ok(head) => head,
        Err(error) => {
            store::failed(pool, job, now, &error).await?;
            return Ok(());
        }
    };
    reconcile_head(pool, root, remote, job, now, head).await
}
async fn reconcile_head(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
    head: Option<String>,
) -> Result<()> {
    if store::withdraw(pool, job, head.as_deref()).await? {
        return Ok(());
    }
    let operation = match head {
        Some(head) if head == job.head_sha => "create",
        Some(head) if job.manifest["workspace"]["baseline"] != head => {
            store::failed(pool, job, now, &invalid()).await?;
            return Ok(());
        }
        _ => "push",
    };
    publish(pool, root, remote, job, now, operation).await
}
async fn publish(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
    operation: &str,
) -> Result<()> {
    crate::github_publication_adapter::submit(pool, root, remote, job, now, operation).await
}

async fn reconcile_pr(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
    pr: Value,
) -> Result<()> {
    let fact = job.fact(&pr);
    if previous_head_open(job, &pr) {
        return publish(pool, root, remote, job, now, "push").await;
    }
    if matches!(fact, PrFact::Conflict | PrFact::Unknown) {
        store::failed(pool, job, now, &invalid()).await?;
        return Ok(());
    }
    store::confirmed(pool, job, &pr).await?;
    if job.kind != "close" || fact != PrFact::Open {
        return Ok(());
    }
    close(pool, root, remote, job, now, pr).await
}
fn previous_head_open(job: &Pending, pr: &Value) -> bool {
    if job.kind != "publish" {
        return false;
    }
    let Some(expected) = &job.expected_head else {
        return false;
    };
    if pr["head"]["sha"] != *expected {
        return false;
    }
    let mut previous = job.clone();
    previous.head_sha = expected.clone();
    previous.fact(pr) == PrFact::Open
}

async fn close(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
    pr: Value,
) -> Result<()> {
    if job.pr_number != pr["number"].as_i64() {
        return Err(invalid().into());
    }
    crate::github_publication_adapter::submit(pool, root, remote, job, now, "close").await
}
