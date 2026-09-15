//! Minimal localhost API; no scheduler or business completion state is implied.
pub mod business;
pub mod config;
pub mod contract;
pub mod coordinator;
pub mod execution;
pub mod execution_api;
pub mod git_broker;
pub mod github;
pub mod github_http;
pub mod github_observe;
pub mod github_service;
pub mod github_store;
pub mod preparation;
pub mod preparation_service;
pub mod preparation_store;
pub mod process;
pub mod run_store;
pub mod security;
pub mod storage;
pub mod workspace;
pub mod workspace_files;
pub mod workspace_store;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
    database: &'static str,
}

pub fn router(pool: PgPool, policy: security::RequestPolicy) -> Router {
    policy.protect(
        Router::new()
            .route("/api/health", get(health))
            .merge(business::routes())
            .merge(execution_api::routes())
            .with_state(pool),
    )
}

async fn health(State(pool): State<PgPool>) -> (StatusCode, Json<Health>) {
    response(database_available(&pool).await)
}

async fn database_available(pool: &PgPool) -> bool {
    let probe = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool);
    probe_succeeded(tokio::time::timeout(Duration::from_secs(2), probe).await)
}

fn probe_succeeded(result: Result<Result<i32, sqlx::Error>, tokio::time::error::Elapsed>) -> bool {
    matches!(result, Ok(Ok(1)))
}

fn response(available: bool) -> (StatusCode, Json<Health>) {
    let (code, status, database) = if available {
        (StatusCode::OK, "ok", "ok")
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "unavailable",
        )
    };
    (code, Json(Health { status, database }))
}
