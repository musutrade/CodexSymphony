//! Legacy response adaptation preserves status and non-JSON HTTP responses.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use tower::ServiceExt;

#[tokio::test]
async fn non_json_response_is_preserved() {
    let app = Router::new()
        .route("/", get(|| async { (StatusCode::ACCEPTED, "unchanged") }))
        .layer(axum::middleware::from_fn(
            codexsymphony_server::business_legacy::response,
        ));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        to_bytes(response.into_body(), 100).await.unwrap(),
        "unchanged"
    );
}
