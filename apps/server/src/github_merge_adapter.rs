//! The merge operation is a GitHub adapter action; the generic boundary owns
//! admission and intent ordering, while the original merge ledger owns recovery.
use crate::{
    automatic_merge::Intent,
    controlled_contract::Operation,
    delivery_extension::{self, Adapter, Control, Reply},
    merge_store as store,
    merge_worker::Remote,
};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = delivery_extension::Result<T>;
struct Merge<'a, R> {
    remote: &'a mut R,
    now: i64,
}
impl<R: Remote> Merge<'_, R> {
    async fn check_evidence(&mut self, intent: &Intent) -> Result<()> {
        let capability: crate::github::Capability =
            serde_json::from_value(self.capability_check(intent).await?.facts)?;
        if capability.policy != intent.policy || !capability.blockers.is_empty() {
            return Err("GitHub merge capability changed after hook".into());
        }
        let observation: crate::github::Observation =
            serde_json::from_value(self.observe(intent).await?.facts)?;
        if !crate::automatic_merge::eligible(
            intent,
            &observation,
            self.remote.current_time(self.now),
        ) {
            return Err("GitHub merge evidence changed after hook".into());
        }
        Ok(())
    }
}

impl<R: Remote> Adapter for Merge<'_, R> {
    type Input = Intent;
    type Facts = Value;
    async fn capability_check(&mut self, intent: &Intent) -> Result<Reply<Intent, Value>> {
        Ok(Reply {
            request: intent.clone(),
            facts: json!(self.remote.preflight(intent).await?),
        })
    }
    async fn submit(&mut self, intent: &Intent) -> Result<Reply<Intent, Value>> {
        self.check_evidence(intent).await?;
        let pr = self.remote.pr(intent).await?;
        if !crate::merge_worker::pr_ready(&pr, intent) {
            return Err("GitHub head/base changed after hook".into());
        }
        Ok(Reply {
            request: intent.clone(),
            facts: self.remote.merge(intent).await?,
        })
    }
    async fn observe(&mut self, intent: &Intent) -> Result<Reply<Intent, Value>> {
        Ok(Reply {
            request: intent.clone(),
            facts: json!(self.remote.observe(intent).await?),
        })
    }
    async fn reconcile(&mut self, intent: &Intent) -> Result<Reply<Intent, Value>> {
        Ok(Reply {
            request: intent.clone(),
            facts: json!(self.remote.reconcile(intent).await?),
        })
    }
}
struct Admission<'a> {
    pool: &'a PgPool,
    now: i64,
}
impl Control<Intent, Value> for Admission<'_> {
    fn check(&self, operation: &Operation, _: &Intent) -> Result<()> {
        if *operation != Operation::Submit {
            return Err("unapproved merge operation".into());
        }
        Ok(())
    }
    async fn admit(&mut self, intent: &Intent) -> Result<()> {
        crate::plugin_scope::admit(
            self.pool,
            "delivery:github",
            &intent.action_key(),
            intent.requirement,
            intent.revision,
        )
        .await?;
        crate::environment_service::admit(
            self.pool,
            intent.requirement,
            intent.revision,
            "delivery",
            None,
        )
        .await?;
        crate::validation_context::delivery(self.pool, &intent.delivery_key).await
    }
    async fn before_deliver(&mut self, intent: &Intent) -> Result<()> {
        for stage in [
            crate::delivery_hooks::Stage::BeforeDeliver,
            crate::delivery_hooks::Stage::BeforeMerge,
        ] {
            crate::delivery_hooks::run(
                self.pool,
                &intent.delivery_key,
                &intent.action_key(),
                stage,
            )
            .await?;
        }
        Ok(())
    }
    async fn begin(&mut self, intent: &Intent) -> Result<bool> {
        Ok(store::begin(self.pool, intent, self.now).await?)
    }
    async fn retain(
        &mut self,
        intent: &Intent,
        reply: &Result<Reply<Intent, Value>>,
    ) -> Result<()> {
        let fact = match reply {
            Ok(reply) => json!({"merge_response":reply.facts}),
            Err(error) => json!({"error":crate::operator_view::redact_text(&error.to_string())}),
        };
        store::receipt(self.pool, intent, fact, self.now, 30).await?;
        Ok(())
    }
    async fn verify(&mut self, intent: &Intent, _: &Reply<Intent, Value>) -> Result<()> {
        crate::validation_context::delivery(self.pool, &intent.delivery_key).await
    }
    async fn post_delivery_validate(&mut self, _: &Intent, _: &Reply<Intent, Value>) -> Result<()> {
        // This adapter's applicable post-merge validation remains in the existing
        // merge_acceptance worker, after independent observation of merged SHA.
        Ok(())
    }
}
pub async fn submit(
    pool: &PgPool,
    remote: &mut impl Remote,
    intent: &Intent,
    now: i64,
) -> Result<()> {
    delivery_extension::invoke(
        &mut Merge { remote, now },
        &mut Admission { pool, now },
        Operation::Submit,
        intent,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/github_merge_adapter.rs"]
mod tests;
