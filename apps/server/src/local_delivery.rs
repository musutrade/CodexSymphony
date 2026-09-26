//! Local adapter plugs into the same delivery boundary and publication ledger.
use crate::{
    controlled_contract::Operation,
    delivery_extension::{self, Adapter, Control, Reply, Result},
    git_broker::GitBroker,
    local_delivery_store::{self as store, Job},
    local_git::{self, Observation},
};
use sqlx::PgPool;
use std::path::Path;

struct Local<'a> {
    pool: &'a PgPool,
    broker: &'a GitBroker,
}
impl Adapter for Local<'_> {
    type Input = Job;
    type Facts = Observation;
    async fn capability_check(&mut self, job: &Job) -> Result<Reply<Job, Observation>> {
        self.observe(job).await
    }
    async fn observe(&mut self, job: &Job) -> Result<Reply<Job, Observation>> {
        let binding = job.binding()?;
        Ok(Reply {
            request: job.clone(),
            facts: local_git::observe(&binding, &job.action_key, job.baseline()?, &job.head_sha)?,
        })
    }
    async fn reconcile(&mut self, job: &Job) -> Result<Reply<Job, Observation>> {
        self.observe(job).await
    }
    async fn submit(&mut self, job: &Job) -> Result<Reply<Job, Observation>> {
        let mut tx = crate::run_store::lock(self.pool).await?;
        if !store::allowed(&mut tx, job).await? {
            return Err("local delivery authority changed".into());
        }
        let facts = submit_candidate(self.broker, job)?;
        tx.commit().await?;
        Ok(Reply {
            request: job.clone(),
            facts,
        })
    }
}
fn submit_candidate(broker: &GitBroker, job: &Job) -> Result<Observation> {
    let binding = job.binding()?;
    if store::resolve_document(&job.policy)? != binding {
        return Err("local target registration changed".into());
    }
    let manifest = serde_json::from_value(job.manifest.clone())?;
    let source = broker.delivery_repository(&manifest)?;
    local_git::submit(
        &binding,
        &source,
        &job.action_key,
        job.baseline()?,
        &job.head_sha,
    )
}

struct Admission<'a> {
    pool: &'a PgPool,
    root: &'a Path,
    attempt: Option<i64>,
}
impl Control<Job, Observation> for Admission<'_> {
    fn check(&self, operation: &Operation, _: &Job) -> Result<()> {
        match operation {
            Operation::Submit
            | Operation::Observe
            | Operation::Reconcile
            | Operation::CapabilityCheck => Ok(()),
            _ => Err("unapproved local operation".into()),
        }
    }
    async fn admit(&mut self, job: &Job) -> Result<()> {
        crate::plugin_scope::admit(
            self.pool,
            "delivery:local_git",
            &job.action_key,
            job.requirement_id,
            job.revision,
        )
        .await?;
        if !crate::storage::permit(self.pool, self.root).await {
            return Err("local delivery storage unavailable".into());
        }
        crate::environment_service::admit(
            self.pool,
            job.requirement_id,
            job.revision,
            "delivery",
            None,
        )
        .await?;
        crate::validation_context::delivery(self.pool, &job.action_key).await
    }
    async fn before_deliver(&mut self, job: &Job) -> Result<()> {
        crate::delivery_hooks::run(
            self.pool,
            &job.action_key,
            "local_update",
            crate::delivery_hooks::Stage::BeforeDeliver,
        )
        .await
    }
    async fn begin(&mut self, job: &Job) -> Result<bool> {
        self.attempt = store::begin(self.pool, job).await?;
        Ok(self.attempt.is_some())
    }
    async fn retain(&mut self, _: &Job, reply: &Result<Reply<Job, Observation>>) -> Result<()> {
        if let Some(attempt) = self.attempt
            && let Ok(reply) = reply
        {
            crate::delivery_store::receipt(
                self.pool,
                attempt,
                &serde_json::json!({"local_observation":reply.facts}),
            )
            .await?;
        }
        Ok(())
    }
    async fn verify(&mut self, job: &Job, reply: &Reply<Job, Observation>) -> Result<()> {
        store::observed(self.pool, job, &reply.facts).await
    }
    async fn post_delivery_validate(&mut self, _: &Job, _: &Reply<Job, Observation>) -> Result<()> {
        Ok(())
    }
}

pub async fn tick(pool: &PgPool, root: &Path, broker: &GitBroker) -> Result<bool> {
    tick_with_hooks(pool, root, broker, &serde_json::Value::Null).await
}

pub async fn tick_with_hooks(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    hooks: &serde_json::Value,
) -> Result<bool> {
    let Some(job) = store::pending(pool).await? else {
        return Ok(false);
    };
    if let Err(error) = process(pool, root, broker, &job, hooks).await {
        store::blocked(pool, &job, &error.to_string()).await?;
    }
    Ok(true)
}

async fn process(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    job: &Job,
    hooks: &serde_json::Value,
) -> Result<()> {
    crate::local_acceptance_process::reconcile_started(pool, root, job).await?;
    if job.attempts == 0 && cancelled(pool, job).await? {
        return Ok(());
    }
    let mut adapter = Local { pool, broker };
    let mut control = Admission {
        pool,
        root,
        attempt: None,
    };
    let reply = observe(&mut adapter, &mut control, job).await?;
    if cancelled(pool, job).await? {
        return Ok(());
    }
    continue_observed(
        pool,
        root,
        broker,
        job,
        &reply.facts,
        &mut adapter,
        &mut control,
        hooks,
    )
    .await
}

async fn cancelled(pool: &PgPool, job: &Job) -> Result<bool> {
    let cancelled = store::cancel_unsent(pool, job).await?;
    if cancelled {
        crate::delivery_control::settle(pool).await?;
    }
    Ok(cancelled)
}

async fn observe(
    adapter: &mut Local<'_>,
    control: &mut Admission<'_>,
    job: &Job,
) -> Result<Reply<Job, Observation>> {
    let operation = if job.attempts == 0 {
        Operation::CapabilityCheck
    } else {
        Operation::Reconcile
    };
    let reply = delivery_extension::invoke(adapter, control, operation, job)
        .await?
        .ok_or("local observation unavailable")?;
    Ok(reply)
}

#[allow(clippy::too_many_arguments)]
async fn continue_observed(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    job: &Job,
    fact: &Observation,
    adapter: &mut Local<'_>,
    control: &mut Admission<'_>,
    hooks: &serde_json::Value,
) -> Result<()> {
    match fact {
        Observation::Delivered => {
            crate::local_acceptance::accept(pool, root, broker, job, hooks).await
        }
        Observation::Conflict => Err("local target baseline or operation receipt conflict".into()),
        Observation::NotSubmitted => {
            if job.attempts > 0 {
                return Err("local write outcome unknown; retain original intent for operator reconciliation".into());
            }
            delivery_extension::invoke(adapter, control, Operation::Submit, job).await?;
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/local_delivery.rs"]
mod tests;
