use super::*;
use crate::github::{Capability, Observation};
use crate::github_http::{Error, invalid};

struct Recovery {
    fail: bool,
}
impl Remote for Recovery {
    async fn observe(&mut self, _: &Intent) -> std::result::Result<Observation, Error> {
        Err(invalid())
    }
    async fn preflight(&mut self, _: &Intent) -> std::result::Result<Capability, Error> {
        Err(invalid())
    }
    async fn pr(&mut self, _: &Intent) -> std::result::Result<Value, Error> {
        Err(invalid())
    }
    async fn merge(&mut self, _: &Intent) -> std::result::Result<Value, Error> {
        panic!("recovery must never repeat merge")
    }
    async fn reconcile(&mut self, intent: &Intent) -> std::result::Result<Observation, Error> {
        if self.fail {
            return Err(invalid());
        }
        Ok(serde_json::from_value(json!({
            "policy":intent.policy, "repository_id":7, "number":12,
            "head":intent.head, "base":intent.base, "head_ref":intent.branch,
            "base_ref":"main", "test_merge_sha":null, "merged_sha":"merged",
            "merge":"Merged", "closed":true, "checks":[], "last_synced_at":100,
            "actual_checkout_sha":null, "phases":null
        }))
        .unwrap())
    }
}

#[tokio::test]
async fn reconciliation_preserves_original_identity_and_remote_errors() {
    let intent: Intent = serde_json::from_value(json!({
        "delivery_key":"delivery", "requirement":1, "revision":2, "authorization":null,
        "policy":{"repository_id":7,"repository":"owner/repo","default_branch":"main",
            "version":1,"required":[],"wait_seconds":900,"delivery":null},
        "pr":12,"head":"candidate","base":"base","checkout_sha":null,
        "branch":"ai/candidate","validation_id":"validation","dependencies":{}
    }))
    .unwrap();
    let mut remote = Recovery { fail: false };
    let reply = Merge {
        remote: &mut remote,
        now: 100,
    }
    .reconcile(&intent)
    .await
    .unwrap();
    assert_eq!(reply.request, intent);
    assert_eq!(reply.facts["merged_sha"], "merged");
    remote.fail = true;
    assert!(
        Merge {
            remote: &mut remote,
            now: 100
        }
        .reconcile(&intent)
        .await
        .is_err()
    );
}
