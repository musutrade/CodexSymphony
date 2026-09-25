//! Native GitHub publication adapter and compatibility binding to the existing
//! outbox. Legacy frozen jobs retain their policy and attempt counters.
use crate::{
    controlled_contract::Operation,
    delivery_extension::{self, Adapter, Control, Reply},
    delivery_store::{self as store, Pending},
    delivery_worker::Remote,
    github_http::Error,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = delivery_extension::Result<T>;

#[derive(Clone, PartialEq)]
pub struct Input {
    pub job: Pending,
    pub operation: String,
}
struct Publication<'a, R> {
    remote: &'a mut R,
}
impl<R: Remote> Adapter for Publication<'_, R> {
    type Input = Input;
    type Facts = Value;
    async fn capability_check(&mut self, input: &Input) -> Result<Reply<Input, Value>> {
        Ok(Reply {
            request: input.clone(),
            facts: self.remote.capability_check(&input.job).await?,
        })
    }
    async fn submit(&mut self, input: &Input) -> Result<Reply<Input, Value>> {
        if input.operation != "close" {
            self.capability_check(input).await?;
        }
        let facts = self.send(input).await?;
        Ok(Reply {
            request: input.clone(),
            facts,
        })
    }
    async fn observe(&mut self, input: &Input) -> Result<Reply<Input, Value>> {
        let facts = self.remote.find(&input.job).await?.unwrap_or(Value::Null);
        Ok(Reply {
            request: input.clone(),
            facts,
        })
    }
    async fn reconcile(&mut self, input: &Input) -> Result<Reply<Input, Value>> {
        self.observe(input).await
    }
}
impl<R: Remote> Publication<'_, R> {
    async fn send(&mut self, input: &Input) -> Result<Value> {
        let facts = match input.operation.as_str() {
            "push" => self.remote.push(&input.job).await?,
            "create" => self.remote.create(&input.job).await?,
            "close" => {
                self.remote
                    .close(
                        &input.job,
                        input.job.pr_number.ok_or("close PR identity missing")? as u64,
                    )
                    .await?
            }
            _ => return Err("unapproved GitHub publication operation".into()),
        };
        Ok(facts)
    }
}

struct Admission<'a> {
    pool: &'a PgPool,
    root: &'a Path,
    now: i64,
    attempt: Option<i64>,
}
impl Control<Input, Value> for Admission<'_> {
    fn check(&self, operation: &Operation, input: &Input) -> Result<()> {
        if !matches!(
            (operation, input.operation.as_str()),
            (Operation::Submit, "push" | "create" | "close")
                | (Operation::Observe | Operation::Reconcile, "observe")
        ) {
            return Err("unapproved publication capability".into());
        }
        Ok(())
    }
    async fn admit(&mut self, input: &Input) -> Result<()> {
        if !crate::storage::permit(self.pool, self.root).await {
            return Err("delivery storage unavailable".into());
        }
        if input.operation == "close" {
            return Ok(());
        }
        crate::environment_service::admit(
            self.pool,
            input.job.requirement_id,
            input.job.revision,
            "delivery",
            None,
        )
        .await?;
        crate::validation_context::delivery(self.pool, &input.job.action_key).await
    }
    async fn before_deliver(&mut self, input: &Input) -> Result<()> {
        if input.operation == "close" {
            return Ok(());
        }
        crate::delivery_hooks::run(
            self.pool,
            &input.job.action_key,
            &input.operation,
            crate::delivery_hooks::Stage::BeforeDeliver,
        )
        .await?;
        crate::delivery_hooks::run(
            self.pool,
            &input.job.action_key,
            &input.operation,
            crate::delivery_hooks::Stage::BeforePublish,
        )
        .await
    }
    async fn begin(&mut self, input: &Input) -> Result<bool> {
        self.attempt = store::begin(self.pool, &input.job, &input.operation, self.now).await?;
        Ok(self.attempt.is_some())
    }
    async fn retain(&mut self, input: &Input, reply: &Result<Reply<Input, Value>>) -> Result<()> {
        if input.operation == "observe" {
            return Ok(());
        }
        let attempt = self.attempt.ok_or("delivery intent missing")?;
        match reply {
            Ok(reply) => {
                store::receipt(self.pool, attempt, &reply.facts).await?;
                sqlx::query(
                    "UPDATE delivery_action SET next_attempt_at=$3 WHERE action_key=$1 AND kind=$2",
                )
                .bind(&input.job.action_key)
                .bind(&input.job.kind)
                .bind(self.now)
                .execute(self.pool)
                .await?;
            }
            Err(error) => {
                let error = error
                    .downcast_ref::<Error>()
                    .ok_or("delivery adapter outcome unknown")?;
                store::receipt(
                    self.pool,
                    attempt,
                    &json!({"code":error.code,"http_status":error.status}),
                )
                .await?;
                store::failed(self.pool, &input.job, self.now, error).await?;
            }
        }
        Ok(())
    }
    async fn verify(&mut self, input: &Input, _: &Reply<Input, Value>) -> Result<()> {
        if matches!(input.operation.as_str(), "observe" | "close") {
            return Ok(());
        }
        // A write response is not an observation or a business acceptance. The
        // existing worker reads the exact remote identity on the next tick.
        crate::validation_context::delivery(self.pool, &input.job.action_key).await
    }
    async fn post_delivery_validate(&mut self, _: &Input, _: &Reply<Input, Value>) -> Result<()> {
        // This adapter's applicable post-merge validation remains in the existing
        // merge_acceptance worker, after independent observation of merged SHA.
        Ok(())
    }
}
pub async fn submit(
    pool: &PgPool,
    root: &Path,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
    operation: &str,
) -> Result<()> {
    let input = Input {
        job: job.clone(),
        operation: operation.into(),
    };
    let mut adapter = Publication { remote };
    let mut control = Admission {
        pool,
        root,
        now,
        attempt: None,
    };
    let result =
        delivery_extension::invoke(&mut adapter, &mut control, Operation::Submit, &input).await;
    // Adapter errors have been durably retained. Preserve the existing worker's
    // bounded polling behavior; admission errors still block before any write.
    match result {
        Err(error) if error.downcast_ref::<Error>().is_some() => Ok(()),
        result => {
            result?;
            Ok(())
        }
    }
}

pub async fn observe(
    pool: &PgPool,
    remote: &mut impl Remote,
    job: &Pending,
    now: i64,
) -> Result<Option<Value>> {
    let input = Input {
        job: job.clone(),
        operation: "observe".into(),
    };
    let mut adapter = Publication { remote };
    let mut control = Admission {
        pool,
        root: Path::new("."),
        now,
        attempt: None,
    };
    let operation = if job.attempts == 0 {
        Operation::Observe
    } else {
        Operation::Reconcile
    };
    let reply = delivery_extension::invoke(&mut adapter, &mut control, operation, &input)
        .await?
        .ok_or("delivery observation missing")?;
    Ok((!reply.facts.is_null()).then_some(reply.facts))
}
