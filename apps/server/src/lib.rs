pub mod auth;
pub mod auth_admin;
pub mod auth_api;
pub mod auth_store;
pub mod recovery;

pub mod budget;
pub mod budget_store;
pub mod business;
pub mod config;
pub mod contract;
pub mod coordinator;
pub mod delivery;
pub mod delivery_control;
pub mod delivery_remote;
pub mod delivery_store;
pub mod delivery_worker;
pub mod draft;
pub mod draft_api;
pub mod execution;
pub mod execution_api;
pub mod generation;
pub mod generation_api;
pub mod generation_runtime;
pub mod generation_store;
pub mod git_broker;
pub mod github;
pub mod github_contract;
pub mod github_delivery;
pub mod github_http;
pub mod github_observe;
pub mod github_service;
pub mod github_store;
pub mod group_api;
pub mod group_budget;
pub mod group_completion;
pub mod group_dependency;
pub mod group_edit;
pub mod group_edit_api;
pub mod group_edit_apply;
pub mod group_edit_requirement;
pub mod group_edit_store;
pub mod group_queue_store;
pub mod group_queue_view;
pub mod group_review;
pub mod group_store;
pub mod operator_api;
pub mod operator_control;
pub mod operator_view;
pub mod preparation;
pub mod preparation_service;
pub mod preparation_store;
pub mod process;
pub mod run_store;
pub mod runtime;
pub mod runtime_api;
pub mod runtime_client;
pub mod runtime_protocol;
pub mod runtime_questions;
pub mod runtime_resume;
pub mod runtime_service;
pub mod runtime_store;
pub mod runtime_tools;
pub mod runtime_transport;
pub mod security;
pub mod storage;
pub mod storage_archive;
pub mod storage_cleanup;
pub mod storage_consumers;
pub mod storage_db;
pub mod storage_files;
pub mod storage_inventory;
pub mod storage_lifecycle;
pub mod storage_measure;
pub mod storage_output;
pub mod storage_service;
pub mod storage_store;
pub mod storage_view;
pub mod validation;
pub mod validation_api;
pub mod validation_store;
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
            .merge(auth_api::routes())
            .merge(business::routes())
            .merge(draft_api::routes())
            .merge(group_api::routes())
            .merge(group_edit_api::routes())
            .merge(generation_api::routes())
            .merge(operator_api::routes())
            .merge(execution_api::routes())
            .merge(validation_api::routes())
            .merge(runtime_api::routes())
            .layer(axum::middleware::from_fn(draft_api::guard_legacy))
            .with_state(pool.clone()),
        pool,
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

pub mod validation_repair;
pub mod validation_repair_worker;
pub mod validation_runner;
pub mod validation_service;
pub mod validation_worker;

pub mod runtime_initial;

pub mod runtime_routes;

pub mod business_legacy;
