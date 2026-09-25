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
    pub retry_after_seconds: Option<u64>,
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
        retry_after_seconds: None,
    }
}
fn transport(_: reqwest::Error) -> Error {
    Error {
        code: "github_transient_or_unknown",
        status: None,
        retry_after_seconds: None,
    }
}
struct Token {
    value: String,
    expires: i64,
    permissions: Value,
    policy: Option<Policy>,
}
pub struct AppClient {
    client: Client,
    api: Url,
    app_id: u64,
    key: EncodingKey,
    tokens: HashMap<u64, Token>,
}
impl AppClient {
    /// Fetch an exact observed object into the platform-owned bare repository.
    /// Authentication is process-local and never written into Git configuration.
    pub async fn fetch_commit(
        &mut self,
        policy: &Policy,
        repository: &std::path::Path,
        sha: &str,
        now: i64,
    ) -> Result<()> {
        if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        self.ensure_token(policy, now).await?;
        let authorization = encode_basic(&format!(
            "x-access-token:{}",
            self.tokens[&policy.repository_id].value
        ));
        let mut command = tokio::process::Command::new("git");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraHeader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Basic {authorization}"),
            )
            .env("GIT_CONFIG_KEY_1", "credential.helper")
            .env("GIT_CONFIG_VALUE_1", "")
            .arg("--git-dir")
            .arg(repository)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--",
            ])
            .arg(format!("https://github.com/{}.git", policy.repository))
            .arg(sha)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        configure_proxy(&mut command, std::env::vars_os());
        let status = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            command.status(),
        )
        .await
        {
            Ok(Ok(status)) => status,
            _ => return Err(invalid()),
        };
        if !status.success() {
            return Err(invalid());
        }
        Ok(())
    }
    pub(crate) async fn write(
        &mut self,
        policy: &Policy,
        method: Method,
        path: &str,
        body: Value,
        now: i64,
    ) -> Result<Value> {
        self.ensure_token(policy, now).await?;
        self.request(
            method,
            path,
            &self.tokens[&policy.repository_id].value,
            Some(body),
        )
        .await
    }
    /// Push an authorized candidate; credentials remain inside this App client.
    pub async fn push(
        &mut self,
        policy: &Policy,
        repository: &std::path::Path,
        head: &str,
        branch: &str,
        now: i64,
    ) -> Result<Value> {
        self.push_conditional(policy, repository, head, branch, None, now)
            .await
    }
    pub async fn push_conditional(
        &mut self,
        policy: &Policy,
        repository: &std::path::Path,
        head: &str,
        branch: &str,
        expected: Option<&str>,
        now: i64,
    ) -> Result<Value> {
        if let Some(expected) = expected {
            let status = tokio::process::Command::new("git")
                .arg("--git-dir")
                .arg(repository)
                .args([
                    "merge-base",
                    "--is-ancestor",
                    "--end-of-options",
                    expected,
                    head,
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .or(Err(invalid()))?;
            if !status.success() {
                return Err(invalid());
            }
        }
        self.ensure_token(policy, now).await?;
        let command = push_command(
            policy,
            repository,
            head,
            branch,
            &self.tokens[&policy.repository_id].value,
            expected,
        );
        push_git(command, head).await
    }
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
                code: response_code(status.as_u16()),
                status: Some(status.as_u16()),
                retry_after_seconds: response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_retry_after),
            });
        }
        if status.as_u16() == 204 || status.as_u16() == 202 {
            return Ok(Value::Null);
        }
        response.json().await.map_err(transport)
    }
    /// Verify a bounded real log read. Signed URLs and log contents are never
    /// persisted, and App credentials are never forwarded to download storage.
    pub async fn logs_readable(&mut self, policy: &Policy, job: u64, now: i64) -> Result<Value> {
        let response = self.job_log_response(policy, job, now).await?;
        log_digest(response, job).await
    }
    pub async fn job_log(&mut self, policy: &Policy, job: u64, now: i64) -> Result<String> {
        let mut response = self.job_log_response(policy, job, now).await?;
        if !response.status().is_success() {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            if bytes.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
                return Err(invalid());
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| invalid())
    }
    async fn job_log_response(
        &mut self,
        policy: &Policy,
        job: u64,
        now: i64,
    ) -> Result<reqwest::Response> {
        self.ensure_token(policy, now).await?;
        let path = format!("/repos/{}/actions/jobs/{job}/logs", policy.repository);
        let url = self.api.join(&path).or(Err(invalid()))?;
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.tokens[&policy.repository_id].value)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "CodexSymphony-control-plane")
            .send()
            .await
            .map_err(transport)?;
        if response.status().as_u16() != 302 || !response.headers().contains_key("location") {
            return Err(Error {
                code: "github_logs_unavailable",
                status: Some(response.status().as_u16()),
                retry_after_seconds: None,
            });
        }
        let location = response.headers()["location"].to_str().or(Err(invalid()))?;
        self.download_log(location).await
    }
    async fn download_log(&self, location: &str) -> Result<reqwest::Response> {
        let url = Url::parse(location).or(Err(invalid()))?;
        if !log_origin(&self.api, &url) {
            return Err(invalid());
        }
        self.client.get(url).send().await.map_err(transport)
    }
    pub async fn permissions(&mut self, policy: &Policy, now: i64) -> Result<Value> {
        if policy.delivery.is_some() {
            self.tokens.remove(&policy.repository_id);
        }
        self.ensure_token(policy, now).await?;
        Ok(self.tokens[&policy.repository_id].permissions.clone())
    }
    async fn ensure_token(&mut self, policy: &Policy, now: i64) -> Result<()> {
        if !crate::github_credentials::separate_check_identity(policy, self.app_id) {
            return Err(invalid());
        }

        if self
            .tokens
            .get(&policy.repository_id)
            .is_some_and(|token| token.expires > now + 60 && token.policy.as_ref() == Some(policy))
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
            Some(json!({"repository_ids":[policy.repository_id],"permissions":crate::github_contract::permissions(policy)}))).await?;
        let mut token = decode_token(&grant, now)?;
        token.policy = Some(policy.clone());
        self.tokens.insert(policy.repository_id, token);
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

            retry_after_seconds: None,
        })
    }
}

pub fn encode_basic(value: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in value.as_bytes().chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    output
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
        policy: None,
    })
}

fn page_values<'a>(response: &'a Value, field: Option<&str>) -> Result<&'a Vec<Value>> {
    // GitHub caps filtered workflow-run searches at 1000. Never call a capped
    // search a complete snapshot, including the boundary where more may exist.
    if field == Some("workflow_runs") && response["total_count"].as_u64().unwrap_or(0) >= 1000 {
        return Err(Error {
            code: "github_pagination_incomplete",
            status: None,

            retry_after_seconds: None,
        });
    }
    let values = match field {
        Some(field) => &response[field],
        None => response,
    };
    values.as_array().ok_or_else(invalid)
}

fn push_command(
    policy: &Policy,
    repository: &std::path::Path,
    head: &str,
    branch: &str,
    token: &str,
    expected: Option<&str>,
) -> tokio::process::Command {
    let authorization = encode_basic(&format!("x-access-token:{token}"));
    let mut command = tokio::process::Command::new("git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Basic {authorization}"),
        )
        .env("GIT_CONFIG_KEY_1", "credential.helper")
        .env("GIT_CONFIG_VALUE_1", "")
        .arg("--git-dir")
        .arg(repository)
        .args(["push", "--porcelain", "--no-verify"]);
    if let Some(expected) = expected {
        command.arg(format!("--force-with-lease=refs/heads/{branch}:{expected}"));
    }
    command
        .arg("--")
        .arg(format!("https://github.com/{}.git", policy.repository))
        .arg(format!("{head}:refs/heads/{branch}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    configure_proxy(&mut command, std::env::vars_os());
    command
}

/// Only operator service proxy routing crosses the cleared Git environment.
/// Git configuration, credential helpers and unrelated secrets stay excluded.
pub fn configure_proxy(
    command: &mut tokio::process::Command,
    environment: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) {
    command.envs(environment.filter(|(name, _)| {
        matches!(
            name.to_str(),
            Some(
                "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "ALL_PROXY"
                    | "NO_PROXY"
                    | "http_proxy"
                    | "https_proxy"
                    | "all_proxy"
                    | "no_proxy"
            )
        )
    }));
}
/// Execute an already platform-authorized Git push. The App adapter builds the
/// fixed origin and non-forcing refspec; local fixtures exercise real Git here.
pub async fn push_git(mut command: tokio::process::Command, head: &str) -> Result<Value> {
    let status = tokio::time::timeout(std::time::Duration::from_secs(30), command.status())
        .await
        .or(Err(Error {
            code: "github_transient_or_unknown",
            status: None,

            retry_after_seconds: None,
        }))?
        .or(Err(invalid()))?;
    if !status.success() {
        return Err(Error {
            code: "github_transient_or_unknown",
            status: None,

            retry_after_seconds: None,
        });
    }
    Ok(json!({"push":"accepted","head":head}))
}

fn log_origin(api: &Url, url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if api.host_str() == Some("127.0.0.1") && api.origin() == url.origin() {
        return true;
    }
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            [
                ".blob.core.windows.net",
                ".githubusercontent.com",
                ".amazonaws.com",
            ]
            .iter()
            .any(|suffix| host.ends_with(suffix))
        })
}
async fn log_digest(mut response: reqwest::Response, job: u64) -> Result<Value> {
    use sha2::{Digest, Sha256};
    if !response.status().is_success() {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    let mut bytes = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        bytes = bytes.saturating_add(chunk.len());
        if bytes > 8 * 1024 * 1024 {
            return Err(invalid());
        }
        digest.update(&chunk);
    }
    if bytes == 0 {
        return Err(invalid());
    }
    Ok(json!({"job_id":job,"bytes":bytes,"sha256":format!("{:x}",digest.finalize())}))
}

fn parse_retry_after(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.parse() {
        return Some(seconds);
    }
    let deadline = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .timestamp();
    Some(deadline.saturating_sub(crate::github_service::now()).max(0) as u64)
}

fn response_code(status: u16) -> &'static str {
    match status {
        429 => "rate_limited",
        401 | 403 => "permission_denied",
        500..=599 => "service_unavailable",
        _ => "github_transient_or_unknown",
    }
}
