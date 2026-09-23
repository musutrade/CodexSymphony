//! Preparation decisions and bounded recovery; no runtime, SQL, or GitHub calls.
use serde::{Deserialize, Serialize};

pub const CORE_VERSION: &str = "harness-gate 0.4.5";
pub const CORE_SHA256: &str = "70721282c751826ed4d57e14bd7de9516e73e833aa058d758dbd2154c0aa5e10";
pub const CODEX_VERSION: &str = "codex-cli 0.156.1";
pub const RESERVE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub code: String,
    pub phase: String,
    pub detail: String,
    pub exit_code: Option<i32>,
    pub evidence: String,
}

impl Failure {
    /// Only enabled preparation failures are automatically probed. Other current
    /// errors retain their raw facts and require their phase-specific controller.
    pub fn automatic_retry(&self) -> bool {
        matches!(
            self.code.as_str(),
            "preparation_dependency_missing"
                | "preparation_capability_mismatch"
                | "preparation_path_unwritable"
                | "network_scope_unavailable"
                | "cleanup_failed"
        )
    }

    pub fn new(code: &str, detail: &str, evidence: &str) -> Self {
        Self {
            code: code.into(),
            phase: "preparation".into(),
            detail: detail.into(),
            exit_code: None,
            evidence: evidence.into(),
        }
    }
}

/// A single ledger is shared by preparation and future infrastructure phases.
/// There is deliberately no code-repair counter in this record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retry {
    pub phase: String,
    pub attempts: u32,
    pub group_attempts: u32,
    pub probes: u32,
    pub started_at: i64,
    pub next_attempt_at: Option<i64>,
    pub todo: bool,
    pub last_failure: Option<Failure>,
}

impl Retry {
    pub fn new(phase: &str, now: i64) -> Self {
        Self {
            phase: phase.into(),
            attempts: 0,
            group_attempts: 0,
            probes: 0,
            started_at: now,
            next_attempt_at: Some(now),
            todo: false,
            last_failure: None,
        }
    }

    pub fn due(&self, now: i64, paused: bool) -> bool {
        !paused && !self.todo && self.next_attempt_at.is_some_and(|at| now >= at)
    }

    /// Persist before attempting any work so a crash consumes the attempt.
    pub fn begin(&mut self, now: i64, paused: bool) -> bool {
        if !self.due(now, paused) {
            return false;
        }
        if self.expired(now) {
            self.exhaust();
            return false;
        }
        self.attempts += 1;
        self.group_attempts += 1;
        if self.group_attempts > 1 {
            self.probes += 1;
        }
        self.next_attempt_at = None;
        true
    }

    fn expired(&self, now: i64) -> bool {
        self.group_attempts >= 3 || self.probes >= 2 || now >= self.started_at.saturating_add(600)
    }

    pub fn fail(&mut self, failure: Failure, now: i64) {
        let requires_authorization = !failure.automatic_retry();
        self.last_failure = Some(failure);
        if requires_authorization || self.expired(now) {
            self.exhaust();
            return;
        }
        let delay = if self.group_attempts == 1 { 30 } else { 120 };
        self.next_attempt_at = Some(now.saturating_add(delay));
    }

    pub fn active(&self) -> bool {
        self.next_attempt_at.is_none() && !self.todo && self.attempts > 0
    }

    pub fn complete(&mut self, failure: Option<Failure>, now: i64) {
        if now >= self.started_at.saturating_add(600) {
            self.fail(
                Failure::new(
                    "budget_exhausted",
                    "preparation exceeded ten minute deadline",
                    "preparation_history",
                ),
                now,
            );
        } else if let Some(failure) = failure {
            self.fail(failure, now);
        } else {
            self.success();
        }
    }

    pub fn success(&mut self) {
        self.next_attempt_at = None;
        self.last_failure = None;
    }

    pub fn exhaust(&mut self) {
        if self.last_failure.is_none() {
            self.last_failure = Some(Failure::new(
                "budget_exhausted",
                "preparation probe budget exhausted; reconcile and explicitly authorize recovery",
                "preparation_history",
            ));
        }
        self.todo = true;
        self.next_attempt_at = None;
    }

    /// Caller must persist an explicit user authorization before calling this.
    /// The history table and total attempts are retained; pause is independent.
    pub fn authorize_retry_group(&mut self, now: i64) {
        self.group_attempts = 0;
        self.probes = 0;
        self.started_at = now;
        self.next_attempt_at = Some(now);
        self.todo = false;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkEvidence {
    pub configuration_identity: String,
    pub reachable: bool,
}

/// Identity includes the exact launch, immutable review, deployment and fresh
/// command/exec sample. Only the platform adapter constructs admission evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub deployment_identity: String,
    pub execution_identity: String,
    pub network: NetworkEvidence,
    pub failures: Vec<Failure>,
    pub sample: serde_json::Value,
}

impl Evidence {
    pub fn failure(&self, expected: &str, requested: &[String]) -> Option<Failure> {
        if self.deployment_identity != expected || self.execution_identity.is_empty() {
            return Some(Failure::new(
                "policy_identity_mismatch",
                "deployment or execution identity unknown",
                "preparation_history",
            ));
        }
        if let Some(failure) = self.failures.first() {
            return Some(failure.clone());
        }
        if !network_ready(expected, requested, &self.network) {
            return Some(Failure::new(
                "network_scope_unavailable",
                "verify required service connectivity in the trusted development environment",
                "preparation_history",
            ));
        }
        None
    }
}

pub fn network_ready(
    expected_identity: &str,
    requested: &[String],
    evidence: &NetworkEvidence,
) -> bool {
    !expected_identity.is_empty()
        && expected_identity == evidence.configuration_identity
        && (requested.is_empty() || evidence.reachable)
}
