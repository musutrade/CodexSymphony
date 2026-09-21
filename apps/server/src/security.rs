//! One fail-closed boundary around every route, including future business APIs.
use crate::{auth, auth_store};
use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::Response,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
pub const CSRF_HEADER: &str = "x-codexsymphony-csrf";
#[derive(Clone)]
pub struct RequestPolicy {
    authority: String,
    origin: String,
    proxies: Vec<IpAddr>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    public_origin: String,
    trusted_proxies: Vec<IpAddr>,
}
#[derive(Clone)]
pub struct Identity {
    pub token: Option<String>,
    pub username: Option<String>,
    pub source: String,
}
impl RequestPolicy {
    pub fn new(address: SocketAddr, origin: String) -> Result<Self, &'static str> {
        crate::config::loopback_address(&address.to_string())?;
        validate_web_origin(&origin)?;
        Ok(Self {
            authority: address.to_string(),
            origin,
            proxies: Vec::new(),
            clock: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            }),
        })
    }
    pub fn configured(address: SocketAddr, origin: String) -> Result<Self, &'static str> {
        let path = std::env::var("AUTH_CONFIG").or(Err("AUTH_CONFIG is required"))?;
        let settings: Settings =
            serde_json::from_slice(&std::fs::read(path).or(Err("cannot read AUTH_CONFIG"))?)
                .or(Err("invalid AUTH_CONFIG"))?;
        if settings.public_origin != origin {
            return Err("WEB_ORIGIN must match AUTH_CONFIG public_origin");
        }
        Self::new(address, origin)?.with_proxies(settings.trusted_proxies)
    }
    pub fn with_proxies(mut self, proxies: Vec<IpAddr>) -> Result<Self, &'static str> {
        if proxies
            .iter()
            .any(|ip| ip.is_unspecified() || ip.is_multicast())
        {
            return Err("trusted proxies must be explicit unicast addresses");
        }
        self.proxies = proxies;
        Ok(self)
    }
    /// Dependency injection for tests; no environment/configuration clock or auth bypass.
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }
    pub fn now(&self) -> i64 {
        (self.clock)()
    }
    pub fn protect(&self, router: Router, pool: PgPool) -> Router {
        router.layer(middleware::from_fn_with_state(
            (self.clone(), pool),
            enforce,
        ))
    }
    fn source(&self, request: &Request) -> Result<String, StatusCode> {
        self.verify_host(request)?;
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or(StatusCode::FORBIDDEN)?
            .0
            .ip();
        if !self.proxies.contains(&peer) {
            return Ok(peer.to_string());
        }
        self.forwarded_source(request.headers())
    }
    fn verify_host(&self, request: &Request) -> Result<(), StatusCode> {
        require(single(request.headers(), "host")? == Some(&self.authority))?;
        if let Some(authority) = request.uri().authority() {
            require(authority.as_str() == self.authority)?;
        }
        Ok(())
    }
    fn forwarded_source(&self, headers: &HeaderMap) -> Result<String, StatusCode> {
        require(single(headers, "x-forwarded-proto")? == Some("https"))?;
        let origin: Uri = self.origin.parse().or(Err(StatusCode::FORBIDDEN))?;
        require(single(headers, "x-forwarded-host")? == origin.authority().map(|a| a.as_str()))?;
        let source: IpAddr = single(headers, "x-forwarded-for")?
            .ok_or(StatusCode::FORBIDDEN)?
            .parse()
            .or(Err(StatusCode::FORBIDDEN))?;
        Ok(source.to_string())
    }
}
pub fn validate_web_origin(value: &str) -> Result<(), &'static str> {
    let uri: Uri = value
        .parse()
        .or(Err("WEB_ORIGIN must be an HTTPS origin"))?;
    let authority = uri.authority().ok_or("WEB_ORIGIN requires authority")?;
    if value != format!("https://{authority}")
        || authority.as_str().contains('@')
        || authority.port_u16() == Some(0)
    {
        return Err("WEB_ORIGIN must be an HTTPS origin without credentials/path/query/fragment");
    }
    if authority.as_str() != authority.host() && authority.port_u16().is_none() {
        return Err("invalid origin port");
    }
    Ok(())
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    require(values.next().is_none())?;
    first
        .map(|v| v.to_str().or(Err(StatusCode::FORBIDDEN)))
        .transpose()
}
fn token(headers: &HeaderMap) -> Result<Option<String>, StatusCode> {
    let Some(cookie) = single(headers, "cookie")? else {
        return Ok(None);
    };
    let mut tokens = cookie
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| *name == auth::COOKIE);
    let first = tokens.next();
    require(tokens.next().is_none())?;
    let Some((_, value)) = first else {
        return Ok(None);
    };
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    Ok(Some(value.to_owned()))
}
fn require(allowed: bool) -> Result<(), StatusCode> {
    allowed.then_some(()).ok_or(StatusCode::FORBIDDEN)
}
async fn enforce(
    State((policy, pool)): State<(RequestPolicy, PgPool)>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let source = policy.source(&request)?;
    let public = is_public(&request);
    let candidate = token(request.headers())?;
    let stored = match &candidate {
        Some(token) => auth_store::session(&pool, token, policy.now())
            .await
            .or(Err(StatusCode::SERVICE_UNAVAILABLE))?,
        None => None,
    };
    let identity = Identity {
        token: stored.as_ref().and(candidate),
        username: stored.flatten(),
        source,
    };
    if !public && identity.username.is_none() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    verify_csrf(&policy, &request, &identity)?;
    request.extensions_mut().insert(identity);
    request.extensions_mut().insert(policy);
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    Ok(response)
}

fn is_public(request: &Request) -> bool {
    matches!(
        (request.method(), request.uri().path()),
        (&Method::GET, "/api/health" | "/api/auth/csrf") | (&Method::POST, "/api/auth/login")
    )
}
fn verify_csrf(
    policy: &RequestPolicy,
    request: &Request,
    identity: &Identity,
) -> Result<(), StatusCode> {
    let origin = single(request.headers(), "origin")?;
    if let Some(origin) = origin {
        require(origin == policy.origin)?;
    }
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        require(origin == Some(&policy.origin))?;
        let token = identity.token.as_ref().ok_or(StatusCode::FORBIDDEN)?;
        require(single(request.headers(), CSRF_HEADER)? == Some(auth::csrf(token).as_str()))?;
    }
    Ok(())
}
