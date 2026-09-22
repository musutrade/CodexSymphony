//! One persisted workflow dispatch per approved workflow and merged source.
use crate::{
    automatic_merge::Intent,
    github::{Policy, Source},
    github_contract::{PostMerge, Trigger},
    github_http::AppClient,
    merge_store,
};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) async fn dispatch(
    pool: &PgPool,
    client: &mut AppClient,
    intent: &Intent,
    sha: &str,
    now: i64,
) -> Result<()> {
    let Some(contract) = &intent.policy.delivery else {
        return Ok(());
    };
    let PostMerge::Checks { checks, .. } = &contract.post_merge else {
        return Ok(());
    };
    for check in checks {
        if check.trigger != Trigger::WorkflowDispatch {
            continue;
        }
        let Source::Actions {
            workflow_id,
            branch,
            ..
        } = &check.selector.source
        else {
            return Err("dispatch requires a pinned Actions workflow".into());
        };
        send(pool, client, intent, sha, *workflow_id, branch, now).await?;
    }
    Ok(())
}
async fn send(
    pool: &PgPool,
    client: &mut AppClient,
    intent: &Intent,
    sha: &str,
    workflow: u64,
    branch: &str,
    now: i64,
) -> Result<()> {
    let identity = json!({"workflow":workflow,"sha":sha,"branch":branch});
    let mut tx = crate::run_store::lock(pool).await?;
    if !merge_store::allowed(&mut tx, intent).await? {
        return Ok(());
    }
    let existing: bool =
        sqlx::query_scalar("SELECT dispatches @> $2 FROM merge_operation WHERE action_key=$1")
            .bind(intent.action_key())
            .bind(json!([identity]))
            .fetch_one(&mut *tx)
            .await?;
    if existing {
        return Ok(());
    }
    tx.commit().await?;
    if !branch_matches(client, &intent.policy, branch, sha, now).await? {
        return Err("post_merge dispatch branch no longer names merged SHA".into());
    }
    let mut tx = crate::run_store::lock(pool).await?;
    if !merge_store::allowed(&mut tx, intent).await? {
        return Ok(());
    }
    let changed=sqlx::query("UPDATE merge_operation SET dispatches=dispatches||$2 WHERE action_key=$1 AND state='merged' AND NOT dispatches @> $2")
        .bind(intent.action_key()).bind(json!([identity])).execute(&mut *tx).await?;
    tx.commit().await?;
    if changed.rows_affected() != 1 {
        return Ok(());
    }
    let result = client
        .write(
            &intent.policy,
            reqwest::Method::POST,
            &format!(
                "/repos/{}/actions/workflows/{workflow}/dispatches",
                intent.policy.repository
            ),
            json!({"ref":branch}),
            now,
        )
        .await;
    let receipt: Value = match result {
        Ok(value) => json!({"workflow_dispatch":identity,"response":value}),
        Err(error) => json!({"workflow_dispatch":identity,"error":error.to_string()}),
    };
    merge_store::receipt(pool, intent, receipt, now, 30).await?;
    Ok(())
}
async fn branch_matches(
    client: &mut AppClient,
    policy: &Policy,
    branch: &str,
    sha: &str,
    now: i64,
) -> Result<bool> {
    let path = format!(
        "/repos/{}/git/ref/heads/{}",
        policy.repository,
        crate::github_observe::segment(branch)
    );
    let value = client.get(policy, &path, now).await?;
    Ok(value["object"]["sha"] == sha)
}
