//! Local browser boundary, not authentication. Never enable permissive CORS here.
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::Response,
};
use std::net::SocketAddr;

pub const CSRF_HEADER: &str = "x-codexsymphony-csrf";
pub const CSRF_VALUE: &str = "1";

#[derive(Clone)]
pub struct RequestPolicy {
    authority: String,
    origins: [String; 2],
}

impl RequestPolicy {
    /// Use the actual listener address (including its assigned port).
    pub fn new(address: SocketAddr, web_origin: String) -> Result<Self, &'static str> {
        crate::config::loopback_address(&address.to_string())?;
        validate_web_origin(&web_origin)?;
        Ok(Self {
            authority: address.to_string(),
            origins: [["http://", &address.to_string()].concat(), web_origin],
        })
    }

    /// Apply after registering all routes, including every future business write route.
    pub fn protect(&self, router: Router) -> Router {
        router.layer(middleware::from_fn_with_state(self.clone(), enforce))
    }

    fn check(&self, request: &Request) -> Result<(), StatusCode> {
        self.check_host(request)?;
        let headers = request.headers();
        let origin = self.check_origin(headers)?;
        if !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        ) {
            check_csrf(headers, origin)?;
        }
        Ok(())
    }

    fn check_host(&self, request: &Request) -> Result<(), StatusCode> {
        require(single_header(request.headers(), "host")? == Some(self.authority.as_str()))?;
        // Absolute-form targets must agree with Host. Forwarded headers are ignored:
        // Phase 0a has no trusted remote reverse proxy.
        if let Some(authority) = request.uri().authority() {
            require(authority.as_str() == self.authority)?;
        }
        Ok(())
    }

    fn check_origin<'a>(&self, headers: &'a HeaderMap) -> Result<Option<&'a str>, StatusCode> {
        let origin = single_header(headers, "origin")?;
        if let Some(origin) = origin {
            require(self.origins[0] == origin || self.origins[1] == origin)?;
        }
        Ok(origin)
    }
}

pub fn validate_web_origin(value: &str) -> Result<(), &'static str> {
    let uri: Uri = value
        .parse()
        .or(Err("WEB_ORIGIN must be an HTTP loopback origin"))?;
    let authority = uri.authority().ok_or("WEB_ORIGIN requires an authority")?;
    if value != ["http://", authority.as_str()].concat() {
        return Err("WEB_ORIGIN must be an HTTP origin without a path, query or fragment");
    }
    validate_origin_authority(authority)
}

fn validate_origin_authority(authority: &axum::http::uri::Authority) -> Result<(), &'static str> {
    let host = authority.host();
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if !local || authority.port_u16() == Some(0) || authority.as_str().contains('@') {
        return Err("WEB_ORIGIN must be an HTTP localhost/127.0.0.1/[::1] origin without a path");
    }
    // Parsing as an HTTP URI does not reject every invalid port spelling.
    if authority.as_str() != host && authority.port_u16().is_none() {
        return Err("WEB_ORIGIN port must be between 1 and 65535");
    }
    Ok(())
}

fn check_csrf(headers: &HeaderMap, origin: Option<&str>) -> Result<(), StatusCode> {
    require(origin.is_some())?;
    require(single_header(headers, CSRF_HEADER)? == Some(CSRF_VALUE))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    require(values.next().is_none())?;
    match value {
        Some(value) => value.to_str().map(Some).or(Err(StatusCode::FORBIDDEN)),
        None => Ok(None),
    }
}

fn require(allowed: bool) -> Result<(), StatusCode> {
    allowed.then_some(()).ok_or(StatusCode::FORBIDDEN)
}

async fn enforce(
    State(policy): State<RequestPolicy>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    policy.check(&request)?;
    Ok(next.run(request).await)
}
