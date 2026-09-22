//! Pure admission and reconciliation identities. GitHub success is not business Done.
use crate::{
    github::{CheckState, MergeFact, Observation, Policy},
    validation::{self, ValidationEvidence},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub delivery_key: String,
    pub requirement: i64,
    pub revision: i64,
    pub authorization: Option<i64>,
    pub policy: Policy,
    pub pr: u64,
    pub head: String,
    pub base: String,
    pub checkout_sha: Option<String>,
    pub branch: String,
    pub validation_id: String,
    pub dependencies: Value,
}
impl Intent {
    pub fn action_key(&self) -> String {
        validation::sha256(serde_json::to_vec(self).expect("serializable merge identity"))
    }
    pub fn matches(&self, observation: &Observation) -> bool {
        observation.policy == self.policy
            && observation.repository_id == self.policy.repository_id
            && observation.number == self.pr
            && observation.head == self.head
            && observation.head_ref == self.branch
            && observation.base_ref == self.policy.default_branch
    }
}

/// Head CAS is combined with enforced up-to-date checks on the repository.
pub fn protected(policy: &Policy) -> bool {
    policy.delivery.as_ref().is_some_and(|delivery| {
        delivery.actions.merge
            && delivery.protection["required_status_checks"]["strict"] == true
            && delivery.protection["enforce_admins"]["enabled"] == true
            && delivery.blockers(policy).is_empty()
    })
}

pub fn admit(
    intent: &Intent,
    observation: &Observation,
    evidence: &ValidationEvidence,
    required: &[String],
    now: i64,
) -> bool {
    eligible(intent, observation, now) && checkout_passes(intent, observation, evidence, required)
}

pub fn eligible(intent: &Intent, observation: &Observation, now: i64) -> bool {
    protected(&intent.policy)
        && admission_identity(intent, observation)
        && (0..60).contains(&now.saturating_sub(observation.last_synced_at))
}
fn admission_identity(intent: &Intent, observation: &Observation) -> bool {
    let checkout = observation
        .phases
        .as_ref()
        .and_then(|phases| phases.first())
        .and_then(|phase| phase.expected_checkout_sha.as_ref());
    sha(&intent.head)
        && sha(&intent.base)
        && intent.matches(observation)
        && observation.base == intent.base
        && !observation.closed
        && observation.merge == MergeFact::Unmerged
        && checkout == intent.checkout_sha.as_ref()
}

fn checkout_passes(
    intent: &Intent,
    observation: &Observation,
    evidence: &ValidationEvidence,
    required: &[String],
) -> bool {
    let Some(phase) = observation
        .phases
        .as_ref()
        .and_then(|phases| phases.first())
    else {
        return false;
    };
    if !phase_matches(intent, observation, phase, &evidence.candidate.sha) {
        return false;
    }
    phase
        .checks
        .iter()
        .all(|check| check.state == CheckState::Success)
        && validation::verify(evidence, &evidence.candidate, &evidence.trusted, required).is_ok()
}

fn phase_matches(
    intent: &Intent,
    observation: &Observation,
    phase: &crate::github_contract::PhaseEvidence,
    candidate: &str,
) -> bool {
    !(phase.phase != "pre_merge"
        || phase.head_sha != intent.head
        || phase.base_sha != intent.base
        || phase.expected_checkout_sha.as_deref() != Some(candidate)
        || phase.checks.is_empty()
        || !phase.blockers.is_empty()
        || !check_identity(intent, observation, phase))
}

fn check_identity(
    intent: &Intent,
    observation: &Observation,
    phase: &crate::github_contract::PhaseEvidence,
) -> bool {
    let Some(contract) = &intent.policy.delivery else {
        return false;
    };
    let selected = match contract.pre_merge.source {
        crate::github_contract::PreMergeSource::Head => Some(&intent.head),
        crate::github_contract::PreMergeSource::TestMerge => observation.test_merge_sha.as_ref(),
    };
    selected.is_some()
        && phase.check_sha.as_ref() == selected
        && phase.checks.len() == contract.pre_merge.checks.len()
        && phase
            .checks
            .iter()
            .zip(&contract.pre_merge.checks)
            .all(|(actual, expected)| actual.selector == expected.selector)
}

fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A changed base after merging is expected; only the original PR/head can reconcile.
pub fn confirmed(intent: &Intent, observation: &Observation) -> Option<String> {
    if !intent.matches(observation) || observation.merge != MergeFact::Merged {
        return None;
    }
    observation
        .merged_sha
        .as_ref()
        .filter(|value| sha(value))
        .cloned()
}
