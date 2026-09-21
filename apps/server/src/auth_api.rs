use crate::{
    auth, auth_store,
    security::{Identity, RequestPolicy},
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, header::SET_COOKIE},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/auth/csrf", get(csrf))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/session", get(session))
        .layer(DefaultBodyLimit::max(8192))
}
fn response(token: &str, age: i64, username: Option<&str>) -> Response {
    (
        [(SET_COOKIE, auth::cookie(token, age))],
        Json(json!({"csrf_token":auth::csrf(token),"username":username})),
    )
        .into_response()
}
async fn csrf(
    State(pool): State<PgPool>,
    Extension(identity): Extension<Identity>,
    Extension(policy): Extension<RequestPolicy>,
) -> Result<Response, StatusCode> {
    if let Some(token) = identity.token {
        return Ok(
            Json(json!({"csrf_token":auth::csrf(&token),"username":identity.username}))
                .into_response(),
        );
    }
    let token = auth_store::issue(&pool, policy.now())
        .await
        .or(Err(StatusCode::SERVICE_UNAVAILABLE))?;
    Ok(response(&token, 600, None))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    username: String,
    password: String,
}
async fn login(
    State(pool): State<PgPool>,
    Extension(identity): Extension<Identity>,
    Extension(policy): Extension<RequestPolicy>,
    Json(body): Json<Credentials>,
) -> Result<Response, StatusCode> {
    if body.username.len() > 128 || body.password.len() > 1024 {
        return Ok(failure(StatusCode::UNAUTHORIZED));
    }
    let result = auth_store::login(
        &pool,
        &body.username,
        &body.password,
        &identity.source,
        identity.token.as_deref().ok_or(StatusCode::FORBIDDEN)?,
        policy.now(),
    )
    .await
    .or(Err(StatusCode::SERVICE_UNAVAILABLE))?;
    Ok(match result {
        auth_store::Login::Success(token) => {
            response(&token, auth::SESSION_SECONDS, Some(&body.username))
        }
        auth_store::Login::Invalid => failure(StatusCode::UNAUTHORIZED),
        auth_store::Login::Limited => failure(StatusCode::TOO_MANY_REQUESTS),
    })
}
fn failure(status: StatusCode) -> Response {
    (status, Json(json!({"message":"用户名或密码错误"}))).into_response()
}
async fn session(Extension(identity): Extension<Identity>) -> Json<serde_json::Value> {
    Json(
        json!({"username":identity.username,"csrf_token":auth::csrf(identity.token.as_deref().unwrap_or_default())}),
    )
}
async fn logout(
    State(pool): State<PgPool>,
    Extension(identity): Extension<Identity>,
) -> Result<Response, StatusCode> {
    auth_store::revoke(
        &pool,
        identity.token.as_deref().ok_or(StatusCode::UNAUTHORIZED)?,
    )
    .await
    .or(Err(StatusCode::SERVICE_UNAVAILABLE))?;
    Ok(([(SET_COOKIE, auth::cookie("", 0))], StatusCode::NO_CONTENT).into_response())
}
