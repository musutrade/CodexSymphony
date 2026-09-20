//! Deterministic requirement data. This module grants no execution authority.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    pub id: String,
    pub description: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub id: String,
    pub goal: String,
    pub scope: String,
    pub acceptance_criteria: Vec<Acceptance>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Child {
    pub id: String,
    pub parent_id: String,
    pub kind: String,
    pub order: u32,
    pub depends_on: Vec<String>,
    pub repository_id: Option<i64>,
    pub goal: String,
    pub acceptance_criteria: Vec<Acceptance>,
    pub validation_plan: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema: String,
    pub parent: Parent,
    pub children: Vec<Child>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub format: String,
    pub label: String,
    pub text: String,
}
pub fn parse(source: &Source) -> Result<Document, String> {
    if source.text.len() > 262144 || source.label.len() > 1024 {
        return Err("source exceeds size limit".into());
    }
    let text = match source.format.as_str() {
        "json" => source.text.as_str(),
        "markdown" => source
            .text
            .trim()
            .strip_prefix("# CodexSymphony Draft v1\n\n```json\n")
            .and_then(|s| s.strip_suffix("\n```"))
            .ok_or("Markdown requires the exact Draft v1 header and one json fence")?,
        _ => return Err("source format must be json or markdown".into()),
    };
    let document: Document =
        serde_json::from_str(text).map_err(|error| format!("invalid draft JSON: {error}"))?;
    validate(&document)?;
    Ok(document)
}
fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("invalid stable ID: {value}"));
    }
    Ok(())
}
fn acceptance(items: &[Acceptance]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for item in items {
        identifier(&item.id)?;
        if !ids.insert(&item.id) {
            return Err(format!("duplicate AC ID: {}", item.id));
        }
    }
    Ok(())
}
pub fn validate(document: &Document) -> Result<(), String> {
    if document.schema != "codexsymphony-draft/v1" {
        return Err("unsupported draft schema".into());
    }
    identifier(&document.parent.id)?;
    acceptance(&document.parent.acceptance_criteria)?;
    if document.children.len() > 200 {
        return Err("at most 200 children per draft".into());
    }
    let mut ids = BTreeSet::from([&document.parent.id]);
    let mut orders = BTreeSet::new();
    for child in &document.children {
        validate_child(child, &document.parent.id)?;
        if !ids.insert(&child.id) {
            return Err(format!("duplicate requirement ID: {}", child.id));
        }
        if !orders.insert(child.order) {
            return Err(format!("duplicate child order: {}", child.order));
        }
    }
    dependencies(&document.children)
}
fn validate_child(child: &Child, parent: &str) -> Result<(), String> {
    identifier(&child.id)?;
    if child.parent_id != parent {
        return Err(format!("{}: parent_id does not match parent", child.id));
    }
    if !matches!(child.kind.as_str(), "code_change" | "validation_only") {
        return Err(format!(
            "{}: kind must be code_change or validation_only",
            child.id
        ));
    }
    if child.repository_id.is_some_and(|id| id <= 0) {
        return Err(format!(
            "{}: repository_id must be positive or null",
            child.id
        ));
    }
    acceptance(&child.acceptance_criteria)
}
fn dependencies(children: &[Child]) -> Result<(), String> {
    let ids: BTreeSet<_> = children.iter().map(|child| child.id.as_str()).collect();
    for child in children {
        let mut seen = BTreeSet::new();
        for dependency in &child.depends_on {
            if dependency == &child.id {
                return Err(format!("{}: self dependency", child.id));
            }
            if !ids.contains(dependency.as_str()) || !seen.insert(dependency) {
                return Err(format!(
                    "{}: missing or duplicate dependency {dependency}",
                    child.id
                ));
            }
        }
    }
    let mut complete = BTreeSet::new();
    loop {
        let before = complete.len();
        for child in children {
            if child
                .depends_on
                .iter()
                .all(|id| complete.contains(id.as_str()))
            {
                complete.insert(child.id.as_str());
            }
        }
        if complete.len() == children.len() {
            return Ok(());
        }
        if complete.len() == before {
            return Err("dependency cycle".into());
        }
    }
}
pub fn warnings(document: &Document) -> Vec<String> {
    let mut result = Vec::new();
    missing(&mut result, "parent.goal", &document.parent.goal);
    missing(&mut result, "parent.scope", &document.parent.scope);
    ac_warnings(&mut result, "parent", &document.parent.acceptance_criteria);
    if document.children.is_empty() {
        result.push("children: 待补齐".into());
    }
    for child in &document.children {
        missing(&mut result, &format!("{}.goal", child.id), &child.goal);
        missing(
            &mut result,
            &format!("{}.validation_plan", child.id),
            &child.validation_plan,
        );
        ac_warnings(&mut result, &child.id, &child.acceptance_criteria);
        if child.repository_id.is_none() {
            result.push(format!("{}.repository_id: 待选择仓库", child.id));
        }
    }
    result
}
fn missing(result: &mut Vec<String>, field: &str, value: &str) {
    if value.trim().is_empty() {
        result.push(format!("{field}: 待补齐"));
    }
}
fn ac_warnings(result: &mut Vec<String>, field: &str, items: &[Acceptance]) {
    if items.is_empty() {
        result.push(format!("{field}.acceptance_criteria: 待补齐"));
    }
    for item in items {
        missing(result, &format!("{field}.{}", item.id), &item.description);
    }
}
