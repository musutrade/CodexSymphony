use axum::{
    Router,
    body::Body,
    http::{HeaderValue, Request, StatusCode},
    routing::any,
};
use codexsymphony_server::{config::Config, security::RequestPolicy};
use tower::ServiceExt;

fn policy() -> RequestPolicy {
    RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "http://localhost:4200".into(),
    )
    .unwrap()
}

async fn status(method: &str, headers: &[(&str, &str)]) -> StatusCode {
    let mut request = Request::builder().method(method).uri("/test-only");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = policy()
        .protect(Router::new().route("/test-only", any(|| async { StatusCode::NO_CONTENT })))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    response.status()
}

#[tokio::test]
async fn accepts_expected_reads_and_rejects_unexpected_or_ambiguous_sources() {
    for method in ["GET", "HEAD", "OPTIONS"] {
        for origin in [
            None,
            Some("http://localhost:4200"),
            Some("http://127.0.0.1:3081"),
        ] {
            let mut headers = vec![("host", "127.0.0.1:3081")];
            if let Some(origin) = origin {
                headers.push(("origin", origin));
            }
            assert_eq!(status(method, &headers).await, StatusCode::NO_CONTENT);
        }
    }
    for headers in [
        vec![],
        vec![("host", "evil.example:3081")],
        vec![("host", "127.0.0.1:3082")],
        vec![("host", "127.0.0.1:3081"), ("host", "127.0.0.1:3081")],
        vec![("host", "127.0.0.1:3081"), ("origin", "null")],
        vec![
            ("host", "127.0.0.1:3081"),
            ("origin", "https://localhost:4200"),
        ],
        vec![
            ("host", "127.0.0.1:3081"),
            ("origin", "http://localhost:4201"),
        ],
        vec![
            ("host", "127.0.0.1:3081"),
            ("origin", "http://localhost:4200.evil.example"),
        ],
        vec![
            ("host", "127.0.0.1:3081"),
            ("origin", "http://localhost:4200/"),
        ],
        vec![
            ("host", "127.0.0.1:3081"),
            ("origin", "http://localhost:4200"),
            ("origin", "http://localhost:4200"),
        ],
        vec![
            ("host", "evil.example"),
            ("x-forwarded-host", "127.0.0.1:3081"),
        ],
    ] {
        assert_eq!(
            status("GET", &headers).await,
            StatusCode::FORBIDDEN,
            "{headers:?}"
        );
    }
}

#[tokio::test]
async fn writes_require_origin_and_non_simple_csrf_header() {
    for method in ["POST", "PUT", "PATCH", "DELETE", "CUSTOM"] {
        let mut headers = vec![("host", "127.0.0.1:3081")];
        assert_eq!(status(method, &headers).await, StatusCode::FORBIDDEN);
        headers.push(("x-codexsymphony-csrf", "1"));
        assert_eq!(status(method, &headers).await, StatusCode::FORBIDDEN);
        headers.push(("origin", "http://localhost:4200"));
        assert_eq!(status(method, &headers).await, StatusCode::NO_CONTENT);
    }
    for csrf in [None, Some(""), Some("0"), Some("1, 1")] {
        let mut headers = vec![
            ("host", "127.0.0.1:3081"),
            ("origin", "http://localhost:4200"),
            ("content-type", "application/x-www-form-urlencoded"),
        ];
        if let Some(csrf) = csrf {
            headers.push(("x-codexsymphony-csrf", csrf));
        }
        assert_eq!(status("POST", &headers).await, StatusCode::FORBIDDEN);
    }
    for origin in ["null", "http://evil.example", "http://127.0.0.1:4200"] {
        assert_eq!(
            status(
                "POST",
                &[
                    ("host", "127.0.0.1:3081"),
                    ("origin", origin),
                    ("x-codexsymphony-csrf", "1")
                ]
            )
            .await,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        status(
            "POST",
            &[
                ("host", "127.0.0.1:3081"),
                ("origin", "http://localhost:4200"),
                ("x-codexsymphony-csrf", "1"),
                ("x-codexsymphony-csrf", "1")
            ]
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn rejects_invalid_header_bytes_and_conflicting_absolute_authority() {
    for (uri, host, expected) in [
        (
            "http://evil.example/test",
            HeaderValue::from_static("127.0.0.1:3081"),
            StatusCode::FORBIDDEN,
        ),
        (
            "/test",
            HeaderValue::from_bytes(b"\xff").unwrap(),
            StatusCode::FORBIDDEN,
        ),
        (
            "http://127.0.0.1:3081/test",
            HeaderValue::from_static("127.0.0.1:3081"),
            StatusCode::NO_CONTENT,
        ),
    ] {
        let response = policy()
            .protect(Router::new().route("/test", any(|| async { StatusCode::NO_CONTENT })))
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("host", host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[test]
fn validates_configuration_without_echoing_database_credentials() {
    let database = "postgres://synthetic:fixture@127.0.0.1:54329/example_test";
    for bind in ["127.0.0.1:3081", "127.0.0.1:0", "[::1]:3081"] {
        for origin in [
            "http://localhost:4200",
            "http://127.0.0.1:4200",
            "http://[::1]:4200",
            "http://localhost",
        ] {
            assert!(Config::parse(database.into(), bind, origin.into()).is_ok());
        }
    }
    for bind in [
        "0.0.0.0:3081",
        "[::]:3081",
        "192.168.1.2:3081",
        "localhost:3081",
        "invalid",
    ] {
        assert!(Config::parse(database.into(), bind, "http://localhost:4200".into()).is_err());
    }
    for origin in [
        "",
        "null",
        "localhost:4200",
        "https://localhost:4200",
        "http://evil.example",
        "http://localhost.evil.example",
        "http://user@localhost:4200",
        "http://localhost:4200/",
        "http://localhost:4200?q=1",
        "http://localhost:4200#fragment",
        "http://localhost:0",
        "http://localhost:65536",
        "http://localhost:bad",
    ] {
        assert!(
            Config::parse(database.into(), "127.0.0.1:3081", origin.into()).is_err(),
            "{origin}"
        );
    }
    assert!(
        Config::parse(
            "not-a-url-with-sensitive-data".into(),
            "127.0.0.1:3081",
            "http://localhost:4200".into()
        )
        .err()
        .unwrap()
        .contains("PostgreSQL URL")
    );
    assert!(
        RequestPolicy::new(
            "0.0.0.0:3081".parse().unwrap(),
            "http://localhost:4200".into()
        )
        .is_err()
    );
    assert!(
        RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "http://evil.example".into()
        )
        .is_err()
    );
}
