//! Versioned delivery requirements, never authority to execute a remote action.
use crate::github::{Policy, Selector, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeliveryContract {
    pub schema_version: u32,
    pub pre_merge: PreMerge,
    pub post_merge: PostMerge,
    pub actions: Actions,
    /// Reviewed full protection response and effective rules. Any change blocks
    /// until a new repository policy is authorized; never loosen live rules.
    pub protection: Value,
    pub rules: Vec<Value>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Actions {
    pub rerun_actions: bool,
    pub rerequest_checks: bool,
    pub merge: bool,
    pub merge_method: Option<String>,
    pub read_logs: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreMergeSource {
    Head,
    TestMerge,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreMerge {
    pub source: PreMergeSource,
    pub checkout: PreMergeSource,
    pub checks: Vec<RequiredCheck>,
    pub wait_seconds: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PostMerge {
    Checks {
        checks: Vec<RequiredCheck>,
        wait_seconds: u64,
        probe_pr: u64,
    },
    FixedValidation {
        /// Reference to a reviewed platform validation plan, not a shell command.
        plan_id: String,
        configuration_sha256: String,
        authorization: String,
        wait_seconds: u64,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheck {
    pub selector: Selector,
    /// All required checks apply to the entire selected phase. Conditional
    /// skipped/N/A checks are deliberately not accepted as successes.
    pub applicability: String,
    pub trigger: Trigger,
    /// Actions job display name, independently bound through check_run_url.
    pub job: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    PullRequest,
    Push,
    WorkflowDispatch,
    /// External publisher configuration pinned in the target repository.
    /// The approved configuration defines the publisher's trigger contract.
    External {
        path: String,
        blob_sha: String,
        event: String,
    },
}
impl DeliveryContract {
    pub fn all_checks(&self) -> Vec<&RequiredCheck> {
        let mut checks: Vec<_> = self.pre_merge.checks.iter().collect();
        if let PostMerge::Checks { checks: post, .. } = &self.post_merge {
            checks.extend(post);
        }
        checks
    }
    pub fn blockers(&self, policy: &Policy) -> Vec<String> {
        let mut blockers = Vec::new();
        if self.schema_version != 1 {
            blockers.push("delivery.schema_version: expected 1".into());
        }
        if self.pre_merge.wait_seconds == 0 || self.pre_merge.checks.is_empty() {
            blockers.push("delivery.pre_merge: checks and deadline required".into());
        }
        let selectors: Vec<_> = self
            .pre_merge
            .checks
            .iter()
            .map(|c| c.selector.clone())
            .collect();
        if selectors != policy.required {
            blockers
                .push("delivery.pre_merge: required selectors must match legacy projection".into());
        }
        for check in &self.pre_merge.checks {
            validate_check(check, false, &mut blockers);
        }
        self.post_blockers(&mut blockers);
        self.protection_blockers(&mut blockers);
        blockers
    }
    fn post_blockers(&self, blockers: &mut Vec<String>) {
        match &self.post_merge {
            PostMerge::Checks {
                checks,
                wait_seconds,
                probe_pr,
            } => {
                if checks.is_empty() || *wait_seconds == 0 || *probe_pr == 0 {
                    blockers.push("delivery.post_merge: checks and deadline required".into());
                }
                for check in checks {
                    validate_check(check, true, blockers);
                }
            }
            PostMerge::FixedValidation {
                plan_id,
                configuration_sha256,
                authorization,
                wait_seconds,
            } => {
                if !fixed_identity(plan_id, authorization, *wait_seconds, configuration_sha256) {
                    blockers.push(
                        "delivery.post_merge: authorized fixed validation identity required".into(),
                    );
                }
            }
        }
    }
    fn protection_blockers(&self, blockers: &mut Vec<String>) {
        self.merge_blockers(blockers);
        if !self.protection.is_object() {
            blockers.push("delivery.protection: reviewed protection snapshot required".into());
        }
        if (self.actions.merge
            || self.pre_merge.source == PreMergeSource::TestMerge
            || self.pre_merge.checkout == PreMergeSource::TestMerge)
            && !strict(&self.protection)
        {
            blockers.push(
                "delivery.pre_merge: automatic merge or test-merge requires enforced strict base protection".into(),
            );
        }
    }
    fn merge_blockers(&self, blockers: &mut Vec<String>) {
        if self.actions.merge {
            merge_constraints(&self.protection, &self.rules, blockers);
        }
        if self.actions.merge
            && !matches!(
                self.actions.merge_method.as_deref(),
                Some("merge" | "squash" | "rebase")
            )
        {
            blockers.push("delivery.actions: merge method required".into());
        }
        if self.actions.merge && reviews_required(&self.protection, &self.rules) {
            blockers.push(
                "delivery.review: required independent approvals need a supported review path"
                    .into(),
            );
        }
    }
}
fn merge_constraints(protection: &Value, rules: &[Value], blockers: &mut Vec<String>) {
    if protection["required_signatures"]["enabled"] == true {
        blockers.push("delivery.protection: signed-commit delivery capability unavailable".into());
    }
    for rule in rules {
        if !matches!(
            rule["type"].as_str(),
            Some(
                "required_status_checks"
                    | "pull_request"
                    | "non_fast_forward"
                    | "deletion"
                    | "creation"
            )
        ) {
            blockers.push(format!(
                "delivery.protection: unsupported merge constraint: {}",
                rule["type"]
            ));
        }
    }
}
fn reviews_required(protection: &Value, rules: &[Value]) -> bool {
    review_requirement(&protection["required_pull_request_reviews"])
        || rules
            .iter()
            .any(|r| r["type"] == "pull_request" && review_requirement(&r["parameters"]))
}
fn review_requirement(value: &Value) -> bool {
    value["required_approving_review_count"]
        .as_u64()
        .unwrap_or(0)
        > 0
        || value["require_code_owner_reviews"] == true
        || value["require_last_push_approval"] == true
}
fn fixed_identity(plan: &str, authorization: &str, wait: u64, hash: &str) -> bool {
    !plan.is_empty() && !authorization.is_empty() && wait > 0 && digest(hash)
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn strict(protection: &Value) -> bool {
    protection["required_status_checks"]["strict"] == true
        && protection["enforce_admins"]["enabled"] == true
        && !protection["required_status_checks"]["checks"]
            .as_array()
            .is_none_or(Vec::is_empty)
}
fn validate_check(check: &RequiredCheck, post: bool, blockers: &mut Vec<String>) {
    if check.applicability != "always" || check.selector.name.is_empty() {
        blockers.push("delivery.check: explicit always applicability and name required".into());
    }
    let event = trigger_event(&check.trigger, blockers);
    if post && !matches!(event, "push" | "workflow_dispatch") {
        blockers
            .push("delivery.post_merge: PR-only trigger cannot produce merged-SHA evidence".into());
    }
    match &check.selector.source {
        Source::Actions { .. } => validate_actions(check, event, post, blockers),
        Source::CheckRun { app_id } => validate_external(*app_id, &check.trigger, blockers),
        Source::Status { creator_id } => validate_external(*creator_id, &check.trigger, blockers),
    }
}
fn validate_actions(check: &RequiredCheck, event: &str, post: bool, blockers: &mut Vec<String>) {
    let Source::Actions {
        app_id,
        workflow_id,
        workflow_sha,
        event: selected,
        branch,
        branch_from_pr,
    } = &check.selector.source
    else {
        return;
    };
    if !actions_identity(*app_id, *workflow_id, workflow_sha, selected, event, branch)
        || check.job.as_ref().is_none_or(|v| v.is_empty())
    {
        blockers.push("delivery.check: incomplete Actions workflow/job/event identity".into());
    }
    if post && *branch_from_pr == Some(true) {
        blockers.push("delivery.post_merge: PR branch binding is not a merged branch".into());
    }
}
fn actions_identity(
    app: u64,
    workflow: u64,
    sha: &str,
    selected: &str,
    event: &str,
    branch: &str,
) -> bool {
    app > 0 && workflow > 0 && !sha.is_empty() && selected == event && !branch.is_empty()
}
fn trigger_event<'a>(trigger: &'a Trigger, blockers: &mut Vec<String>) -> &'a str {
    match trigger {
        Trigger::PullRequest => "pull_request",
        Trigger::Push => "push",
        Trigger::WorkflowDispatch => "workflow_dispatch",
        Trigger::External {
            event,
            path,
            blob_sha,
        } => {
            if path.is_empty() || blob_sha.is_empty() {
                blockers.push("delivery.trigger: external configuration identity required".into());
            }
            event
        }
    }
}
fn validate_external(publisher: u64, trigger: &Trigger, blockers: &mut Vec<String>) {
    if publisher == 0 || !matches!(trigger, Trigger::External { .. }) {
        blockers.push(
            "delivery.check: external publisher and pinned trigger configuration required".into(),
        );
    }
}
/// Request only the configured actions; legacy grants retain their original scope.
pub fn permissions(policy: &Policy) -> Value {
    let mut permissions =
        json!({"contents":"write","pull_requests":"write","checks":"read","actions":"read"});
    let Some(contract) = &policy.delivery else {
        return permissions;
    };
    permissions["administration"] = json!("read");
    if contract.actions.rerun_actions || contract.all_checks().iter().any(dispatch_check) {
        permissions["actions"] = json!("write");
    }
    if contract.actions.rerequest_checks {
        permissions["checks"] = json!("write");
    }
    if contract.all_checks().iter().any(status_check) {
        permissions["statuses"] = json!("read");
    }
    permissions
}

fn dispatch_check(check: &&RequiredCheck) -> bool {
    matches!(check.trigger, Trigger::WorkflowDispatch)
}
fn status_check(check: &&RequiredCheck) -> bool {
    matches!(check.selector.source, Source::Status { .. })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PhaseEvidence {
    pub phase: String,
    pub head_sha: String,
    pub base_sha: String,
    pub check_sha: Option<String>,
    /// Populated only by independent fixed-candidate validation, never inferred
    /// from an Actions run's head_sha or GitHub's synthetic merge commit.
    pub actual_checkout_sha: Option<String>,
    pub expected_checkout_sha: Option<String>,
    pub validation: Option<crate::validation::ValidationEvidence>,
    pub checks: Vec<crate::github::Check>,
    pub blockers: Vec<String>,
    pub wait_seconds: u64,
}
impl PhaseEvidence {
    pub fn bind_validation(
        &mut self,
        evidence: crate::validation::ValidationEvidence,
        candidate: &crate::validation::Candidate,
        trusted: &crate::validation::TrustedIdentity,
        steps: &[String],
    ) -> Result<(), crate::validation::ValidationError> {
        if self.expected_checkout_sha.as_deref() != Some(candidate.sha.as_str()) {
            return Err(crate::validation::ValidationError::CandidateMismatch);
        }
        crate::validation::verify(&evidence, candidate, trusted, steps)?;
        self.actual_checkout_sha = Some(candidate.sha.clone());
        self.validation = Some(evidence);
        Ok(())
    }

    fn completed(&self) -> bool {
        let checks_passed = if self.checks.is_empty() {
            self.validation.is_some()
        } else {
            self.checks
                .iter()
                .all(|c| c.state == crate::github::CheckState::Success)
        };
        checks_passed
            && self.actual_checkout_sha.is_some()
            && self.actual_checkout_sha == self.expected_checkout_sha
    }
    /// A phase deadline is supplied by its persisted start, not renewed by polls.
    pub fn state(&self, started_at: i64, now: i64) -> crate::github::CheckState {
        use crate::github::CheckState;
        if !self.blockers.is_empty() {
            return CheckState::Ambiguous;
        }
        if self.check_sha.is_none() {
            return CheckState::Missing;
        }
        if self.checks.iter().any(|c| c.state == CheckState::Failure) {
            return CheckState::Failure;
        }
        if self.checks.iter().any(|c| c.state == CheckState::Ambiguous) {
            return CheckState::Ambiguous;
        }
        if self.completed() {
            return CheckState::Success;
        }
        if now.saturating_sub(started_at) >= self.wait_seconds as i64 {
            return CheckState::Failure;
        }
        CheckState::Pending
    }
}
