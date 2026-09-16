//! Dynamic tools return only durable results, keyed by the original RPC id.
use crate::{
    execution::RunKey,
    git_broker::GitBroker,
    runtime,
    runtime_protocol::DynamicToolCallParams,
    runtime_store,
    workspace_store::{self, Operation},
};
use serde_json::Value;
use sqlx::PgPool;

pub async fn handle(
    pool: &PgPool,
    broker: &GitBroker,
    key: &RunKey,
    request: &Value,
) -> runtime_store::Result<Value> {
    if let Some(result) = runtime_store::request(pool, key, request).await? {
        return Ok(result);
    };
    let operation = dispatch(pool, broker, key, request).await;
    match operation {
        Ok(Some(ended)) => Ok(ended),
        Ok(None) => Err(runtime_store::invalid("tool result unavailable")),
        Err(_) => {
            // Diagnostics and SQL/Git errors may contain local secrets. Return a
            // fixed failure; retain pending Broker intents for reconciliation.
            let result = runtime::reply(
                false,
                "Tool rejected: invalid arguments, authorization, candidate or unresolved operation.",
            );
            runtime_store::result(pool, key, &request["id"], &result).await?;
            Ok(result)
        }
    }
}
async fn dispatch(
    pool: &PgPool,
    broker: &GitBroker,
    key: &RunKey,
    request: &Value,
) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let params: DynamicToolCallParams = serde_json::from_value(request["params"].clone())?;
    runtime_store::require(params.namespace.is_none(), "unexpected tool namespace")?;
    match params.tool.as_str() {
        "create_local_commit" => commit(pool, broker, key, request, params.arguments).await,
        "report_completion" => completion(pool, key, request, &params.arguments).await,
        "report_blocker" => blocker(pool, key, request, &params.arguments).await,
        _ => Err("unknown tool".into()),
    }
}

/// Automatic sandbox execution needs no client approval. Requests to expand
/// permissions are denied; only an explicit report_blocker creates a blocker.
pub fn deny(method: &str) -> Value {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            serde_json::json!({"decision":"decline"})
        }
        "item/permissions/requestApproval" => serde_json::json!({"permissions":{},"scope":"turn"}),
        "execCommandApproval" | "applyPatchApproval" => serde_json::json!({"decision":"denied"}),
        _ => serde_json::json!({"error":{"code":-32601,"message":"unsupported server request"}}),
    }
}

async fn commit(
    pool: &PgPool,
    broker: &GitBroker,
    key: &RunKey,
    request: &Value,
    arguments: Value,
) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let args: runtime::Commit = serde_json::from_value(arguments)?;
    runtime_store::require(
        runtime::text_valid(&args.message, 4096),
        "invalid commit message",
    )?;
    let id = request["id"].to_string();
    let value = workspace_store::execute(
        pool,
        broker,
        key,
        &format!("rpc:{id}"),
        Operation::Commit {
            message: args.message,
        },
    )
    .await?;
    let result = runtime::reply(true, &value.to_string());
    runtime_store::result(pool, key, &request["id"], &result).await?;
    Ok(Some(result))
}

async fn completion(
    pool: &PgPool,
    key: &RunKey,
    request: &Value,
    arguments: &Value,
) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let args: runtime::Completion = serde_json::from_value(arguments.clone())?;
    runtime_store::require(runtime::text_valid(&args.summary, 8192), "invalid summary")?;
    Ok(Some(
        runtime_store::end(pool, key, request, "completion", arguments).await?,
    ))
}
async fn blocker(
    pool: &PgPool,
    key: &RunKey,
    request: &Value,
    arguments: &Value,
) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let args: runtime::Blocker = serde_json::from_value(arguments.clone())?;
    runtime_store::require(
        runtime::text_valid(&args.reason, 8192),
        "invalid blocker reason",
    )?;
    Ok(Some(
        runtime_store::end(pool, key, request, "blocker", arguments).await?,
    ))
}
