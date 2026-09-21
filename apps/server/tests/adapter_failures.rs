//! Adapter failure contracts remain testable when authentication itself needs the DB.
//! Mount only the adapter under test; production router authentication stays intact.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::IntoResponse,
};
use codexsymphony_server::{business, execution_api, operator_api, runtime_api, validation_api};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

#[tokio::test]
async fn storage_failures_are_sanitized_by_each_business_adapter() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/unused")
        .unwrap();
    pool.close().await;
    let app = Router::new()
        .merge(business::routes())
        .merge(execution_api::routes())
        .merge(operator_api::routes())
        .merge(runtime_api::routes())
        .merge(validation_api::routes())
        .with_state(pool);
    for (path, message) in [
        ("/api/requirements", "database unavailable"),
        ("/api/execution", "database unavailable"),
        ("/api/requirements/1/operations", "storage unavailable"),
        (
            "/api/requirements/1/questions",
            "question storage unavailable",
        ),
        ("/api/validations/missing", "database unavailable"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"error":message}),
            "{path}"
        );
    }
    let invalid = serde_json::from_str::<serde_json::Value>("private damaged record").unwrap_err();
    let response = business::ApiError::from(invalid).into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"error":"stored record unavailable"})
    );
}
