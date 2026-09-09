//! S3 spike: validate Cloudflare Access JWT (`Cf-Access-Jwt-Assertion`) in Axum.
//!
//! Env:
//!   ACCESS_TEAM_DOMAIN  e.g. higoalzm.cloudflareaccess.com
//!   ACCESS_AUD          application audience tag (64 hex)
//!   ACCESS_ALLOWED_EMAIL  optional; if set, `email` claim must match exactly
//!   BIND                default 127.0.0.1:8790
//!
//! Rules (mirror of plan §17.4): verify RS256 signature against JWKS fetched from the fixed
//! team domain, pin `iss` to https://<team>, require `aud` contains ACCESS_AUD, reject expired,
//! never trust `Cf-Access-Authenticated-User-Email` on its own. JWKS cached by kid, refetched
//! once on unknown kid.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::RwLock;

#[derive(Clone)]
struct AppState {
    team: String,
    aud: String,
    allowed_email: Option<String>,
    http: reqwest::Client,
    jwks: Arc<RwLock<(Instant, HashMap<String, DecodingKey>)>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    aud: serde_json::Value, // Access sends an array
    iss: String,
    sub: String,
    exp: u64,
    iat: Option<u64>,
    nbf: Option<u64>,
    email: Option<String>,
    #[serde(rename = "type")]
    typ: Option<String>,
    identity_nonce: Option<String>,
    country: Option<String>,
    #[serde(flatten)]
    rest: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct Denied {
    error: &'static str,
    detail: String,
}

fn deny(code: &'static str, detail: impl Into<String>) -> (StatusCode, Json<Denied>) {
    let detail = detail.into();
    tracing::warn!(code, %detail, "denied");
    (StatusCode::UNAUTHORIZED, Json(Denied { error: code, detail }))
}

async fn refresh_jwks(st: &AppState) -> Result<(), String> {
    let url = format!("https://{}/cdn-cgi/access/certs", st.team);
    let set: JwkSet = st
        .http
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let mut map = HashMap::new();
    for k in set.keys {
        if let (Some(kid), Ok(dk)) = (k.common.key_id.clone(), DecodingKey::from_jwk(&k)) {
            map.insert(kid, dk);
        }
    }
    tracing::info!(n = map.len(), "jwks refreshed");
    *st.jwks.write().await = (Instant::now(), map);
    Ok(())
}

async fn key_for(st: &AppState, kid: &str) -> Option<DecodingKey> {
    if let Some(k) = st.jwks.read().await.1.get(kid) {
        return Some(k.clone());
    }
    // unknown kid: refetch once (rate-limited to once per 30s)
    if st.jwks.read().await.0.elapsed().as_secs() >= 30 || st.jwks.read().await.1.is_empty() {
        let _ = refresh_jwks(st).await;
    }
    st.jwks.read().await.1.get(kid).cloned()
}

async fn verify(st: &AppState, headers: &HeaderMap) -> Result<Claims, (StatusCode, Json<Denied>)> {
    let token = headers
        .get("cf-access-jwt-assertion")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| deny("missing_assertion", "no Cf-Access-Jwt-Assertion header"))?;
    let header = decode_header(token).map_err(|e| deny("bad_header", e.to_string()))?;
    if header.alg != Algorithm::RS256 {
        return Err(deny("bad_alg", format!("{:?}", header.alg)));
    }
    let kid = header.kid.ok_or_else(|| deny("no_kid", ""))?;
    let key = key_for(st, &kid).await.ok_or_else(|| deny("unknown_kid", kid.clone()))?;

    let mut v = Validation::new(Algorithm::RS256);
    v.set_issuer(&[format!("https://{}", st.team)]);
    v.set_audience(&[st.aud.clone()]);
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    v.leeway = 30;
    let data = decode::<Claims>(token, &key, &v).map_err(|e| deny("invalid_token", e.to_string()))?;
    let c = data.claims;
    if let Some(want) = &st.allowed_email {
        if c.email.as_deref() != Some(want.as_str()) {
            return Err(deny("email_not_allowed", c.email.clone().unwrap_or_default()));
        }
    }
    Ok(c)
}

async fn whoami(State(st): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    match verify(&st, &headers).await {
        Ok(c) => {
            // Compare with the informational header (must never be trusted alone).
            let hdr_email = headers
                .get("cf-access-authenticated-user-email")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let via_tunnel = headers.contains_key("cf-connecting-ip");
            tracing::info!(sub = %c.sub, email = ?c.email, exp = c.exp, "accepted");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "hello": c.email,
                    "claims": c,
                    "header_email_matches_claim": hdr_email.as_deref() == c.email.as_deref(),
                    "via_cloudflare": via_tunnel,
                })),
            )
                .into_response()
        }
        Err(e) => e.into_response(),
    }
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let st = AppState {
        team: std::env::var("ACCESS_TEAM_DOMAIN").expect("ACCESS_TEAM_DOMAIN"),
        aud: std::env::var("ACCESS_AUD").expect("ACCESS_AUD"),
        allowed_email: std::env::var("ACCESS_ALLOWED_EMAIL").ok(),
        http: reqwest::Client::builder().build().unwrap(),
        jwks: Arc::new(RwLock::new((Instant::now(), HashMap::new()))),
    };
    refresh_jwks(&st).await.expect("initial jwks fetch");
    let app = Router::new()
        .route("/", get(whoami))
        .route("/api/whoami", get(whoami))
        .route("/healthz", get(health))
        .with_state(st);
    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:8790".into());
    tracing::info!(%bind, "listening");
    let l = tokio::net::TcpListener::bind(&bind).await.unwrap();
    axum::serve(l, app).await.unwrap();
}
