//! Group review decisions. Mapping is a reviewed claim, never business completion.
use crate::{
    budget::Amount,
    contract::{self, Contract, Criterion, Repository, Step},
    draft::{self, Document},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub parent_ac: String,
    pub child_id: String,
    pub child_revision: i64,
    pub child_ac: String,
    pub step_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub ac_id: String,
    pub step: Step,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub child_id: String,
    pub revision: i64,
    pub repository_version: i64,
    pub budget: Amount,
    pub repair_scope: String,
    pub merged_baseline_review: String,
    pub verification: Vec<Verification>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub parent_revision: i64,
    /// Every parent AC is required. These additionally require the final integration item.
    pub full_chain_acs: Vec<String>,
    pub coverage: Vec<Coverage>,
    pub items: Vec<Item>,
    pub group_budget: Option<Amount>,
    pub semantic_review: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    pub id: i64,
    pub version: i64,
    pub repository: Repository,
}
fn require(ok: bool, message: &str) -> Result<()> {
    ok.then_some(()).ok_or_else(|| message.into())
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 16000
}

pub fn validate(
    document: &Document,
    revision: i64,
    review: &Review,
    repositories: &[RepositorySnapshot],
) -> Result<Amount> {
    draft::validate(document)?;
    require(
        review.parent_revision == revision,
        "stale parent revision; reload review",
    )?;
    validate_parent(document, review)?;
    validate_order(document)?;
    validate_items(document, revision, review, repositories)?;
    validate_coverage(document, revision, review)?;
    total_budget(review)
}
fn validate_parent(document: &Document, review: &Review) -> Result<()> {
    require(
        text(&document.parent.goal) && text(&document.parent.scope),
        "parent goal and scope required",
    )?;
    require(
        !document.parent.acceptance_criteria.is_empty() && !document.children.is_empty(),
        "parent AC and children required",
    )?;
    require(
        text(&review.semantic_review),
        "user semantic coverage review required",
    )?;
    Ok(())
}
fn validate_order(document: &Document) -> Result<()> {
    for child in &document.children {
        require(child.order > 0, "order must be positive")?;
        for dependency in &child.depends_on {
            let predecessor = document
                .children
                .iter()
                .find(|c| &c.id == dependency)
                .ok_or("missing dependency")?;
            require(
                predecessor.order < child.order,
                "order must place dependencies before dependents",
            )?;
        }
    }
    Ok(())
}
fn validate_items(
    document: &Document,
    revision: i64,
    review: &Review,
    repositories: &[RepositorySnapshot],
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for item in &review.items {
        require(ids.insert(&item.child_id), "duplicate reviewed child")?;
        let child = document
            .children
            .iter()
            .find(|c| c.id == item.child_id)
            .ok_or("dangling reviewed child")?;
        require(item.revision == revision, "stale child revision")?;
        let repo = repositories
            .iter()
            .find(|r| Some(r.id) == child.repository_id)
            .ok_or("repository missing")?;
        require(
            repo.version == item.repository_version,
            "stale repository policy; review again",
        )?;
        validate_item(child, item, &repo.repository)?;
    }
    require(
        ids.len() == document.children.len(),
        "all children must be reviewed",
    )
}
fn validate_item(child: &draft::Child, item: &Item, repository: &Repository) -> Result<()> {
    require(
        text(&child.validation_plan),
        "child validation plan required",
    )?;
    require(text(&item.repair_scope), "allowed repair scope required")?;
    // Scope cannot grant new repositories, checks or repair ordinals.
    require(item.budget.positive(), "positive item budget required")?;
    let policy = &repository.policy;
    let ceiling = Amount {
        tokens: policy.token_limit,
        turns: policy.turn_limit,
        model_seconds: policy.model_work_seconds,
    };
    require(
        item.budget.fits(ceiling),
        "item budget exceeds repository policy",
    )?;
    if child.kind == "code_change" {
        require(
            text(&item.merged_baseline_review),
            "independent verification and safe merge on predecessor-merged baseline must be reviewed",
        )?;
    }
    let contract = verification_contract(child, item)?;
    contract::authorize(&contract, repository).map_err(str::to_owned)
}
pub(crate) fn verification_contract(child: &draft::Child, item: &Item) -> Result<Contract> {
    let mut acs = BTreeSet::new();
    let mut criteria = Vec::new();
    for check in &item.verification {
        require(acs.insert(&check.ac_id), "duplicate AC verification")?;
        let ac = child
            .acceptance_criteria
            .iter()
            .find(|a| a.id == check.ac_id)
            .ok_or("dangling verification AC")?;
        criteria.push(Criterion {
            description: ac.description.clone(),
            verification_ref: check.step.id.clone(),
        });
    }
    require(
        acs.len() == child.acceptance_criteria.len(),
        "every child AC requires a machine verification step",
    )?;
    Ok(Contract {
        title: child.goal.clone(),
        description: child.validation_plan.clone(),
        acceptance_criteria: criteria,
        validation_plan: item.verification.iter().map(|v| v.step.clone()).collect(),
        network_access: Vec::new(),
    })
}
fn validate_coverage(document: &Document, revision: i64, review: &Review) -> Result<()> {
    let mut covered = BTreeSet::new();
    for mapping in &review.coverage {
        require(
            mapping.child_revision == revision,
            "stale coverage revision",
        )?;
        require(
            document
                .parent
                .acceptance_criteria
                .iter()
                .any(|a| a.id == mapping.parent_ac),
            "dangling parent AC",
        )?;
        let item = review
            .items
            .iter()
            .find(|i| i.child_id == mapping.child_id)
            .ok_or("dangling coverage child")?;
        require(
            item.verification
                .iter()
                .any(|v| v.ac_id == mapping.child_ac && v.step.id == mapping.step_id),
            "dangling coverage AC or verification step",
        )?;
        covered.insert(&mapping.parent_ac);
    }
    for ac in &document.parent.acceptance_criteria {
        require(
            text(&ac.description) && covered.contains(&ac.id),
            "required parent AC missing coverage",
        )?;
    }
    validate_integration(document, review)
}
fn validate_integration(document: &Document, review: &Review) -> Result<()> {
    let final_item = document
        .children
        .iter()
        .max_by_key(child_order)
        .ok_or("children required")?;
    for ac in &review.full_chain_acs {
        require(
            document
                .parent
                .acceptance_criteria
                .iter()
                .any(|a| &a.id == ac),
            "unknown full-chain parent AC",
        )?;
        require(
            final_item.kind == "validation_only",
            "full-chain AC requires final integration item",
        )?;
        require(
            review
                .coverage
                .iter()
                .any(|m| &m.parent_ac == ac && m.child_id == final_item.id),
            "full-chain AC must map to final integration item",
        )?;
        for child in &document.children {
            if child.kind == "code_change" {
                require(
                    precedes(document, &final_item.id, &child.id),
                    "integration item must depend on every code item",
                )?;
            }
        }
    }
    Ok(())
}
fn precedes(document: &Document, child: &str, predecessor: &str) -> bool {
    let mut pending = Vec::from([child]);
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        if id == predecessor {
            return true;
        }
        if let Some(item) = document.children.iter().find(|c| c.id == id) {
            pending.extend(item.depends_on.iter().map(String::as_str));
        }
    }
    false
}
pub fn total_budget(review: &Review) -> Result<Amount> {
    let mut sum = Amount::default();
    for item in &review.items {
        sum = sum
            .checked_add(item.budget)
            .ok_or("group budget overflow")?;
    }
    let limit = review.group_budget.unwrap_or(sum);
    require(
        limit.positive() && limit.fits(sum),
        "group budget must be positive and no greater than approved item sum",
    )?;
    Ok(limit)
}
/// Reauthorization changes limits only; cumulative usage and reservations survive.
pub fn check_balance(used: Amount, reserved: Amount, limit: Amount) -> Result<()> {
    require(
        used.nonnegative() && reserved.nonnegative(),
        "invalid cumulative budget",
    )?;
    let committed = used
        .checked_add(reserved)
        .ok_or("cumulative budget overflow")?;
    require(
        committed.fits(limit),
        "budget insufficient for cumulative usage and in-flight reservations",
    )
}

fn child_order(child: &&draft::Child) -> u32 {
    child.order
}
