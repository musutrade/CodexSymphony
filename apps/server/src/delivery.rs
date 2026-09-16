//! Delivery decisions depend on exact identities, never on execution success.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub requirement: i64,
    pub revision: i64,
    pub repository_id: u64,
    pub repository: String,
    pub branch: String,
    pub base_branch: String,
    pub head: String,
}
impl Identity {
    pub fn action_key(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("identity JSON"))
        )
    }
    pub fn marker(&self) -> String {
        format!("<!-- codexsymphony-delivery:{} -->", self.action_key())
    }
    pub fn matches_pr(&self, pr: &Value) -> bool {
        pr["base"]["repo"]["id"] == self.repository_id
            && pr["head"]["repo"]["id"] == self.repository_id
            && pr["base"]["repo"]["full_name"] == self.repository
            && pr["head"]["ref"] == self.branch
            && pr["base"]["ref"] == self.base_branch
            && pr["head"]["sha"] == self.head
            && pr["body"]
                .as_str()
                .is_some_and(|s| s.contains(&self.marker()))
            && pr["number"].as_u64().is_some_and(|n| n > 0)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PrFact {
    Open,
    Closed,
    Merged,
    Unknown,
    Conflict,
}
pub fn pr_fact(identity: &Identity, pr: &Value) -> PrFact {
    if !identity.matches_pr(pr) {
        return PrFact::Conflict;
    }
    match crate::github::merge_fact(pr) {
        crate::github::MergeFact::Merged => PrFact::Merged,
        crate::github::MergeFact::Unknown => PrFact::Unknown,
        crate::github::MergeFact::Unmerged => match pr["state"].as_str() {
            Some("open") => PrFact::Open,
            Some("closed") => PrFact::Closed,
            _ => PrFact::Unknown,
        },
    }
}
pub fn failure(code: &str, phase: &str, attempts: i32, now: i64, status: Option<u16>) -> Value {
    let delay = if attempts == 1 { 30 } else { 120 };
    json!({"code":code,"phase":phase,"attempts":attempts,"http_status":status,
        "next_attempt_at":now+delay,"evidence":"delivery_attempt"})
}
