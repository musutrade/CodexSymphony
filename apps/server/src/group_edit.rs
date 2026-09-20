//! Pure queue change analysis. Review decisions do not create completion facts.
use crate::{draft::Document, group_review::Review};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn reorder(document: &Document, order: &[String]) -> Result<Document, String> {
    let mut result = document.clone();
    let unique: BTreeSet<_> = order.iter().collect();
    if unique.len() != order.len() || order.len() != result.children.len() {
        return Err("order must contain every child exactly once".into());
    }
    for child in &mut result.children {
        child.order = order
            .iter()
            .position(|id| id == &child.id)
            .ok_or("order contains unknown or missing child")? as u32
            + 1;
    }
    crate::draft::validate(&result)?;
    crate::group_review::validate_order(&result)?;
    Ok(result)
}

fn items(document: &Document, review: &Review) -> BTreeMap<String, Value> {
    document
        .children
        .iter()
        .map(|child| {
            let mut content = json!(child);
            content.as_object_mut().unwrap().remove("order");
            let mut policy = review
                .items
                .iter()
                .find(|item| item.child_id == child.id)
                .map(|item| json!(item));
            if let Some(value) = policy.as_mut() {
                value.as_object_mut().unwrap().remove("revision");
            }
            let coverage: Vec<_> = review
                .coverage
                .iter()
                .filter(|m| m.child_id == child.id)
                .map(|m| json!([m.parent_ac, m.child_ac, m.step_id]))
                .collect();
            (child.id.clone(), json!([content, policy, coverage]))
        })
        .collect()
}

pub fn affected(
    before: &Document,
    old: &Review,
    after: &Document,
    new: &Review,
) -> BTreeSet<String> {
    let previous = items(before, old);
    let next = items(after, new);
    let mut changed: BTreeSet<String> = previous
        .keys()
        .chain(next.keys())
        .filter(|id| previous.get(*id) != next.get(*id))
        .cloned()
        .collect();
    if json!(before.parent) != json!(after.parent)
        || old.full_chain_acs != new.full_chain_acs
        || old.group_budget != new.group_budget
    {
        changed.extend(previous.keys().chain(next.keys()).cloned());
    }
    loop {
        let size = changed.len();
        for document in [before, after] {
            for child in &document.children {
                if depends_on_changed(document, child, &changed) {
                    changed.insert(child.id.clone());
                }
            }
        }
        if changed.len() == size {
            return changed;
        }
    }
}

fn depends_on_changed(
    document: &Document,
    child: &crate::draft::Child,
    changed: &BTreeSet<String>,
) -> bool {
    if child.depends_on.iter().any(|id| changed.contains(id)) {
        return true;
    }
    // The scheduler also binds every earlier same-repository item as baseline evidence.
    document.children.iter().any(|previous| {
        previous.repository_id == child.repository_id
            && previous.order < child.order
            && changed.contains(&previous.id)
    })
}
