use axum::{body::Body, http::Request};
use codexsymphony_server::recovery;
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tower::ServiceExt;

#[tokio::test]
async fn restored_database_is_guarded_and_drill_has_no_business_routes() {
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let options: PgConnectOptions = url.parse().unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    let name = format!(
        "symphony_restore_{}",
        codexsymphony_server::process::new_identity()
            .unwrap()
            .replace('-', "")
    );
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let isolated = options.database(&name);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(isolated.clone())
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    assert!(!recovery::guarded(&pool).await.unwrap());
    recovery::refuse_normal_start(&pool).await.unwrap();
    let drill_url = isolated.to_url_lossy().to_string();
    assert!(
        recovery::serve(&drill_url, "127.0.0.1:0".parse().unwrap())
            .await
            .is_err()
    );
    assert!(
        recovery::serve(&drill_url, "0.0.0.0:0".parse().unwrap())
            .await
            .is_err()
    );
    sqlx::query("CREATE TABLE symphony_recovery_guard(id integer)")
        .execute(&pool)
        .await
        .unwrap();
    assert!(recovery::refuse_normal_start(&pool).await.is_err());
    let app = recovery::router(pool.clone());
    for (method, path, expected) in [
        ("GET", "/api/recovery", 200),
        ("POST", "/api/recovery", 405),
        ("POST", "/api/requirements", 404),
        ("POST", "/api/auth/login", 404),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
    }
    // Same auth-store predicate used by production: backup restoration must not
    // turn either a revoked or expired token into an authenticated session.
    let now = 1_000;
    let token = codexsymphony_server::auth_store::issue(&pool, now)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::auth_store::session(&pool, &token, now)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        codexsymphony_server::auth_store::session(&pool, &token, now + 601)
            .await
            .unwrap()
            .is_none()
    );
    codexsymphony_server::auth_store::revoke(&pool, &token)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::auth_store::session(&pool, &token, now)
            .await
            .unwrap()
            .is_none()
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    // Exercise the real CLI branch and graceful signal handling, not task abort.
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .arg("--recovery-drill")
        .env("DATABASE_URL", &drill_url)
        .env("BIND_ADDRESS", address.to_string())
        .env("WEB_ORIGIN", "https://localhost:4200")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = reqwest::Client::new();
    let endpoint = format!("http://{address}/api/recovery");
    let mut reached = false;
    for _ in 0..100 {
        if let Ok(response) = client.get(&endpoint).send().await {
            assert_eq!(response.status().as_u16(), 200);
            reached = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(reached);
    let signal = std::process::Command::new("/bin/kill")
        .args(["-INT", &child.id().unwrap().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .expect("recovery drill must shut down gracefully")
        .unwrap();
    assert!(status.success());
    assert!(client.get(&endpoint).send().await.is_err());

    // A restored database must also be rejected through the normal CLI startup.
    let refused = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
            .env("DATABASE_URL", &drill_url)
            .env("BIND_ADDRESS", address.to_string())
            .env("WEB_ORIGIN", "https://localhost:4200")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("normal startup must promptly reject a restored database")
    .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("normal startup forbidden"));
    pool.close().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/recovery")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 503);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}
