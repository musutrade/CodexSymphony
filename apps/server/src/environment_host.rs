//! Operator-owned environment registry. Project configuration never selects an
//! executable or upgrades an installed host profile.
use crate::{controlled_contract::Registration, environment::Plan};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub registration: Registration,
    pub executable: PathBuf,
    pub approved_plans: Vec<String>,
    pub resource_root: PathBuf,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub evidence_root: PathBuf,
    pub profiles: BTreeMap<String, Profile>,
}

impl Registry {
    pub fn load() -> Result<Self> {
        let path = std::env::var_os("ENVIRONMENT_CONFIG")
            .ok_or("reviewed environment registry unavailable")?;
        Self::read(Path::new(&path))
    }
    pub fn read(path: &Path) -> Result<Self> {
        let registry: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        registry.validate()?;
        Ok(registry)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.evidence_root.is_absolute() || self.profiles.is_empty() {
            return Err("invalid environment registry".into());
        }
        for profile in self.profiles.values() {
            profile.validate()?;
        }
        let roots: Vec<_> = self
            .profiles
            .values()
            .map(|p| std::fs::canonicalize(&p.resource_root))
            .collect::<std::io::Result<_>>()?;
        self.validate_evidence_root(&roots)?;
        for (index, root) in roots.iter().enumerate() {
            if roots[..index]
                .iter()
                .any(|other| root.starts_with(other) || other.starts_with(root))
            {
                return Err("repository resource roots overlap".into());
            }
        }
        Ok(())
    }
    fn validate_evidence_root(&self, roots: &[PathBuf]) -> Result<()> {
        let actual = self.canonical_evidence_root()?;
        if actual != self.evidence_root {
            return Err("environment evidence root must be canonical".into());
        }
        if roots
            .iter()
            .any(|root| root.starts_with(&actual) || actual.starts_with(root))
        {
            return Err("environment evidence and project resources overlap".into());
        }
        Ok(())
    }
    fn canonical_evidence_root(&self) -> Result<PathBuf> {
        if self.evidence_root.try_exists()? {
            if !self.evidence_root.is_dir() {
                return Err("environment evidence root must be a directory".into());
            }
            Ok(std::fs::canonicalize(&self.evidence_root)?)
        } else {
            let parent = self
                .evidence_root
                .parent()
                .ok_or("environment evidence parent missing")?;
            Ok(std::fs::canonicalize(parent)?.join(
                self.evidence_root
                    .file_name()
                    .ok_or("environment evidence directory name missing")?,
            ))
        }
    }
    pub fn resolve(&self, plan: &Plan) -> Result<&Profile> {
        let profile = self
            .profiles
            .get(&plan.controlled.environment.host_profile_ref)
            .ok_or("unapproved host profile")?;
        if plan.host_profile_digest != profile.identity(&self.evidence_root) {
            return Err(format!(
                "host profile drift: expected {}, actual {}",
                plan.host_profile_digest,
                profile.identity(&self.evidence_root)
            )
            .into());
        }
        plan.validate(std::slice::from_ref(&profile.registration))?;
        if profile.registration.id != plan.extension_id
            || !profile.approved_plans.contains(&plan.digest())
        {
            return Err("environment plan has not been approved on host".into());
        }
        if plan
            .cache
            .as_ref()
            .is_some_and(|cache| cache.scope != plan.controlled.environment.host_profile_ref)
        {
            return Err("cache scope differs from approved repository profile".into());
        }
        profile.verify_installation()?;
        Ok(profile)
    }
}

impl Profile {
    pub fn identity(&self, evidence_root: &Path) -> String {
        crate::validation::sha256(
            serde_json::to_vec(&(
                &self.registration,
                &self.executable,
                &self.resource_root,
                self.timeout_seconds,
                evidence_root,
            ))
            .expect("host profile serializes"),
        )
    }
    fn validate(&self) -> Result<()> {
        if !self.executable.is_absolute()
            || !self.resource_root.is_absolute()
            || !(1..=600).contains(&self.timeout_seconds)
        {
            return Err("invalid environment executable/root/timeout".into());
        }
        if self.registration.credential_provider_ref.is_some() {
            return Err("environment probes cannot request delivery credentials".into());
        }
        if std::fs::canonicalize(&self.resource_root)? != self.resource_root {
            return Err("repository resource root must be canonical".into());
        }
        if std::fs::canonicalize(&self.executable)?.starts_with(&self.resource_root) {
            return Err("environment probe cannot be installed in project resources".into());
        }
        self.verify_installation()
    }
    pub fn verify_installation(&self) -> Result<()> {
        let actual = crate::validation::sha256(std::fs::read(&self.executable)?);
        if actual != self.registration.implementation_digest {
            return Err(format!(
                "environment extension drift: expected {}, actual {actual}",
                self.registration.implementation_digest
            )
            .into());
        }
        Ok(())
    }
}
