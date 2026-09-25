//! Reviewed development constraints are task data, never plugin authority.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub id: String,
    pub version: String,
    pub source: String,
    pub reason: String,
    pub instruction: String,
    pub paths: Vec<String>,
    pub code_scope: CodeScope,
    pub release_condition: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeScope {
    ChangedProduction,
    ChangedTests,
    SpecifiedFiles,
}

pub fn validate(constraints: &[Constraint]) -> Result<(), &'static str> {
    if constraints.len() > 32 {
        return Err("too many development constraints");
    }
    let mut ids = std::collections::HashSet::new();
    for constraint in constraints {
        validate_one(constraint)?;
        if !ids.insert(&constraint.id) {
            return Err("duplicate development constraint");
        }
    }
    Ok(())
}

fn validate_one(c: &Constraint) -> Result<(), &'static str> {
    for value in [
        &c.id,
        &c.version,
        &c.source,
        &c.reason,
        &c.instruction,
        &c.release_condition,
    ] {
        if value.trim().is_empty() || value.len() > 4096 {
            return Err("constraint requires bounded identity, provenance and release condition");
        }
    }
    if c.paths.is_empty() || c.paths.len() > 64 {
        return Err("constraint requires explicit paths");
    }
    for path in &c.paths {
        validate_path(path)?;
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > 1024 || path.starts_with('/') || path.contains('\\') {
        return Err("constraint path must be repository relative");
    }
    for component in std::path::Path::new(path).components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("constraint path may not escape its scope");
        }
    }
    Ok(())
}
