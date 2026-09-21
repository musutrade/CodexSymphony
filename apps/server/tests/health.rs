use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower::ServiceExt;

// The approved host collects the health and startup integration targets.
#[path = "support/security.rs"]
mod security;

#[tokio::test]
async fn health_checks_a_real_database() {
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to a disposable test database");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let response = codexsymphony_server::router(pool.clone(), policy())
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .extension(axum::extract::ConnectInfo(
                    "127.0.0.1:50000".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .header("host", "127.0.0.1:3081")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(json, serde_json::json!({"status":"ok","database":"ok"}));
    pool.close().await;
}

#[tokio::test]
async fn unavailable_database_is_503_without_connection_details() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://not-a-secret@127.0.0.1:1/unavailable_test")
        .unwrap();
    pool.close().await;
    let response = codexsymphony_server::router(pool, policy())
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .extension(axum::extract::ConnectInfo(
                    "127.0.0.1:50000".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .header("host", "127.0.0.1:3081")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"status":"unavailable","database":"unavailable"})
    );
}

fn policy() -> codexsymphony_server::security::RequestPolicy {
    codexsymphony_server::security::RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "https://localhost:4200".into(),
    )
    .unwrap()
}
