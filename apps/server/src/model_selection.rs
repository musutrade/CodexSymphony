//! Fixed, reviewed model choices. Deployment capabilities remain authoritative.
use crate::extension_contract::{AgentCapability, ModelConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub config: ModelConfig,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub version: String,
    pub repositories: Vec<i64>,
    pub agent: AgentCapability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frozen {
    pub selection: Selection,
    pub source: String,
    pub repository_id: i64,
    pub repository_version: i64,
    pub capability_version: String,
}

fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 200
}

pub fn validate(selection: &Selection) -> Result<()> {
    if !text(&selection.reason) || !text(&selection.config.provider) {
        return Err("model provider and selection reason required");
    }
    match &selection.config.model {
        Some(model) if text(model) => {}
        _ => return Err("explicit model ID required"),
    }
    match &selection.config.effort {
        Some(effort) if text(effort) => Ok(()),
        _ => Err("explicit model effort required"),
    }
}

impl Registration {
    pub fn admit(&self, repository: i64, config: &ModelConfig) -> Result<()> {
        if !text(&self.version) || !self.repositories.contains(&repository) {
            return Err("model capability is not authorized for this repository");
        }
        if self.agent.name != "codex" || !self.agent.reliable_stop || !self.agent.cancel {
            return Err("model adapter lacks required supervision capabilities");
        }
        if !self.agent.models.contains(config) {
            return Err("provider/model/effort is not deployed; fallback is forbidden");
        }
        Ok(())
    }
}

pub fn freeze(
    selected: Option<&Selection>,
    default: Option<&Selection>,
    repository: i64,
    version: i64,
    registration: Option<&Registration>,
) -> Result<Option<Frozen>> {
    let (selection, source) = match (selected, default) {
        (Some(selection), _) => (selection, "requirement_override"),
        (None, Some(selection)) => (selection, "project_default"),
        (None, None) => return Ok(None),
    };
    validate(selection)?;
    let registration = registration.ok_or("model capability registration unavailable")?;
    registration.admit(repository, &selection.config)?;
    Ok(Some(Frozen {
        selection: selection.clone(),
        source: source.into(),
        repository_id: repository,
        repository_version: version,
        capability_version: registration.version.clone(),
    }))
}

/// A response must acknowledge the exact selection before any paid turn starts.
pub fn check_response(frozen: &Frozen, response: &Value) -> Result<()> {
    let config = &frozen.selection.config;
    if response["model"].as_str() != config.model.as_deref()
        || response["modelProvider"].as_str() != Some(config.provider.as_str())
        || response["reasoningEffort"].as_str() != config.effort.as_deref()
    {
        return Err("Runtime acknowledged a different model configuration");
    }
    Ok(())
}

pub fn serialize_selection<S: serde::ser::SerializeStruct>(
    value: &mut S,
    selection: Option<&Selection>,
) -> std::result::Result<(), S::Error> {
    if let Some(selection) = selection {
        value.serialize_field("model_selection", selection)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/model_selection.rs"]
mod tests;
