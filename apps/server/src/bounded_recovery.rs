//! Bounded recovery decisions from observed facts, never model diagnoses.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Code,
    Infrastructure,
    Configuration,
    Security,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Failure {
    pub phase: String,
    pub step: String,
    pub candidate_sha: String,
    pub pr_head: Option<String>,
    pub input_identity: String,
    pub environment_identity: String,
    pub command: Vec<String>,
    pub log_ref: String,
    pub raw: String,
    pub exit_code: Option<i32>,
    /// A trusted adapter's native error code. Text and model advice cannot set it.
    pub native_code: String,
    pub authorized_code_check: bool,
    pub retry_after_seconds: Option<u64>,
}

impl Failure {
    pub fn classify(&self) -> Class {
        match self.native_code.as_str() {
            "authorization_revoked" | "configuration_invalid" | "permission_denied" => {
                Class::Configuration
            }
            "security_failure" | "policy_identity_mismatch" => Class::Security,
            "rate_limited" | "service_unavailable" | "runner_unavailable" | "transport_timeout" => {
                Class::Infrastructure
            }
            "check_exit" if self.code_evidence() => Class::Code,
            _ => Class::Unknown,
        }
    }

    fn code_evidence(&self) -> bool {
        self.authorized_code_check
            && self
                .exit_code
                .map_or(self.pr_head.is_some(), |code| code != 0)
            && !self.raw.is_empty()
            && !self.log_ref.is_empty()
            && !self.command.is_empty()
            && !self.candidate_sha.is_empty()
    }

    pub fn fingerprint(&self) -> String {
        // Exclude timestamps and transport event IDs. A new observation of the
        // same failed input is not progress. Preserve the original log separately.
        crate::validation::sha256(
            serde_json::json!([
                self.phase,
                self.step,
                self.candidate_sha,
                self.input_identity,
                self.environment_identity,
                self.command,
                self.native_code,
                self.exit_code,
                self.raw
            ])
            .to_string(),
        )
    }
}

/// Infrastructure has two retries and a ten-minute phase deadline. A server's
/// Retry-After can delay or prevent admission; it can never shorten the backoff.
pub fn next_retry(now: i64, deadline: i64, retries: u32, retry_after: Option<u64>) -> Option<i64> {
    let delay = match retries {
        0 => 30,
        1 => 120,
        _ => return None,
    };
    let delay = u64::max(delay, retry_after.unwrap_or(0));
    let next = now.checked_add(i64::try_from(delay).ok()?)?;
    (next <= deadline).then_some(next)
}

pub fn repair_limit(policy: &str) -> i64 {
    match policy {
        "bounded_v1" => 3,
        _ => 1,
    }
}

/// Conservative native diagnostics. A failed service-dependent test is not a
/// code diagnosis. Unrecognized output remains unknown, even with a nonzero exit.
pub fn native_failure(raw: &str) -> &'static str {
    let lower = raw.to_ascii_lowercase();
    if [
        "permission denied",
        "invalid configuration",
        "unauthorized",
        "forbidden",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return "permission_denied";
    }
    if [
        "connection refused",
        "connection reset",
        "service unavailable",
        "could not resolve host",
        "temporary failure in name resolution",
        "pooltimedout",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        return "service_unavailable";
    }
    if lower.contains("http 429") || lower.contains("rate limit exceeded") {
        return "rate_limited";
    }
    if raw.contains("error[E")
        || raw.contains("error TS")
        || lower.contains("assertion `left == right` failed")
        || lower.contains("assertion failed:")
    {
        return "check_exit";
    }
    "unknown"
}
