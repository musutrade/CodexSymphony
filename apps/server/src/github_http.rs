//! Control-plane App credentials; no token serialization, Debug, or Agent tools.
use crate::github::Policy;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, Method, Url};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub status: Option<u16>,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{} (HTTP {:?})", self.code, self.status))
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
pub fn invalid() -> Error {
    Error {
        code: "github_identity_conflict",
        status: None,
    }
}
fn transport(_: reqwest::Error) -> Error {
    Error {
        code: "github_transient_or_unknown",
        status: None,
    }
}
struct Token {
    value: String,
    expires: i64,
    permissions: Value,
}
pub struct AppClient {
    client: Client,
    api: Url,
    app_id: u64,
    key: EncodingKey,
    tokens: HashMap<u64, Token>,
}
impl AppClient {
    /// api.github.com in production; loopback HTTP solely for deterministic fixtures.
    pub fn new(api: &str, app_id: u64, pem: &[u8]) -> Result<Self> {
        let api = Url::parse(api).or(Err(invalid()))?;
        let allowed =
            api.as_str() == "https://api.github.com/" || api.host_str() == Some("127.0.0.1");
        if !allowed || app_id == 0 {
            return Err(invalid());
        }
        let key = EncodingKey::from_rsa_pem(pem).or(Err(invalid()))?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(transport)?;
        Ok(Self {
            client,
            api,
            app_id,
            key,
            tokens: HashMap::new(),
        })
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = self.api.join(path).or(Err(invalid()))?;
        if url.origin() != self.api.origin() {
            return Err(invalid());
        }
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "CodexSymphony-control-plane");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error {
                code: "github_transient_or_unknown",
                status: Some(status.as_u16()),
            });
        }
        response.json().await.map_err(transport)
    }
    pub async fn permissions(&mut self, policy: &Policy, now: i64) -> Result<Value> {
        self.ensure_token(policy, now).await?;
        Ok(self.tokens[&policy.repository_id].permissions.clone())
    }
    async fn ensure_token(&mut self, policy: &Policy, now: i64) -> Result<()> {
        if self
            .tokens
            .get(&policy.repository_id)
            .is_some_and(|token| token.expires > now + 60)
        {
            return Ok(());
        }
        let jwt = jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &json!({"iat":now-60,"exp":now+300,"iss":self.app_id.to_string()}),
            &self.key,
        )
        .or(Err(invalid()))?;
        let installation = self
            .request(
                Method::GET,
                &format!("/repos/{}/installation", policy.repository),
                &jwt,
                None,
            )
            .await?;
        if installation["app_id"] != self.app_id {
            return Err(invalid());
        }
        let id = installation["id"].as_u64().ok_or_else(invalid)?;
        let grant = self.request(Method::POST, &format!("/app/installations/{id}/access_tokens"), &jwt,
            Some(json!({"repository_ids":[policy.repository_id],"permissions":{"contents":"write","pull_requests":"write","checks":"read","actions":"read"}}))).await?;
        self.tokens
            .insert(policy.repository_id, decode_token(&grant, now)?);
        Ok(())
    }
    pub async fn get(&mut self, policy: &Policy, path: &str, now: i64) -> Result<Value> {
        self.ensure_token(policy, now).await?;
        let first = self
            .request(
                Method::GET,
                path,
                &self.tokens[&policy.repository_id].value,
                None,
            )
            .await;
        if !matches!(
            &first,
            Err(Error {
                status: Some(401),
                ..
            })
        ) {
            return first;
        }
        self.tokens.remove(&policy.repository_id);
        self.ensure_token(policy, now).await?;
        self.request(
            Method::GET,
            path,
            &self.tokens[&policy.repository_id].value,
            None,
        )
        .await
    }
    /// Numbered pages continue to an empty page, independent of API `latest` or
    /// combined pending. A bounded overflow fails the whole snapshot closed.
    pub async fn pages(
        &mut self,
        policy: &Policy,
        path: &str,
        field: Option<&str>,
        now: i64,
    ) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        let separator = if path.contains('?') { '&' } else { '?' };
        for page in 1..=1000 {
            let response = self
                .get(
                    policy,
                    &format!("{path}{separator}per_page=100&page={page}"),
                    now,
                )
                .await?;
            let values = page_values(&response, field)?;
            if values.is_empty() {
                return Ok(all);
            }
            all.extend(values.iter().cloned());
        }
        Err(Error {
            code: "github_pagination_incomplete",
            status: None,
        })
    }
}

fn decode_token(grant: &Value, now: i64) -> Result<Token> {
    let expires =
        chrono::DateTime::parse_from_rfc3339(grant["expires_at"].as_str().ok_or_else(invalid)?)
            .or(Err(invalid()))?
            .timestamp();
    if expires <= now + 60 {
        return Err(invalid());
    }
    let value = grant["token"].as_str().ok_or_else(invalid)?.to_owned();
    Ok(Token {
        value,
        expires: expires.min(now + 300),
        permissions: grant["permissions"].clone(),
    })
}

fn page_values<'a>(response: &'a Value, field: Option<&str>) -> Result<&'a Vec<Value>> {
    // GitHub caps filtered workflow-run searches at 1000. Never call a capped
    // search a complete snapshot, including the boundary where more may exist.
    if field == Some("workflow_runs") && response["total_count"].as_u64().unwrap_or(0) >= 1000 {
        return Err(Error {
            code: "github_pagination_incomplete",
            status: None,
        });
    }
    let values = match field {
        Some(field) => &response[field],
        None => response,
    };
    values.as_array().ok_or_else(invalid)
}
