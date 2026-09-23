//! Versioned, pure extension contracts. Adapters may consume these values, but
//! authorization, preservation and business conclusions remain with the core.
use crate::contract::Repository;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_RESULT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    UnsupportedVersion(u32),
    InvalidConfig(&'static str),
    UnsupportedCapability(&'static str),
    IdentityMismatch(&'static str),
    InvalidResult(&'static str),
}

fn version(value: u32) -> Result<(), ProtocolError> {
    if value == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::UnsupportedVersion(value))
    }
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 200
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub provider: String,
    /// None preserves the legacy Codex Runtime choice. It is never silently
    /// substituted for an explicitly selected model.
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl ModelConfig {
    fn validate(&self) -> Result<(), ProtocolError> {
        if !nonempty(&self.provider) {
            return Err(ProtocolError::InvalidConfig("agent/provider required"));
        }
        if self.model.as_ref().is_some_and(|v| !nonempty(v))
            || self.effort.as_ref().is_some_and(|v| !nonempty(v))
            || (self.effort.is_some() && self.model.is_none())
        {
            return Err(ProtocolError::InvalidConfig("invalid model/effort"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    GithubPr,
    LocalGit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    AfterCreate,
    BeforeRun,
    AfterRun,
    BeforeRemove,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookRole {
    Coding,
    Repair,
    Validation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayPolicy {
    Never,
    Idempotent,
    Reconcile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HookConfig {
    pub name: String,
    pub event: HookEvent,
    pub roles: Vec<HookRole>,
    pub argv: Vec<String>,
    /// Identity of reviewed script content and its dependency package version.
    pub script_identity: String,
    pub timeout_seconds: u32,
    pub output_limit_bytes: u32,
    pub replay: ReplayPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookConfigWire {
    name: String,
    event: HookEvent,
    roles: Vec<HookRole>,
    argv: Vec<String>,
    script_identity: String,
    timeout_seconds: u32,
    output_limit_bytes: u32,
    replay: Option<ReplayPolicy>,
}

impl<'de> Deserialize<'de> for HookConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = HookConfigWire::deserialize(deserializer)?;
        Ok(Self {
            name: wire.name,
            event: wire.event,
            roles: wire.roles,
            argv: wire.argv,
            script_identity: wire.script_identity,
            timeout_seconds: wire.timeout_seconds,
            output_limit_bytes: wire.output_limit_bytes,
            replay: wire.replay.unwrap_or(ReplayPolicy::Never),
        })
    }
}

impl HookConfig {
    pub(crate) fn validate(&self) -> Result<(), ProtocolError> {
        if !nonempty(&self.name) || !nonempty(&self.script_identity) {
            return Err(ProtocolError::InvalidConfig("invalid hook"));
        }
        if self.argv.is_empty() || self.argv.iter().any(|a| a.is_empty()) {
            return Err(ProtocolError::InvalidConfig("invalid hook"));
        }
        if self.roles.is_empty() || self.timeout_seconds == 0 || self.output_limit_bytes == 0 {
            return Err(ProtocolError::InvalidConfig("invalid hook"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionConfig {
    pub protocol_version: u32,
    pub agent: String,
    pub model: ModelConfig,
    pub delivery: DeliveryMode,
    pub hooks: Vec<HookConfig>,
    /// None keeps advisory decisions disabled.
    pub decision: Option<String>,
}

impl ExtensionConfig {
    /// Existing repository rows have no extension fields. Their implicit path
    /// is Codex, GitHub PR and no hooks, with the same optional model value.
    pub fn from_legacy_repository(repo: &Repository) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            agent: "codex".into(),
            model: ModelConfig {
                provider: "codex".into(),
                model: repo.model.clone(),
                effort: None,
            },
            delivery: DeliveryMode::GithubPr,
            hooks: repo.hooks.clone(),
            decision: None,
        }
    }

    pub fn validate(&self, capabilities: &Capabilities) -> Result<(), ProtocolError> {
        version(self.protocol_version)?;
        if !nonempty(&self.agent) {
            return Err(ProtocolError::InvalidConfig("agent/provider required"));
        }
        self.model.validate()?;
        capabilities.require_config(self)?;
        self.validate_hook_registration(capabilities)?;
        self.validate_decision_registration(capabilities)
    }

    fn validate_hook_registration(&self, capabilities: &Capabilities) -> Result<(), ProtocolError> {
        for hook in &self.hooks {
            hook.validate()?;
            if !capabilities.hooks.contains(hook) {
                return Err(ProtocolError::UnsupportedCapability("hook registration"));
            }
        }
        Ok(())
    }

    fn validate_decision_registration(
        &self,
        capabilities: &Capabilities,
    ) -> Result<(), ProtocolError> {
        if let Some(decision) = &self.decision
            && (!nonempty(decision) || !capabilities.decisions.contains(decision))
        {
            return Err(ProtocolError::UnsupportedCapability("decision"));
        }
        Ok(())
    }

    /// Freeze the actual selected config. The digest covers script identities,
    /// not mutable workspace paths; #104 verifies those identities at dispatch.
    pub fn freeze(&self, capabilities: &Capabilities) -> Result<FrozenConfig, ProtocolError> {
        self.validate(capabilities)?;
        let bytes = serde_json::to_vec(self).expect("typed extension config serializes");
        let id = format!("sha256:{:x}", Sha256::digest(bytes));
        Ok(FrozenConfig {
            config_id: id,
            value: self.clone(),
        })
    }
}

/// Deployment registration is an allowlist. No capability is inferred from a
/// requested config or from provider names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCapability {
    pub name: String,
    pub models: Vec<ModelConfig>,
    pub reliable_stop: bool,
    pub resume: bool,
    pub cancel: bool,
    pub structured_events: bool,
    pub usage_reporting: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub agents: Vec<AgentCapability>,
    pub deliveries: Vec<DeliveryMode>,
    pub hooks: Vec<HookConfig>,
    pub decisions: Vec<String>,
}

impl Capabilities {
    fn require_config(&self, config: &ExtensionConfig) -> Result<(), ProtocolError> {
        let agent = self
            .agents
            .iter()
            .find(|agent| agent.name == config.agent)
            .ok_or(ProtocolError::UnsupportedCapability("agent/model"))?;
        if !agent.models.contains(&config.model) {
            return Err(ProtocolError::UnsupportedCapability("agent/model"));
        }
        self.require_unattended_agent(&config.agent)?;
        if !self.deliveries.contains(&config.delivery) {
            return Err(ProtocolError::UnsupportedCapability("delivery"));
        }
        Ok(())
    }

    pub fn legacy_codex(model: Option<String>) -> Self {
        Self {
            agents: Vec::from([AgentCapability {
                name: "codex".into(),
                models: Vec::from([ModelConfig {
                    provider: "codex".into(),
                    model,
                    effort: None,
                }]),
                reliable_stop: true,
                resume: true,
                cancel: true,
                structured_events: true,
                usage_reporting: true,
            }]),
            deliveries: Vec::from([DeliveryMode::GithubPr]),
            hooks: Vec::new(),
            decisions: Vec::new(),
        }
    }

    pub fn require_unattended_agent(&self, name: &str) -> Result<(), ProtocolError> {
        if self
            .agents
            .iter()
            .any(|agent| agent.name == name && agent.reliable_stop)
        {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedCapability("reliable stop"))
        }
    }

    pub fn require_resume(&self, name: &str) -> Result<(), ProtocolError> {
        if self
            .agents
            .iter()
            .any(|agent| agent.name == name && agent.resume)
        {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedCapability("resume"))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenConfig {
    pub config_id: String,
    pub value: ExtensionConfig,
}

impl FrozenConfig {
    pub fn validate_identity(&self) -> Result<(), ProtocolError> {
        version(self.value.protocol_version)?;
        let bytes = serde_json::to_vec(&self.value).expect("typed extension config serializes");
        let expected = format!("sha256:{:x}", Sha256::digest(bytes));
        if self.config_id == expected {
            Ok(())
        } else {
            Err(ProtocolError::IdentityMismatch("config_id"))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationIdentity {
    pub protocol_version: u32,
    pub requirement_id: i64,
    pub revision: i64,
    pub run_id: Option<String>,
    pub resource_id: String,
    pub invocation_id: String,
    pub attempt: u32,
    pub config_id: String,
}

impl InvocationIdentity {
    pub fn validate(&self, frozen: &FrozenConfig, cleanup: bool) -> Result<(), ProtocolError> {
        version(self.protocol_version)?;
        frozen.validate_identity()?;
        if !self.has_valid_ids() || !self.has_valid_run(cleanup) {
            return Err(ProtocolError::InvalidConfig("invalid invocation identity"));
        }
        if self.config_id != frozen.config_id {
            return Err(ProtocolError::IdentityMismatch("config_id"));
        }
        Ok(())
    }

    fn has_valid_ids(&self) -> bool {
        self.requirement_id > 0
            && self.revision > 0
            && self.attempt > 0
            && nonempty(&self.resource_id)
            && nonempty(&self.invocation_id)
    }

    fn has_valid_run(&self, cleanup: bool) -> bool {
        self.run_id.as_ref().is_none_or(|v| nonempty(v)) && (cleanup || self.run_id.is_some())
    }

    pub fn matches_result(&self, result: &Self) -> Result<(), ProtocolError> {
        version(result.protocol_version)?;
        if self == result {
            Ok(())
        } else {
            Err(ProtocolError::IdentityMismatch("result identity"))
        }
    }
}

/// P3 script stdin. The caller freezes the configuration and validates this
/// envelope before #104 starts a process; the context contains task data, not
/// credentials or authority to perform a later action.
#[derive(Clone, Debug, PartialEq)]
pub struct HookInvocation {
    pub identity: InvocationIdentity,
    pub event: HookEvent,
    pub role: HookRole,
    pub workspace: String,
    pub output_dir: String,
    pub deadline_at: String,
    pub context: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookInvocationWire {
    protocol_version: u32,
    requirement_id: i64,
    revision: i64,
    run_id: Option<String>,
    resource_id: String,
    invocation_id: String,
    attempt: u32,
    config_id: String,
    event: HookEvent,
    role: HookRole,
    workspace: String,
    output_dir: String,
    deadline_at: String,
    context: serde_json::Map<String, serde_json::Value>,
}

impl From<&HookInvocation> for HookInvocationWire {
    fn from(value: &HookInvocation) -> Self {
        Self {
            protocol_version: value.identity.protocol_version,
            requirement_id: value.identity.requirement_id,
            revision: value.identity.revision,
            run_id: value.identity.run_id.clone(),
            resource_id: value.identity.resource_id.clone(),
            invocation_id: value.identity.invocation_id.clone(),
            attempt: value.identity.attempt,
            config_id: value.identity.config_id.clone(),
            event: value.event.clone(),
            role: value.role.clone(),
            workspace: value.workspace.clone(),
            output_dir: value.output_dir.clone(),
            deadline_at: value.deadline_at.clone(),
            context: value.context.clone(),
        }
    }
}

impl Serialize for HookInvocation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        HookInvocationWire::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HookInvocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = HookInvocationWire::deserialize(deserializer)?;
        Ok(Self {
            identity: InvocationIdentity {
                protocol_version: wire.protocol_version,
                requirement_id: wire.requirement_id,
                revision: wire.revision,
                run_id: wire.run_id,
                resource_id: wire.resource_id,
                invocation_id: wire.invocation_id,
                attempt: wire.attempt,
                config_id: wire.config_id,
            },
            event: wire.event,
            role: wire.role,
            workspace: wire.workspace,
            output_dir: wire.output_dir,
            deadline_at: wire.deadline_at,
            context: wire.context,
        })
    }
}

impl HookInvocation {
    pub fn validate(&self, frozen: &FrozenConfig, hook: &HookConfig) -> Result<(), ProtocolError> {
        self.identity
            .validate(frozen, self.event == HookEvent::BeforeRemove)?;
        if !frozen.value.hooks.contains(hook)
            || self.event != hook.event
            || !hook.roles.contains(&self.role)
        {
            return Err(ProtocolError::UnsupportedCapability("hook event/role"));
        }
        if self.workspace.trim().is_empty()
            || self.output_dir.trim().is_empty()
            || self.deadline_at.trim().is_empty()
        {
            return Err(ProtocolError::InvalidConfig("invalid hook invocation"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub path: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookError {
    pub code: String,
    pub message: String,
    pub evidence_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HookOutcome {
    Success { artifacts: Vec<ArtifactRef> },
    Failed { error: HookError },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookResult {
    pub identity: InvocationIdentity,
    pub outcome: HookOutcome,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum HookStatus {
    Success,
    Failed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookResultWire {
    protocol_version: u32,
    requirement_id: i64,
    revision: i64,
    run_id: Option<String>,
    resource_id: String,
    invocation_id: String,
    attempt: u32,
    config_id: String,
    status: HookStatus,
    artifacts: Option<Vec<ArtifactRef>>,
    error: Option<HookError>,
}

impl Serialize for HookResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(10))?;
        serialize_result_identity_head(&mut map, &self.identity)?;
        serialize_result_identity_tail(&mut map, &self.identity)?;
        serialize_result_outcome(&mut map, &self.outcome)?;
        map.end()
    }
}

fn serialize_result_identity_head<M: SerializeMap>(
    map: &mut M,
    identity: &InvocationIdentity,
) -> Result<(), M::Error> {
    map.serialize_entry("protocol_version", &identity.protocol_version)?;
    map.serialize_entry("requirement_id", &identity.requirement_id)?;
    map.serialize_entry("revision", &identity.revision)?;
    map.serialize_entry("run_id", &identity.run_id)?;
    Ok(())
}

fn serialize_result_identity_tail<M: SerializeMap>(
    map: &mut M,
    identity: &InvocationIdentity,
) -> Result<(), M::Error> {
    map.serialize_entry("resource_id", &identity.resource_id)?;
    map.serialize_entry("invocation_id", &identity.invocation_id)?;
    map.serialize_entry("attempt", &identity.attempt)?;
    map.serialize_entry("config_id", &identity.config_id)?;
    Ok(())
}

fn serialize_result_outcome<M: SerializeMap>(
    map: &mut M,
    outcome: &HookOutcome,
) -> Result<(), M::Error> {
    match outcome {
        HookOutcome::Success { artifacts } => {
            map.serialize_entry("status", "success")?;
            map.serialize_entry("artifacts", artifacts)?;
        }
        HookOutcome::Failed { error } => {
            map.serialize_entry("status", "failed")?;
            map.serialize_entry("error", error)?;
        }
    }
    Ok(())
}

impl<'de> Deserialize<'de> for HookResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = HookResultWire::deserialize(deserializer)?;
        let outcome = match (wire.status, wire.artifacts, wire.error) {
            (HookStatus::Success, Some(artifacts), None) => HookOutcome::Success { artifacts },
            (HookStatus::Failed, None, Some(error)) => HookOutcome::Failed { error },
            _ => return Err(serde::de::Error::custom("inconsistent result status")),
        };
        Ok(Self {
            identity: InvocationIdentity {
                protocol_version: wire.protocol_version,
                requirement_id: wire.requirement_id,
                revision: wire.revision,
                run_id: wire.run_id,
                resource_id: wire.resource_id,
                invocation_id: wire.invocation_id,
                attempt: wire.attempt,
                config_id: wire.config_id,
            },
            outcome,
        })
    }
}

/// P4 observations are execution facts. Completion is an agent declaration,
/// never a validation or business outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageBasis {
    Unknown,
    Incremental,
    Cumulative,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageReport {
    pub basis: UsageBasis,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    Progress { message: String },
    Question { question_id: String, text: String },
    Completion { candidate_ref: Option<String> },
    Failure { code: String, message: String },
    ExecutionEnded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentObservation {
    pub identity: InvocationIdentity,
    pub event_id: String,
    pub event: AgentEvent,
    pub usage: UsageReport,
}

/// P6 records only adapter observations. The core still decides whether a
/// GitHub check or local commit satisfies independent acceptance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryOutcome {
    GithubPr {
        repository_id: i64,
        pr_number: u64,
        head_sha: String,
    },
    LocalGit {
        repository_id: String,
        target_ref: String,
        commit_sha: String,
    },
    Failed {
        code: String,
        message: String,
    },
    TimedOut,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryObservation {
    pub identity: InvocationIdentity,
    pub operation_id: String,
    pub outcome: DeliveryOutcome,
}

/// P7 advice is optional and confers no authority. Probability is optional
/// because different providers do not report comparable confidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionOutcome {
    Answer {
        choice: String,
        probability: Option<f64>,
        provider: String,
        model: String,
        usage: UsageReport,
    },
    Unavailable,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionObservation {
    pub identity: InvocationIdentity,
    pub schema_version: u32,
    pub outcome: DecisionOutcome,
}

/// Parses the small stdout envelope only. #104 owns filesystem/symlink checks,
/// exit status, process quiescence and unknown-side-effect reconciliation.
pub fn parse_hook_result(
    bytes: &[u8],
    expected: &InvocationIdentity,
) -> Result<HookResult, ProtocolError> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(ProtocolError::InvalidResult("result too large"));
    }
    let object: serde_json::Value = serde_json::from_slice(bytes).map_err(malformed_result)?;
    let fields = object
        .as_object()
        .ok_or(ProtocolError::InvalidResult("malformed result"))?;
    validate_result_shape(fields)?;
    let result: HookResult = serde_json::from_value(object).map_err(malformed_result)?;
    expected.matches_result(&result.identity)?;
    validate_result_payload(&result)?;
    Ok(result)
}

fn validate_result_shape(
    fields: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ProtocolError> {
    if fields.keys().any(|key| !known_result_field(key)) {
        return Err(ProtocolError::InvalidResult("unknown result field"));
    }
    match fields.get("status").and_then(serde_json::Value::as_str) {
        Some("success") if fields.contains_key("artifacts") && !fields.contains_key("error") => {}
        Some("failed") if fields.contains_key("error") && !fields.contains_key("artifacts") => {}
        _ => return Err(ProtocolError::InvalidResult("inconsistent result status")),
    }
    Ok(())
}

fn validate_result_payload(result: &HookResult) -> Result<(), ProtocolError> {
    match &result.outcome {
        HookOutcome::Success { artifacts } => {
            if artifacts
                .iter()
                .any(|a| !valid_relative_path(&a.path) || !nonempty(&a.kind))
            {
                return Err(ProtocolError::InvalidResult("invalid artifact reference"));
            }
        }
        HookOutcome::Failed { error } => {
            if !nonempty(&error.code) || error.message.trim().is_empty() {
                return Err(ProtocolError::InvalidResult("invalid hook error"));
            }
        }
    }
    Ok(())
}

fn malformed_result(_: serde_json::Error) -> ProtocolError {
    ProtocolError::InvalidResult("malformed result")
}

fn known_result_field(key: &str) -> bool {
    const FIELDS: &[&str] = &[
        "protocol_version",
        "requirement_id",
        "revision",
        "run_id",
        "resource_id",
        "invocation_id",
        "attempt",
        "config_id",
        "status",
        "artifacts",
        "error",
    ];
    FIELDS.contains(&key)
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
        && !path.contains('\\')
        && !path.contains('\0')
}
