use super::*;
use axum::{body::Body, routing::any};
use tower::ServiceExt;

#[tokio::test]
async fn observation_allows_reads_and_authentication_but_never_business_writes() {
    let router = protect(
        Router::new().fallback(any(|| async { StatusCode::ACCEPTED })),
        Mode::ObservationOnly,
    );
    for (method, path, expected) in [
        ("GET", "/api/requirements", 202),
        ("HEAD", "/api/requirements", 202),
        ("OPTIONS", "/api/requirements", 202),
        ("POST", "/api/auth/login", 202),
        ("POST", "/api/auth/logout", 202),
        ("PUT", "/api/auth/login", 503),
        ("POST", "/api/auth/login/extra", 503),
        ("POST", "/api/operator/resume", 503),
        ("PATCH", "/api/requirements/8", 503),
        ("DELETE", "/api/requirements/8", 503),
        ("POST", "/api/future-operation", 503),
    ] {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected, "{method} {path}");
        assert_eq!(response.headers()[HEADER], "observation-only");
    }
}

#[tokio::test]
async fn normal_mode_preserves_writes_and_admission_error_never_enables_execution() {
    assert_eq!(Mode::from_admission(Ok(())), Mode::Normal);
    assert_eq!(
        Mode::from_admission(Err("unknown probe".into())),
        Mode::ObservationOnly
    );
    let router = protect(
        Router::new().fallback(any(|| async { StatusCode::ACCEPTED })),
        Mode::Normal,
    );
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/operator/resume")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers()[HEADER], "normal");
}
