//! Authentication replaces the former fixed-header localhost boundary.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::Request,
};
use codexsymphony_server::{auth, auth_store, config::Config, security::RequestPolicy};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use tower::ServiceExt;
struct Browser {
    cookie: String,
    csrf: String,
}
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    client: Option<&Browser>,
    headers: &[(&str, &str)],
    body: Value,
) -> axum::response::Response {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .extension(ConnectInfo(
            "127.0.0.1:51000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("host", "127.0.0.1:3081")
        .header("content-type", "application/json");
    if let Some(client) = client {
        req = req.header("cookie", &client.cookie);
    }
    for (key, value) in headers {
        req = req.header(*key, *value);
    }
    app.clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap()
}
async fn bootstrap(app: &Router) -> Browser {
    let response = send(app, "GET", "/api/auth/csrf", None, &[], json!({})).await;
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(attribute));
    }
    let cookie = cookie.split(';').next().unwrap().to_owned();
    Browser {
        cookie,
        csrf: body(response).await["csrf_token"].as_str().unwrap().into(),
    }
}
async fn login(
    app: &Router,
    client: &Browser,
    name: &str,
    password: &str,
) -> axum::response::Response {
    send(
        app,
        "POST",
        "/api/auth/login",
        Some(client),
        &[
            ("origin", "https://localhost:4200"),
            ("x-codexsymphony-csrf", &client.csrf),
        ],
        json!({"username":name,"password":password}),
    )
    .await
}
async fn signed_in(app: &Router) -> Browser {
    let response = login(
        app,
        &bootstrap(app).await,
        "operator",
        "synthetic-password-1",
    )
    .await;
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Max-Age=28800"));
    let cookie = cookie.split(';').next().unwrap().to_owned();
    Browser {
        cookie,
        csrf: body(response).await["csrf_token"].as_str().unwrap().into(),
    }
}
async fn fixture() -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!("auth_{}", auth::random().unwrap());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    pool
}
fn app(pool: &PgPool, clock: Arc<AtomicI64>) -> Router {
    let policy = RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "https://localhost:4200".into(),
    )
    .unwrap()
    .with_clock(Arc::new(move || clock.load(Ordering::SeqCst)));
    codexsymphony_server::router(pool.clone(), policy)
}
#[tokio::test]
async fn accounts_sessions_revocation_expiry_csrf_and_persistent_limits() {
    let pool = fixture().await;
    let clock = Arc::new(AtomicI64::new(1_800_000_000));
    let server = app(&pool, clock.clone());
    assert!(
        auth_store::account(&pool, "operator", "short", false)
            .await
            .is_err()
    );
    auth_store::account(&pool, "operator", "synthetic-password-1", false)
        .await
        .unwrap();
    auth_store::account(&pool, "second", "synthetic-password-1", false)
        .await
        .unwrap();
    let hashes: Vec<String> = sqlx::query_scalar("SELECT password_hash FROM platform_account")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(hashes.iter().all(|h| h.starts_with("$argon2id$")));
    assert_ne!(hashes[0], hashes[1]);
    route_coverage(&server, &pool).await;
    let desktop = signed_in(&server).await;
    let mobile = signed_in(&server).await;
    let proof = send(
        &server,
        "GET",
        "/api/auth/csrf",
        Some(&desktop),
        &[],
        json!({}),
    )
    .await;
    assert_eq!(proof.status(), 200);
    assert!(
        !proof.headers().contains_key("set-cookie"),
        "reads do not renew the absolute expiry"
    );
    assert_eq!(body(proof).await["csrf_token"], desktop.csrf);
    let stored: (String, i64, bool) =
        sqlx::query_as("SELECT digest,expires_at,revoked FROM platform_session WHERE digest=$1")
            .bind(auth::digest(desktop.cookie.split_once('=').unwrap().1))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored.1,
        clock.load(Ordering::SeqCst) + auth::SESSION_SECONDS
    );
    assert!(!stored.2);
    assert!(stored.0 != desktop.cookie.split_once('=').unwrap().1);
    let before = body(
        send(
            &server,
            "GET",
            "/api/drafts",
            Some(&desktop),
            &[],
            json!({}),
        )
        .await,
    )
    .await;
    assert_eq!(
        before,
        body(send(&server, "GET", "/api/drafts", Some(&mobile), &[], json!({})).await).await
    );
    for headers in [
        vec![],
        vec![
            ("origin", "https://evil.example"),
            ("x-codexsymphony-csrf", desktop.csrf.as_str()),
        ],
        vec![
            ("origin", "https://localhost:4200"),
            ("x-codexsymphony-csrf", "1"),
        ],
        vec![
            ("origin", "https://localhost:4200"),
            ("x-codexsymphony-csrf", mobile.csrf.as_str()),
        ],
    ] {
        assert_eq!(
            send(
                &server,
                "POST",
                "/api/auth/logout",
                Some(&desktop),
                &headers,
                json!({})
            )
            .await
            .status(),
            403
        );
        assert_eq!(
            send(
                &server,
                "POST",
                "/api/auth/login",
                Some(&desktop),
                &headers,
                json!({"username":"operator","password":"synthetic-password-1"})
            )
            .await
            .status(),
            403
        );
        assert_eq!(
            send(
                &server,
                "POST",
                "/api/drafts",
                Some(&desktop),
                &headers,
                json!({})
            )
            .await
            .status(),
            403
        );
    }
    assert_eq!(
        send(
            &server,
            "POST",
            "/api/auth/logout",
            Some(&desktop),
            &[
                ("origin", "https://localhost:4200"),
                ("x-codexsymphony-csrf", &desktop.csrf)
            ],
            json!({})
        )
        .await
        .status(),
        204
    );
    let restarted = app(&pool, clock.clone());
    assert_eq!(
        send(
            &restarted,
            "GET",
            "/api/drafts",
            Some(&desktop),
            &[],
            json!({})
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        send(
            &restarted,
            "GET",
            "/api/drafts",
            Some(&mobile),
            &[],
            json!({})
        )
        .await
        .status(),
        200
    );
    clock.fetch_add(auth::SESSION_SECONDS, Ordering::SeqCst);
    assert_eq!(
        send(
            &app(&pool, clock.clone()),
            "GET",
            "/api/drafts",
            Some(&mobile),
            &[],
            json!({})
        )
        .await
        .status(),
        401
    );
    let desktop = signed_in(&restarted).await;
    let mobile = signed_in(&restarted).await;
    auth_store::account(&pool, "operator", "synthetic-password-2", true)
        .await
        .unwrap();
    let restarted = app(&pool, clock.clone());
    for client in [&desktop, &mobile] {
        assert_eq!(
            send(
                &restarted,
                "GET",
                "/api/drafts",
                Some(client),
                &[],
                json!({})
            )
            .await
            .status(),
            401
        );
    }
    let anonymous = bootstrap(&restarted).await;
    let wrong = login(&restarted, &anonymous, "operator", "synthetic-password-1").await;
    assert_eq!(wrong.status(), 401);
    let unknown = login(&restarted, &anonymous, "missing", "synthetic-password-1").await;
    assert_eq!(unknown.status(), 401);
    assert_eq!(body(wrong).await, body(unknown).await);
    assert_eq!(
        login(&restarted, &anonymous, "operator", "synthetic-password-2")
            .await
            .status(),
        200
    );
    persistent_limits(&pool, clock, &bootstrap(&restarted).await).await;
}
async fn route_coverage(server: &Router, pool: &PgPool) {
    let before = business_snapshot(pool).await;
    // Discover actual route declarations, including APIs absent from OpenAPI.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(path).unwrap();
        for route in text.split(".route(").skip(1) {
            let literal = route.trim_start();
            let template = literal.split('"').nth(1).expect("route path string");
            if literal.starts_with('"') {
                paths.insert(template.to_owned());
            } else {
                assert!(
                    literal.starts_with("&format!(\"{prefix}/"),
                    "unsupported dynamic route declaration"
                );
                let prefixes: Vec<_> = text
                    .split("operations(\"")
                    .skip(1)
                    .map(|call| call.split('"').next().unwrap())
                    .collect();
                assert!(
                    !prefixes.is_empty(),
                    "dynamic route prefixes must be inventoried"
                );
                for prefix in prefixes {
                    paths.insert(
                        template
                            .replace("{prefix}", prefix)
                            .replace("{{", "{")
                            .replace("}}", "}"),
                    );
                }
            }
        }
    }
    assert!(
        paths.len() >= 25,
        "route discovery must include every API module"
    );
    for path in paths {
        let path = path
            .replace("{id}", "1")
            .replace("{run_id}", "run")
            .replace("{child_id}", "child");
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] {
            if (method == "GET" && matches!(path.as_str(), "/api/health" | "/api/auth/csrf"))
                || (method == "POST" && path == "/api/auth/login")
            {
                continue;
            }
            assert_eq!(
                send(
                    server,
                    method,
                    &path,
                    None,
                    &[
                        ("cf-access-authenticated-user-email", "owner@example.test"),
                        ("cf-access-jwt-assertion", "forged"),
                        ("x-forwarded-for", "198.51.100.9")
                    ],
                    json!({})
                )
                .await
                .status(),
                401,
                "{method} {path}"
            );
        }
    }
    assert_eq!(
        send(server, "POST", "/api/future-route", None, &[], json!({}))
            .await
            .status(),
        401
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM requirement")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(business_snapshot(pool).await, before);
}
async fn persistent_limits(pool: &PgPool, clock: Arc<AtomicI64>, anonymous: &Browser) {
    let server = app(pool, clock.clone());
    for _ in 0..10 {
        assert_eq!(
            login(&server, anonymous, "limited", "wrong").await.status(),
            401
        );
    }
    let restarted = app(pool, clock.clone());
    assert_eq!(
        login(&restarted, anonymous, "limited", "wrong")
            .await
            .status(),
        429
    );
    for n in 0..55 {
        let result = send(
            &restarted,
            "POST",
            "/api/auth/login",
            Some(anonymous),
            &[
                ("origin", "https://localhost:4200"),
                ("x-codexsymphony-csrf", &anonymous.csrf),
                ("x-forwarded-for", &format!("198.51.100.{n}")),
            ],
            json!({"username":format!("unknown{n}"),"password":"wrong"}),
        )
        .await;
        if n == 54 {
            assert_eq!(result.status(), 429);
        }
    }
    clock.fetch_add(901, Ordering::SeqCst);
    assert_eq!(
        login(
            &app(pool, clock),
            &bootstrap(&restarted).await,
            "limited",
            "wrong"
        )
        .await
        .status(),
        401
    );
}
#[test]
fn invalid_configuration_is_closed() {
    for origin in [
        "http://localhost:4200",
        "https://example.test/",
        "https://user@example.test",
        "https://example.test:0",
        "https://example.test:bad",
        "https://example.test?x=1",
        "https://example.test#fragment",
    ] {
        assert!(
            RequestPolicy::new("127.0.0.1:3081".parse().unwrap(), origin.into()).is_err(),
            "{origin}"
        );
    }
    assert!(
        Config::parse(
            "not-postgres".into(),
            "127.0.0.1:3081",
            "https://localhost:4200".into()
        )
        .is_err()
    );
    assert!(
        RequestPolicy::new(
            "0.0.0.0:3081".parse().unwrap(),
            "https://localhost:4200".into()
        )
        .is_err()
    );
    let policy = RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "https://localhost:4200".into(),
    )
    .unwrap();
    assert!(
        policy
            .with_proxies(vec!["0.0.0.0".parse().unwrap()])
            .is_err()
    );
}

#[tokio::test]
async fn proxy_and_header_boundaries_fail_closed() {
    let pool = fixture().await;
    let policy = RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "https://localhost:4200".into(),
    )
    .unwrap()
    .with_proxies(vec!["127.0.0.1".parse().unwrap()])
    .unwrap();
    let trusted = codexsymphony_server::router(pool.clone(), policy);
    for headers in [
        vec![],
        vec![
            ("x-forwarded-proto", "http"),
            ("x-forwarded-host", "localhost:4200"),
            ("x-forwarded-for", "192.0.2.1"),
        ],
        vec![
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "evil.test"),
            ("x-forwarded-for", "192.0.2.1"),
        ],
        vec![
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "localhost:4200"),
            ("x-forwarded-for", "192.0.2.1, 127.0.0.1"),
        ],
        vec![
            ("x-forwarded-proto", "https"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "localhost:4200"),
            ("x-forwarded-for", "192.0.2.1"),
        ],
    ] {
        assert_eq!(
            send(&trusted, "GET", "/api/health", None, &headers, json!({}))
                .await
                .status(),
            403
        );
    }
    let good = [
        ("x-forwarded-proto", "https"),
        ("x-forwarded-host", "localhost:4200"),
        ("x-forwarded-for", "192.0.2.1"),
    ];
    assert_eq!(
        send(&trusted, "GET", "/api/health", None, &good, json!({}))
            .await
            .status(),
        200
    );
    let direct = app(&pool, Arc::new(AtomicI64::new(1_800_000_000)));
    let anonymous = bootstrap(&direct).await;
    assert_eq!(
        login(&direct, &anonymous, &"x".repeat(129), "synthetic-password")
            .await
            .status(),
        401
    );
    assert_eq!(
        login(&direct, &anonymous, "operator", &"x".repeat(1025))
            .await
            .status(),
        401
    );
    for cookie in [
        "__Host-codexsession=bad",
        "other=value",
        "__Host-codexsession=gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
    ] {
        assert_eq!(
            send(
                &direct,
                "GET",
                "/api/drafts",
                None,
                &[("cookie", cookie)],
                json!({})
            )
            .await
            .status(),
            401
        );
    }
    for headers in [
        vec![("cookie", "__Host-codexsession=a; __Host-codexsession=b")],
        vec![("cookie", "other=a"), ("cookie", "other=b")],
        vec![("host", "evil.test")],
        vec![
            ("origin", "https://localhost:4200"),
            ("origin", "https://localhost:4200"),
        ],
    ] {
        assert_eq!(
            send(&direct, "GET", "/api/health", None, &headers, json!({}))
                .await
                .status(),
            403
        );
    }
    assert_eq!(
        send(
            &direct,
            "POST",
            "/api/auth/login",
            None,
            &[("origin", "https://localhost:4200")],
            json!({})
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        send(
            &direct,
            "GET",
            "http://evil.test/api/health",
            None,
            &[],
            json!({})
        )
        .await
        .status(),
        403
    );
    pool.close().await;
}

async fn business_snapshot(pool: &PgPool) -> Vec<Value> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT tablename::text FROM pg_tables WHERE schemaname=current_schema() AND tablename NOT LIKE 'platform_%' ORDER BY tablename")
        .fetch_all(pool).await.unwrap();
    let mut snapshot = vec![];
    for table in tables {
        let query = format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM \"{table}\" t"
        );
        snapshot.push(sqlx::query_scalar(&query).fetch_one(pool).await.unwrap());
    }
    snapshot
}
