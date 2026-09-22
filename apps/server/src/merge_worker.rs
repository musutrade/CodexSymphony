//! Serial Broker merge execution. A failed/unknown send is only observed afterwards.
use crate::{
    automatic_merge::{self, Intent},
    github::{Capability, Observation},
    github_http::{AppClient, Error},
    merge_store as store,
};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub trait Remote: Send {
    fn current_time(&self, fallback: i64) -> i64 {
        fallback
    }
    fn observe(
        &mut self,
        intent: &Intent,
    ) -> impl std::future::Future<Output = std::result::Result<Observation, Error>> + Send;
    fn reconcile(
        &mut self,
        intent: &Intent,
    ) -> impl std::future::Future<Output = std::result::Result<Observation, Error>> + Send {
        self.observe(intent)
    }
    fn preflight(
        &mut self,
        intent: &Intent,
    ) -> impl std::future::Future<Output = std::result::Result<Capability, Error>> + Send;
    fn pr(
        &mut self,
        intent: &Intent,
    ) -> impl std::future::Future<Output = std::result::Result<Value, Error>> + Send;
    fn merge(
        &mut self,
        intent: &Intent,
    ) -> impl std::future::Future<Output = std::result::Result<Value, Error>> + Send;
}
pub struct Broker<'a> {
    pub client: &'a mut AppClient,
    pub now: i64,
}
impl Remote for Broker<'_> {
    fn current_time(&self, _: i64) -> i64 {
        crate::github_service::now()
    }
    async fn observe(&mut self, intent: &Intent) -> std::result::Result<Observation, Error> {
        self.now = crate::github_service::now();
        crate::github_observe::observe(self.client, &intent.policy, intent.pr, self.now).await
    }
    async fn reconcile(&mut self, intent: &Intent) -> std::result::Result<Observation, Error> {
        self.now = crate::github_service::now();
        crate::github_observe::observe_merge(self.client, &intent.policy, intent.pr, self.now).await
    }
    async fn preflight(&mut self, intent: &Intent) -> std::result::Result<Capability, Error> {
        self.now = crate::github_service::now();
        crate::github_observe::preflight(self.client, &intent.policy, intent.pr, self.now).await
    }
    async fn pr(&mut self, intent: &Intent) -> std::result::Result<Value, Error> {
        self.now = crate::github_service::now();
        self.client
            .get(
                &intent.policy,
                &format!("/repos/{}/pulls/{}", intent.policy.repository, intent.pr),
                self.now,
            )
            .await
    }
    async fn merge(&mut self, intent: &Intent) -> std::result::Result<Value, Error> {
        self.now = crate::github_service::now();
        let method = &intent
            .policy
            .delivery
            .as_ref()
            .ok_or_else(crate::github_http::invalid)?
            .actions
            .merge_method;
        self.client
            .write(
                &intent.policy,
                reqwest::Method::PUT,
                &format!(
                    "/repos/{}/pulls/{}/merge",
                    intent.policy.repository, intent.pr
                ),
                json!({"sha":intent.head,"merge_method":method}),
                self.now,
            )
            .await
    }
}

pub async fn tick(pool: &PgPool, remote: &mut impl Remote, now: i64) -> Result<()> {
    if let Some((intent, state, _)) = store::due(pool, now).await? {
        match advance(pool, remote, &intent, &state, now).await {
            Ok(()) => {}
            Err(error) => {
                record_error(pool, &intent, &*error, now).await?;
            }
        }
        return Ok(());
    }
    let Some(mut intent) = store::candidate(pool).await? else {
        return Ok(());
    };
    let observation = remote.observe(&intent).await?;
    let now = remote.current_time(now);
    intent.base = observation.base.clone();
    intent.checkout_sha = observation
        .phases
        .as_ref()
        .and_then(|phases| phases.first())
        .and_then(|phase| phase.expected_checkout_sha.clone());
    let (evidence, required) = store::validation(pool, &intent).await?;
    let test_merge = intent.policy.delivery.as_ref().is_some_and(|delivery| {
        delivery.pre_merge.checkout == crate::github_contract::PreMergeSource::TestMerge
    });
    if automatic_merge::admit(&intent, &observation, &evidence, &required, now)
        || (test_merge && automatic_merge::eligible(&intent, &observation, now))
    {
        store::prepare(pool, &intent, now).await?;
    }
    Ok(())
}

async fn advance(
    pool: &PgPool,
    remote: &mut impl Remote,
    intent: &Intent,
    state: &str,
    now: i64,
) -> Result<()> {
    let observation = if state == "prepared" {
        remote.observe(intent).await?
    } else {
        remote.reconcile(intent).await?
    };
    let now = remote.current_time(now);
    if let Some(sha) = automatic_merge::confirmed(intent, &observation) {
        store::merged(pool, intent, &sha, &observation).await?;
        return Ok(());
    }
    if state != "prepared" {
        return Ok(
            store::receipt(pool, intent, json!({"observation":observation}), now, 30).await?,
        );
    }
    crate::github_store::save_observation(pool, &observation).await?;
    prepare_send(pool, remote, intent, observation, now).await
}

async fn prepare_send(
    pool: &PgPool,
    remote: &mut impl Remote,
    intent: &Intent,
    observation: Observation,
    now: i64,
) -> Result<()> {
    if !automatic_merge::eligible(intent, &observation, now) {
        sqlx::query("UPDATE merge_operation SET state='invalidated',blocker='PR identity changed' WHERE action_key=$1 AND state='prepared'")
            .bind(intent.action_key()).execute(pool).await?;
        return Ok(());
    }
    let (mut evidence, required) = store::validation(pool, intent).await?;
    if intent.policy.delivery.as_ref().is_some_and(|delivery| {
        delivery.pre_merge.checkout == crate::github_contract::PreMergeSource::TestMerge
    }) {
        let saved: Option<Value> =
            sqlx::query_scalar("SELECT pre_validation FROM merge_operation WHERE action_key=$1")
                .bind(intent.action_key())
                .fetch_one(pool)
                .await?;
        let Some(saved) = saved else { return Ok(()) };
        evidence = serde_json::from_value(saved)?;
    }
    if !automatic_merge::admit(intent, &observation, &evidence, &required, now) {
        sqlx::query("UPDATE merge_operation SET state='invalidated',blocker='pre_merge identity or evidence changed' WHERE action_key=$1 AND state='prepared'")
            .bind(intent.action_key()).execute(pool).await?;
        return Ok(());
    }
    dispatch(pool, remote, intent, &observation, &required, now).await
}

async fn dispatch(
    pool: &PgPool,
    remote: &mut impl Remote,
    intent: &Intent,
    observation: &Observation,
    required: &[String],
    now: i64,
) -> Result<()> {
    let capability = remote.preflight(intent).await?;
    if capability.policy != intent.policy || !capability.blockers.is_empty() {
        store::block(
            pool,
            intent,
            "current repository capability or protection changed",
        )
        .await?;
        return Ok(());
    }
    // A merge cannot start before its configured acceptance path is executable.
    let plan = crate::merge_validation::plan(pool, intent).await?;
    let plan_steps: Vec<String> = plan.steps.iter().map(|step| step.id.clone()).collect();
    if required.iter().any(|step| !plan_steps.contains(step)) {
        store::block(
            pool,
            intent,
            "post_merge plan does not cover authorized AC steps",
        )
        .await?;
        return Ok(());
    }
    // GitHub enforces required reviews and protection at the merge itself; the
    // final read rejects unknown mergeability and a base/head movement first.
    let pr = remote.pr(intent).await?;
    let now = remote.current_time(now);
    if !automatic_merge::eligible(intent, observation, now) {
        return Ok(());
    }
    if pr["head"]["sha"] != intent.head
        || pr["base"]["sha"] != intent.base
        || pr["mergeable"] != true
        || pr["mergeable_state"] != "clean"
        || pr["state"] != "open"
        || pr["draft"] != false
    {
        store::receipt(pool, intent, json!({"pre_merge_pr":pr}), now, 30).await?;
        return Ok(());
    }
    if store::begin(pool, intent, now).await? {
        match remote.merge(intent).await {
            Ok(response) => {
                store::receipt(pool, intent, json!({"merge_response":response}), now, 30).await?
            }
            Err(error) => record_error(pool, intent, &error, now).await?,
        }
    }
    Ok(())
}
async fn record_error(
    pool: &PgPool,
    intent: &Intent,
    error: &(dyn std::error::Error + Send + Sync + 'static),
    now: i64,
) -> Result<()> {
    let delay = error
        .downcast_ref::<Error>()
        .and_then(|error| error.retry_after_seconds)
        .unwrap_or(30);
    store::receipt(
        pool,
        intent,
        json!({"error":crate::operator_view::redact_text(&error.to_string())}),
        now,
        i64::try_from(delay).unwrap_or(i64::MAX),
    )
    .await?;
    Ok(())
}
