use codexsymphony_server::{
    controlled_contract::*,
    delivery_extension::{self, Adapter, Approval, Control, Reply, Request, Result},
    extension_contract::*,
};
use std::sync::{Arc, Mutex};
fn registration() -> Registration {
    Registration {
        id: "local-fixture".into(),
        implementation_digest: "a".repeat(64),
        operations: vec![
            Operation::CapabilityCheck,
            Operation::Submit,
            Operation::Observe,
            Operation::Reconcile,
        ],
        scope_ref: "local".into(),
        config_ref: "reviewed".into(),
        credential_provider_ref: None,
    }
}
fn config() -> ControlledConfig {
    ControlledConfig {
        protocol_version: 1,
        environment: EnvironmentBinding {
            repository_revision: "local@1".into(),
            contract_digest: "b".repeat(64),
            host_profile_ref: "no-db-no-cache".into(),
            role: "test".into(),
        },
        extensions: vec![registration()],
    }
}
fn frozen() -> FrozenConfig {
    let mut capabilities = Capabilities::legacy_codex(None);
    capabilities.deliveries = vec![DeliveryMode::LocalGit];
    ExtensionConfig {
        protocol_version: 1,
        agent: "codex".into(),
        model: ModelConfig {
            provider: "codex".into(),
            model: None,
            effort: None,
        },
        delivery: DeliveryMode::LocalGit,
        hooks: vec![],
        decision: None,
    }
    .freeze(&capabilities)
    .unwrap()
}
fn request(operation: Operation) -> Request {
    Request {
        call: Call {
            identity: InvocationIdentity {
                protocol_version: 1,
                requirement_id: 1,
                revision: 2,
                run_id: Some("run".into()),
                resource_id: "local".into(),
                invocation_id: "original".into(),
                attempt: 1,
                config_id: frozen().config_id,
            },
            controlled_config_digest: config().freeze(&[registration()]).unwrap(),
            operation,
            extension_id: registration().id,
            implementation_digest: registration().implementation_digest,
            candidate: Some(SourceIdentity {
                commit: "commit".into(),
                tree: "tree".into(),
            }),
            environment_digest: "c".repeat(64),
            policy_digest: "d".repeat(64),
            deadline_unix_ms: 100,
            required_checks: vec![],
        },
        target_ref: "local-main".into(),
        operation_ref: "publish-original".into(),
    }
}
#[derive(Default)]
struct Fixture {
    log: Arc<Mutex<Vec<&'static str>>>,
    fail: &'static str,
    admits: usize,
    unknown: bool,
    changed: bool,
}
impl Fixture {
    fn event(&self, name: &'static str) {
        self.log.lock().unwrap().push(name);
    }
}
impl Adapter for Fixture {
    type Input = Request;
    type Facts = Evaluation;
    async fn capability_check(&mut self, r: &Request) -> Result<Reply<Request, Evaluation>> {
        self.event("capability");
        self.reply(r)
    }
    async fn submit(&mut self, r: &Request) -> Result<Reply<Request, Evaluation>> {
        self.event("submit");
        self.reply(r)
    }
    async fn observe(&mut self, r: &Request) -> Result<Reply<Request, Evaluation>> {
        self.event("observe");
        self.reply(r)
    }
    async fn reconcile(&mut self, r: &Request) -> Result<Reply<Request, Evaluation>> {
        self.event("reconcile");
        self.reply(r)
    }
}
impl Fixture {
    fn reply(&self, r: &Request) -> Result<Reply<Request, Evaluation>> {
        if self.fail == "send" {
            return Err("response lost".into());
        }
        let mut bound = r.clone();
        if self.changed {
            bound.target_ref = "other".into();
        }
        Ok(Reply {
            request: bound,
            facts: Evaluation {
                call: r.call.clone(),
                verdict: Verdict::Pass,
                checks: vec![],
            },
        })
    }
}
impl Control<Request, Evaluation> for Fixture {
    fn check(&self, op: &Operation, r: &Request) -> Result<()> {
        self.event("check");
        if op != &r.call.operation {
            return Err("operation changed".into());
        }
        r.check(&Approval {
            frozen: &frozen(),
            controlled: &config(),
            installed: &[registration()],
            target_ref: "local-main",
            operation_ref: "publish-original",
        })
        .map_err(|e| format!("{e:?}").into())
    }
    async fn admit(&mut self, _: &Request) -> Result<()> {
        self.event("admit");
        self.admits += 1;
        if self.fail == "validation" || (self.fail == "revoked" && self.admits == 2) {
            return Err("not admitted".into());
        }
        Ok(())
    }
    async fn before_deliver(&mut self, _: &Request) -> Result<()> {
        self.event("hook");
        if self.fail == "hook" {
            return Err("auxiliary failure".into());
        }
        Ok(())
    }
    async fn begin(&mut self, _: &Request) -> Result<bool> {
        self.event("intent");
        Ok(!self.unknown)
    }
    async fn retain(&mut self, _: &Request, _: &Result<Reply<Request, Evaluation>>) -> Result<()> {
        self.event("retain");
        Ok(())
    }
    async fn verify(&mut self, r: &Request, reply: &Reply<Request, Evaluation>) -> Result<()> {
        self.event("verify");
        reply
            .facts
            .check_pass(&r.call, 10)
            .map_err(|e| format!("{e:?}").into())
    }
    async fn post_delivery_validate(
        &mut self,
        _: &Request,
        _: &Reply<Request, Evaluation>,
    ) -> Result<()> {
        self.event("post");
        if self.fail == "post" {
            return Err("post-delivery validation failed".into());
        }
        Ok(())
    }
}
#[tokio::test]
async fn local_adapter_has_no_vendor_credentials_or_pr_fields_and_dispatches_only_approved_operations()
 {
    for (op, name) in [
        (Operation::CapabilityCheck, "capability"),
        (Operation::Submit, "submit"),
        (Operation::Observe, "observe"),
        (Operation::Reconcile, "reconcile"),
    ] {
        let mut control = Fixture::default();
        let mut adapter = Fixture {
            log: control.log.clone(),
            ..Default::default()
        };
        assert!(
            delivery_extension::invoke(&mut adapter, &mut control, op.clone(), &request(op))
                .await
                .unwrap()
                .is_some()
        );
        let log = control.log.lock().unwrap();
        if name == "submit" {
            assert_eq!(
                *log,
                vec![
                    "check", "admit", "hook", "admit", "intent", "submit", "retain", "verify",
                    "post"
                ]
            );
        } else {
            assert_eq!(*log, vec!["check", name, "retain", "verify", "post"]);
        }
    }
}
#[tokio::test]
async fn validation_auxiliary_failure_revocation_and_unknown_intent_prevent_sends() {
    for fail in ["validation", "hook", "revoked", ""] {
        let mut control = Fixture {
            fail,
            unknown: fail.is_empty(),
            ..Default::default()
        };
        let mut adapter = Fixture {
            log: control.log.clone(),
            ..Default::default()
        };
        let result = delivery_extension::invoke(
            &mut adapter,
            &mut control,
            Operation::Submit,
            &request(Operation::Submit),
        )
        .await;
        if fail.is_empty() {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err());
        }
        assert!(!control.log.lock().unwrap().contains(&"submit"));
    }
}
#[tokio::test]
async fn lost_or_wrong_identity_responses_are_retained_without_approval() {
    for changed in [false, true] {
        let mut control = Fixture::default();
        let mut adapter = Fixture {
            log: control.log.clone(),
            changed,
            fail: if changed { "" } else { "send" },
            ..Default::default()
        };
        assert!(
            delivery_extension::invoke(
                &mut adapter,
                &mut control,
                Operation::Submit,
                &request(Operation::Submit)
            )
            .await
            .is_err()
        );
        assert_eq!(control.log.lock().unwrap().last(), Some(&"retain"));
    }
}
#[tokio::test]
async fn malformed_selection_and_operation_fail_before_any_hook_or_intent() {
    let mut control = Fixture::default();
    let mut adapter = Fixture::default();
    for mut r in [request(Operation::Validate), request(Operation::Submit)] {
        if r.call.operation == Operation::Submit {
            r.target_ref = "unreviewed".into();
        }
        assert!(
            delivery_extension::invoke(&mut adapter, &mut control, r.call.operation.clone(), &r)
                .await
                .is_err()
        );
    }
    assert_eq!(*control.log.lock().unwrap(), vec!["check", "check"]);
}

#[tokio::test]
async fn post_delivery_failure_keeps_the_original_side_effect_receipt() {
    let mut control = Fixture {
        fail: "post",
        ..Default::default()
    };
    let mut adapter = Fixture {
        log: control.log.clone(),
        ..Default::default()
    };
    assert!(
        delivery_extension::invoke(
            &mut adapter,
            &mut control,
            Operation::Submit,
            &request(Operation::Submit)
        )
        .await
        .is_err()
    );
    let log = control.log.lock().unwrap();
    assert_eq!(log.iter().filter(|&&event| event == "submit").count(), 1);
    assert_eq!(&log[log.len() - 3..], &["retain", "verify", "post"]);
}
