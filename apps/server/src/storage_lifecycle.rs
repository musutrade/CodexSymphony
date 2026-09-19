//! Storage decisions only. Execution, business and validation results are never
//! inferred from the availability of a material.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Workspace,
    Hot,
    Cold,
    Database,
    Record,
}
pub const CATEGORIES: [Category; 5] = [
    Category::Workspace,
    Category::Hot,
    Category::Cold,
    Category::Database,
    Category::Record,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    pub bytes: u64,
    pub seconds: u64,
    pub reserve_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: String,
    pub reason: String,
    pub global_bytes: u64,
    pub control_bytes: u64,
    pub run_bytes: u64,
    pub requirement_bytes: u64,
    pub entry_bytes: u64,
    pub entry_count: u64,
    pub categories: BTreeMap<Category, Limit>,
}

fn finite(value: u64) -> bool {
    value > 0 && value <= i64::MAX as u64
}

impl Policy {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version.is_empty() || self.reason.is_empty() {
            return Err("storage policy version and reason required");
        }
        let limits = [
            self.global_bytes,
            self.control_bytes,
            self.run_bytes,
            self.requirement_bytes,
            self.entry_bytes,
            self.entry_count,
        ];
        if !limits.into_iter().all(finite) {
            return Err("finite positive storage limits required");
        }
        if self.control_bytes >= self.global_bytes {
            return Err("control reserve must leave task capacity");
        }
        validate_categories(&self.categories)
    }
}
fn validate_categories(categories: &BTreeMap<Category, Limit>) -> Result<(), &'static str> {
    for category in CATEGORIES {
        let limit = categories.get(&category).ok_or("missing category policy")?;
        if !finite(limit.bytes) || !finite(limit.seconds) || !finite(limit.reserve_bytes) {
            return Err("finite category capacity and retention required");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub actual: u64,
    pub reserved: u64,
    pub category_actual: u64,
    pub category_reserved: u64,
    pub run_allocated: u64,
    pub requirement_allocated: u64,
    pub available: u64,
    pub unknown: bool,
}

fn fits(values: &[u64], limit: u64) -> bool {
    values
        .iter()
        .try_fold(0u64, |total, value| total.checked_add(*value))
        .is_some_and(|total| total <= limit)
}

pub fn admit(policy: &Policy, usage: &Usage, category: Category, bytes: u64) -> bool {
    if policy.validate().is_err() || usage.unknown || !finite(bytes) {
        return false;
    }
    let limit = &policy.categories[&category];
    let checks = [
        fits(
            &[usage.actual, usage.reserved, bytes, policy.control_bytes],
            policy.global_bytes,
        ),
        fits(
            &[usage.category_actual, usage.category_reserved, bytes],
            limit.bytes,
        ),
        fits(&[usage.run_allocated, bytes], policy.run_bytes),
        fits(
            &[usage.requirement_allocated, bytes],
            policy.requirement_bytes,
        ),
        fits(
            &[usage.reserved, bytes, policy.control_bytes],
            usage.available,
        ),
    ];
    checks.into_iter().all(std::convert::identity)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub repository: String,
    pub requirement: i64,
    pub revision: i64,
    pub run: String,
    pub attempt: u64,
    pub candidate: Option<String>,
    pub stage: String,
    pub policy: String,
    pub pr: Option<u64>,
}

/// An explicit predecessor link is necessary even when all other fields match.
/// A new candidate may resolve an older candidate only through that link.
pub fn replaces(older: &Identity, newer: &Identity, predecessor: &str) -> bool {
    let same = [
        older.repository == newer.repository,
        older.requirement == newer.requirement,
        older.revision == newer.revision,
        older.stage == newer.stage,
        older.policy == newer.policy,
        older.pr.is_none() || older.pr == newer.pr,
        older.run == predecessor,
        older.attempt < newer.attempt,
    ];
    !older.repository.is_empty()
        && !older.policy.is_empty()
        && same.into_iter().all(std::convert::identity)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Decision,
    Retrospective,
    Recovery,
    Rebuildable,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Protection {
    pub active: bool,
    pub consumer: bool,
    pub unique: bool,
    pub unknown: bool,
    pub unreconciled: bool,
    pub current_success: bool,
}

impl Protection {
    pub fn reason(&self) -> Option<&'static str> {
        [
            (self.active, "active process"),
            (self.consumer, "active consumer"),
            (self.unique, "unique work"),
            (self.unknown, "unknown identity"),
            (self.unreconciled, "partial/pending requires reconciliation"),
            (self.current_success, "current successful retrospective"),
        ]
        .into_iter()
        .find_map(|(blocked, reason)| blocked.then_some(reason))
    }
}

pub fn expired(now: i64, expires_at: i64, protection: &Protection) -> bool {
    now >= expires_at && protection.reason().is_none()
}
