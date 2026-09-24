//! P9 controlled extension values and compatibility checks, not a dispatcher.
//! See docs/extension-protocol.md; existing lifecycle hooks retain their wire format.
use crate::extension_contract::{
    FrozenConfig, InvocationIdentity, PROTOCOL_VERSION, ProtocolError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    EnvironmentCheck,
    Validate,
    BeforeDeliver,
    CapabilityCheck,
    Submit,
    Observe,
    Reconcile,
    PostDeliveryValidate,
}

/// Deployment-owned allowlist entry. References contain no vendor secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub id: String,
    pub implementation_digest: String,
    pub operations: Vec<Operation>,
    pub scope_ref: String,
    pub config_ref: String,
    pub credential_provider_ref: Option<String>,
}

/// Repository revision -> reviewed environment contract -> installed host profile.
/// Tool/service/cache details belong to the referenced extension configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentBinding {
    pub repository_revision: String,
    pub contract_digest: String,
    pub host_profile_ref: String,
    pub role: String,
}

/// P9 resource-level checks precede a Requirement/AgentRun. They cannot stand
/// in for task authorization or manufacture business/execution records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceCall {
    pub protocol_version: u32,
    pub invocation_id: String,
    pub attempt: u32,
    pub resource_id: String,
    pub controlled_config_digest: String,
    pub environment: EnvironmentBinding,
    pub extension_id: String,
    pub implementation_digest: String,
    pub deadline_unix_ms: i64,
}

impl ResourceCall {
    pub fn validate(
        &self,
        config: &ControlledConfig,
        approved: &[Registration],
    ) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(self.protocol_version));
        }
        labels(&[&self.invocation_id, &self.resource_id])?;
        if self.attempt == 0 || self.deadline_unix_ms <= 0 {
            return Err(ProtocolError::InvalidConfig(
                "invalid resource attempt/deadline",
            ));
        }
        if self.controlled_config_digest != config.freeze(approved)?
            || self.environment != config.environment
        {
            return Err(ProtocolError::IdentityMismatch("resource environment"));
        }
        config.require(&self.extension_id, &Operation::EnvironmentCheck)?;
        if !config.extensions.iter().any(|r| {
            r.id == self.extension_id && r.implementation_digest == self.implementation_digest
        }) {
            return Err(ProtocolError::IdentityMismatch("resource implementation"));
        }
        Ok(())
    }
}

/// Opt-in companion to the existing frozen v1 configuration. Absence means legacy,
/// never an inferred environment or an automatic upgrade of an in-flight Run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlledConfig {
    pub protocol_version: u32,
    pub environment: EnvironmentBinding,
    pub extensions: Vec<Registration>,
}

impl ControlledConfig {
    /// Freeze only after checking the deployment allowlist. This identity binds
    /// the companion configuration without changing existing v1 config hashes.
    pub fn freeze(&self, approved: &[Registration]) -> Result<String, ProtocolError> {
        self.validate(approved)?;
        let bytes = serde_json::to_vec(self).expect("typed controlled config serializes");
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self, approved: &[Registration]) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(self.protocol_version));
        }
        self.environment.validate()?;
        for registration in &self.extensions {
            if !approved.contains(registration) {
                return Err(ProtocolError::UnsupportedCapability("reviewed extension"));
            }
            registration.validate()?;
        }
        unique(self.extensions.iter().map(|r| r.id.as_str()))?;
        Ok(())
    }

    pub fn require(&self, id: &str, operation: &Operation) -> Result<(), ProtocolError> {
        if self
            .extensions
            .iter()
            .any(|r| r.id == id && r.operations.contains(operation))
        {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedCapability("extension operation"))
        }
    }
}

impl EnvironmentBinding {
    fn validate(&self) -> Result<(), ProtocolError> {
        labels(&[
            &self.repository_revision,
            &self.host_profile_ref,
            &self.role,
        ])?;
        digest(&self.contract_digest)
    }
}

impl Registration {
    fn validate(&self) -> Result<(), ProtocolError> {
        labels(&[&self.id, &self.scope_ref, &self.config_ref])?;
        digest(&self.implementation_digest)?;
        if self.operations.is_empty() {
            return Err(ProtocolError::InvalidConfig("empty operations"));
        }
        if let Some(reference) = &self.credential_provider_ref {
            labels(&[reference])?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub commit: String,
    pub tree: String,
}

/// Frozen per-call inputs. The supervisor authenticates the actual source;
/// matching these self-reported fields alone does not establish trust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub identity: InvocationIdentity,
    pub controlled_config_digest: String,
    pub operation: Operation,
    pub extension_id: String,
    pub implementation_digest: String,
    pub candidate: Option<SourceIdentity>,
    pub environment_digest: String,
    pub policy_digest: String,
    pub deadline_unix_ms: i64,
    pub required_checks: Vec<String>,
}

impl Call {
    pub fn validate(
        &self,
        frozen: &FrozenConfig,
        config: &ControlledConfig,
        approved: &[Registration],
    ) -> Result<(), ProtocolError> {
        self.identity.validate(frozen, false)?;
        if config.freeze(approved)? != self.controlled_config_digest {
            return Err(ProtocolError::IdentityMismatch("controlled config"));
        }
        config.require(&self.extension_id, &self.operation)?;
        let registered = config
            .extensions
            .iter()
            .find(|r| r.id == self.extension_id)
            .expect("require checked registration");
        if registered.implementation_digest != self.implementation_digest {
            return Err(ProtocolError::IdentityMismatch("implementation"));
        }
        digest(&self.environment_digest)?;
        digest(&self.policy_digest)?;
        if self.deadline_unix_ms <= 0 {
            return Err(ProtocolError::InvalidConfig("invalid deadline"));
        }
        self.validate_source()?;
        unique(self.required_checks.iter().map(String::as_str))
    }

    fn validate_source(&self) -> Result<(), ProtocolError> {
        match &self.candidate {
            Some(source) => labels(&[&source.commit, &source.tree]),
            None if matches!(
                self.operation,
                Operation::EnvironmentCheck | Operation::CapabilityCheck
            ) =>
            {
                Ok(())
            }
            None => Err(ProtocolError::InvalidConfig("candidate required")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub artifact_id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub id: String,
    pub verdict: Verdict,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evaluation {
    pub call: Call,
    pub verdict: Verdict,
    pub checks: Vec<CheckResult>,
}

impl Evaluation {
    /// Structural prerequisite only. The caller must additionally authenticate
    /// provenance, verify artifact bytes, current authorization and quiescence.
    pub fn check_pass(&self, expected: &Call, now_unix_ms: i64) -> Result<(), ProtocolError> {
        self.check_call(expected, now_unix_ms)?;
        self.check_completeness(&expected.required_checks)?;
        // Extra failing/unknown checks cannot be hidden behind an aggregate pass.
        for check in &self.checks {
            check.check_pass()?;
        }
        Ok(())
    }

    fn check_call(&self, expected: &Call, now_unix_ms: i64) -> Result<(), ProtocolError> {
        expected.identity.matches_result(&self.call.identity)?;
        if &self.call != expected {
            return Err(ProtocolError::IdentityMismatch("controlled call"));
        }
        if now_unix_ms >= expected.deadline_unix_ms {
            return Err(ProtocolError::InvalidResult("expired result"));
        }
        if self.verdict != Verdict::Pass {
            return Err(ProtocolError::InvalidResult("evaluation not pass"));
        }
        Ok(())
    }

    fn check_completeness(&self, required_checks: &[String]) -> Result<(), ProtocolError> {
        unique(required_checks.iter().map(String::as_str))?;
        unique(self.checks.iter().map(|c| c.id.as_str()))?;
        for required in required_checks {
            if !self.checks.iter().any(|c| &c.id == required) {
                return Err(ProtocolError::InvalidResult("missing required check"));
            }
        }
        Ok(())
    }
}

impl CheckResult {
    fn check_pass(&self) -> Result<(), ProtocolError> {
        if self.verdict != Verdict::Pass || self.evidence.is_empty() {
            return Err(ProtocolError::InvalidResult("check not evidenced pass"));
        }
        for evidence in &self.evidence {
            labels(&[&evidence.artifact_id])?;
            digest(&evidence.sha256)?;
        }
        Ok(())
    }
}

fn labels(values: &[&str]) -> Result<(), ProtocolError> {
    if values.iter().any(|v| v.trim().is_empty() || v.len() > 200) {
        Err(ProtocolError::InvalidConfig("invalid reference"))
    } else {
        Ok(())
    }
}

fn digest(value: &str) -> Result<(), ProtocolError> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidConfig("invalid sha256"))
    }
}

fn unique<'a>(values: impl Iterator<Item = &'a str>) -> Result<(), ProtocolError> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        labels(&[value])?;
        if !seen.insert(value) {
            return Err(ProtocolError::InvalidConfig("duplicate identity"));
        }
    }
    Ok(())
}
