//! Reviewed project environment values. No tool installation or host discovery.
use crate::controlled_contract::{ControlledConfig, Registration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Facts = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub controlled: ControlledConfig,
    pub host_profile_digest: String,
    pub extension_id: String,
    /// Paths are references to existing lockfiles, never copied lock contents.
    pub lockfiles: Vec<String>,
    pub roles: BTreeMap<String, Role>,
    pub ci: bool,
    pub cache: Option<Cache>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    /// Tool versions/digests, image identities, limits and scheduling settings.
    pub expected: Facts,
    /// Every service binds installed bytes, running bytes and effective config.
    pub services: BTreeMap<String, Service>,
    /// Explicitly approved alternatives for mutable runtime state only.
    pub runtime: BTreeMap<String, Vec<Value>>,
    pub checks: Vec<String>,
    pub differences: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub installation_digest: String,
    pub config_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    pub identity: String,
    pub scope: String,
    pub capacity_bytes: u64,
    pub writable: bool,
    /// Seed writing is exclusively an operator operation, never an Agent action.
    pub seed: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Difference {
    pub field: String,
    pub expected: Value,
    pub actual: Value,
    pub actual_present: bool,
}

impl Plan {
    pub fn matches_repository(&self, id: i64, version: i64) -> bool {
        if crate::plugin_scope::repository_revision(
            &self.controlled.environment.repository_revision,
        ) != Some((id, version))
        {
            return false;
        }
        let mut found = false;
        for extension in &self.controlled.extensions {
            if !crate::plugin_scope::contains(&extension.scope_ref, id) {
                return false;
            }
            found |= extension.id == self.extension_id;
        }
        found
    }

    pub fn validate(&self, approved: &[Registration]) -> Result<(), String> {
        self.controlled.validate(approved).map_err(protocol)?;
        self.controlled
            .require(
                &self.extension_id,
                &crate::controlled_contract::Operation::EnvironmentCheck,
            )
            .map_err(protocol)?;
        if self.controlled.environment.contract_digest != self.contract_digest() {
            return Err("environment contract digest mismatch".into());
        }
        self.validate_roles()?;
        self.validate_resources()
    }

    fn validate_roles(&self) -> Result<(), String> {
        for role in ["dev", "test"] {
            if !self.roles.contains_key(role) {
                return Err(format!("missing environment role: {role}"));
            }
        }
        for (name, role) in &self.roles {
            role.validate(name)?;
        }
        Ok(())
    }

    fn validate_resources(&self) -> Result<(), String> {
        for path in &self.lockfiles {
            if !relative(path) {
                return Err("invalid lockfile reference".into());
            }
        }
        if let Some(cache) = &self.cache {
            cache.validate()?;
        }
        Ok(())
    }

    pub fn digest(&self) -> String {
        crate::validation::sha256(serde_json::to_vec(self).expect("environment plan serializes"))
    }

    pub fn contract_digest(&self) -> String {
        crate::validation::sha256(
            serde_json::to_vec(&(
                &self.lockfiles,
                &self.roles,
                self.ci,
                &self.cache,
                &self.host_profile_digest,
            ))
            .expect("environment contract serializes"),
        )
    }

    pub fn expected(&self, role: &str) -> Result<Facts, String> {
        let role = self.roles.get(role).ok_or("undeclared environment role")?;
        let mut facts = role.expected.clone();
        for (name, service) in &role.services {
            for key in ["installed", "process"] {
                facts.insert(
                    format!("service.{name}.{key}"),
                    Value::String(service.installation_digest.clone()),
                );
            }
            facts.insert(
                format!("service.{name}.config"),
                Value::String(service.config_digest.clone()),
            );
            facts.insert(
                format!("service.{name}.effective_config"),
                Value::String(service.config_digest.clone()),
            );
            facts.insert(format!("service.{name}.syntax_valid"), Value::Bool(true));
            facts.insert(format!("service.{name}.semantic_valid"), Value::Bool(true));
        }
        facts.insert(
            "cache".into(),
            serde_json::to_value(&self.cache).expect("cache serializes"),
        );
        Ok(facts)
    }

    pub fn differences(&self, role: &str, actual: &Facts) -> Result<Vec<Difference>, String> {
        let mut differences = Vec::new();
        for (field, expected) in self.expected(role)? {
            let value = actual.get(&field).cloned().unwrap_or(Value::Null);
            let actual_present = actual.contains_key(&field);
            // Missing is different even when null explicitly means disabled.
            if !actual_present || value != expected {
                differences.push(Difference {
                    field,
                    expected,
                    actual: value,
                    actual_present,
                });
            }
        }
        for (field, allowed) in &self.roles[role].runtime {
            let value = actual.get(field).cloned().unwrap_or(Value::Null);
            let actual_present = actual.contains_key(field);
            if !actual_present || !allowed.contains(&value) {
                differences.push(Difference {
                    field: field.clone(),
                    expected: serde_json::json!(allowed),
                    actual: value,
                    actual_present,
                });
            }
        }
        Ok(differences)
    }
}

impl Role {
    fn validate(&self, name: &str) -> Result<(), String> {
        if !matches!(name, "dev" | "test")
            || self.checks.is_empty()
            || self.differences.trim().is_empty()
        {
            return Err("invalid role/checks or undeclared dev/test differences".into());
        }
        self.validate_checks()?;
        self.validate_fields()?;
        self.validate_runtime()?;
        self.validate_services()
    }

    fn validate_services(&self) -> Result<(), String> {
        for service in self.services.values() {
            if !digest(&service.installation_digest) || !digest(&service.config_digest) {
                return Err("service identity must be SHA-256".into());
            }
        }
        Ok(())
    }

    fn validate_checks(&self) -> Result<(), String> {
        let mut unique = std::collections::BTreeSet::new();
        for check in &self.checks {
            if check.is_empty() || !unique.insert(check) {
                return Err("invalid or duplicate environment check".into());
            }
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), String> {
        for field in self.expected.keys().chain(self.runtime.keys()) {
            if field.is_empty() || field == "cache" || field.starts_with("service.") {
                return Err("reserved or empty environment field".into());
            }
        }
        Ok(())
    }

    fn validate_runtime(&self) -> Result<(), String> {
        for (field, allowed) in &self.runtime {
            if !field.starts_with("runtime.")
                || allowed.is_empty()
                || self.expected.contains_key(field)
            {
                return Err("mutable facts must be explicit runtime state".into());
            }
        }
        Ok(())
    }
}

impl Cache {
    fn validate(&self) -> Result<(), String> {
        if !digest(&self.identity) || self.scope.is_empty() || self.capacity_bytes == 0 {
            return Err("invalid cache identity/scope/capacity".into());
        }
        if self.seed.as_ref().is_some_and(|seed| !digest(seed)) {
            return Err("invalid seed identity".into());
        }
        Ok(())
    }
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && std::path::Path::new(path)
            .components()
            .all(normal_component)
}
fn normal_component(part: std::path::Component<'_>) -> bool {
    matches!(part, std::path::Component::Normal(_))
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(crate) fn protocol(error: crate::extension_contract::ProtocolError) -> String {
    format!("environment protocol: {error:?}")
}
