//! Existing business suites use real account creation and HTTP login, never a server bypass.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Request, State},
    middleware::{self, Next},
    response::Response,
};
use codexsymphony_server::{auth_store, security::RequestPolicy};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use tokio::sync::Mutex;
use tower::ServiceExt;
#[derive(Clone)]
struct Client {
    app: Router,
    pool: PgPool,
    session: Arc<Mutex<Option<(String, String)>>>,
}
pub fn router(pool: PgPool, policy: RequestPolicy) -> Router {
    let app = codexsymphony_server::router(pool.clone(), policy).layer(axum::Extension(
        ConnectInfo("127.0.0.1:50000".parse::<std::net::SocketAddr>().unwrap()),
    ));
    let client = Client {
        app: app.clone(),
        pool,
        session: Arc::new(Mutex::new(None)),
    };
    app.layer(middleware::from_fn_with_state(client, authenticate))
}
type Session = (String, String);
static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
async fn authenticate(State(client): State<Client>, mut request: Request, next: Next) -> Response {
    if request.uri().path() == "/api/health" {
        return next.run(request).await;
    }
    let mut session = client.session.lock().await;
    if session.is_none() {
        let key: String =
            sqlx::query_scalar("SELECT current_database() || ':' || current_schema()")
                .fetch_one(&client.pool)
                .await
                .unwrap();
        let mut cached = SESSIONS.lock().await;
        if let Some(saved) = cached.get(&key) {
            let token = saved.0.split_once('=').unwrap().1;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            if auth_store::session(&client.pool, token, now)
                .await
                .unwrap()
                .flatten()
                .is_some()
            {
                *session = Some(saved.clone());
            }
        }
        if session.is_none() {
            let name = format!(
                "test-{}",
                codexsymphony_server::process::new_identity().unwrap()
            );
            auth_store::account(&client.pool, &name, "synthetic-test-password", false)
                .await
                .unwrap();
            let first = client
                .app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/auth/csrf")
                        .header("host", "127.0.0.1:3081")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(first.status(), 200);
            let cookie = first.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned();
            let proof: serde_json::Value =
                serde_json::from_slice(&to_bytes(first.into_body(), 8192).await.unwrap()).unwrap();
            let response = client
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("host", "127.0.0.1:3081")
                    .header("origin", "https://localhost:4200")
                    .header("cookie", cookie)
                    .header(
                        "x-codexsymphony-csrf",
                        proof["csrf_token"].as_str().unwrap(),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"username":name,"password":"synthetic-test-password"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 200);
            let cookie = response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned();
            let proof: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap())
                    .unwrap();
            let saved = (cookie, proof["csrf_token"].as_str().unwrap().into());
            cached.insert(key, saved.clone());
            *session = Some(saved);
        }
    }
    let (cookie, csrf) = session.as_ref().unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    if request
        .headers()
        .get("x-codexsymphony-csrf")
        .is_some_and(|v| v == "1")
    {
        request
            .headers_mut()
            .insert("x-codexsymphony-csrf", csrf.parse().unwrap());
    }
    drop(session);
    next.run(request).await
}
