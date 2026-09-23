use codexsymphony_server::{
    contract::{Policy, Repository},
    extension_contract::{
        AgentCapability, Capabilities, DeliveryMode, ExtensionConfig, HookConfig, HookEvent,
        HookInvocation, HookOutcome, HookResult, HookRole, InvocationIdentity, ModelConfig,
        ProtocolError, ReplayPolicy, parse_hook_result,
    },
};

fn legacy() -> Repository {
    Repository {
        model: Some("reviewed-model".into()),
        project: "example".into(),
        remote: "owner/repo".into(),
        github_repository_id: 42,
        base_branch: "main".into(),
        policy: Policy {
            allowed_checks: vec!["cargo_test".into()],
            max_timeout_seconds: 60,
            token_limit: 1000,
            turn_limit: 10,
            model_work_seconds: 60,
            gate_recovery_policy: "bounded_v1".into(),
        },
        revoked: false,
        reason: "approved".into(),
    }
}

fn identity(config_id: String) -> InvocationIdentity {
    InvocationIdentity {
        protocol_version: 1,
        requirement_id: 7,
        revision: 2,
        run_id: Some("run-9".into()),
        resource_id: "workspace-9".into(),
        invocation_id: "hook-42".into(),
        attempt: 1,
        config_id,
    }
}

#[test]
fn legacy_repository_maps_without_changing_its_execution_or_delivery_choice() {
    let old = legacy();
    let config = ExtensionConfig::from_legacy_repository(&old);
    assert_eq!(config.protocol_version, 1);
    assert_eq!(config.agent, "codex");
    assert_eq!(config.model.provider, "codex");
    assert_eq!(config.model.model, old.model);
    assert_eq!(config.model.effort, None);
    assert_eq!(config.delivery, DeliveryMode::GithubPr);
    assert!(config.hooks.is_empty());
    assert_eq!(config.decision, None);
    let frozen = config
        .freeze(&Capabilities::legacy_codex(old.model.clone()))
        .unwrap();
    frozen.validate_identity().unwrap();

    let mut no_model = old;
    no_model.model = None;
    assert_eq!(
        ExtensionConfig::from_legacy_repository(&no_model)
            .model
            .model,
        None
    );
}

#[test]
fn version_and_unregistered_capabilities_fail_before_freezing() {
    let config = ExtensionConfig::from_legacy_repository(&legacy());
    let allowed = Capabilities::legacy_codex(config.model.model.clone());
    let mut unknown = config.clone();
    unknown.protocol_version = 99;
    assert_eq!(
        unknown.freeze(&allowed),
        Err(ProtocolError::UnsupportedVersion(99))
    );
    let mut local = config.clone();
    local.delivery = DeliveryMode::LocalGit;
    assert_eq!(
        local.freeze(&allowed),
        Err(ProtocolError::UnsupportedCapability("delivery"))
    );
    let mut other_model = config.clone();
    other_model.model = ModelConfig {
        provider: "codex".into(),
        model: Some("unreviewed".into()),
        effort: Some("high".into()),
    };
    assert_eq!(
        other_model.freeze(&allowed),
        Err(ProtocolError::UnsupportedCapability("agent/model"))
    );
    let mut split_registration = allowed.clone();
    split_registration.agents.push(AgentCapability {
        name: "other-agent".into(),
        models: vec![other_model.model.clone()],
        reliable_stop: true,
        resume: false,
        cancel: true,
        structured_events: true,
        usage_reporting: false,
    });
    assert_eq!(
        other_model.freeze(&split_registration),
        Err(ProtocolError::UnsupportedCapability("agent/model"))
    );
    let mut no_stop = allowed.clone();
    no_stop.agents[0].reliable_stop = false;
    assert_eq!(
        config.freeze(&no_stop),
        Err(ProtocolError::UnsupportedCapability("reliable stop"))
    );
    no_stop.agents[0].reliable_stop = true;
    no_stop.agents[0].resume = false;
    assert_eq!(
        no_stop.require_resume("codex"),
        Err(ProtocolError::UnsupportedCapability("resume"))
    );
    let mut advisory = config;
    advisory.decision = Some("optional-advisor".into());
    assert_eq!(
        advisory.freeze(&allowed),
        Err(ProtocolError::UnsupportedCapability("decision"))
    );
}

#[test]
fn script_registration_and_frozen_identity_reject_workspace_or_default_changes() {
    let mut config = ExtensionConfig::from_legacy_repository(&legacy());
    let mut allowed = Capabilities::legacy_codex(config.model.model.clone());
    let hook = HookConfig {
        name: "prepare".into(),
        event: HookEvent::BeforeRun,
        roles: vec![HookRole::Coding],
        argv: vec!["/trusted/prepare".into()],
        script_identity: "sha256:reviewed-content-and-deps".into(),
        timeout_seconds: 30,
        output_limit_bytes: 1024,
        replay: ReplayPolicy::Reconcile,
    };
    config.hooks.push(hook.clone());
    assert_eq!(
        config.freeze(&allowed),
        Err(ProtocolError::UnsupportedCapability("hook registration"))
    );
    allowed.hooks.push(hook);
    let frozen = config.freeze(&allowed).unwrap();
    config.hooks[0].script_identity = "sha256:workspace-edit".into();
    assert_eq!(
        config.freeze(&allowed),
        Err(ProtocolError::UnsupportedCapability("hook registration"))
    );
    assert_eq!(frozen.value.hooks[0].argv, vec!["/trusted/prepare"]);
    assert_ne!(
        frozen.config_id,
        config
            .freeze(&{
                let mut changed = allowed.clone();
                changed.hooks = config.hooks.clone();
                changed
            })
            .unwrap()
            .config_id
    );
    frozen.validate_identity().unwrap();
    let mut tampered = frozen;
    tampered.value.model.model = Some("changed-after-review".into());
    assert_eq!(
        tampered.validate_identity(),
        Err(ProtocolError::IdentityMismatch("config_id"))
    );
}

#[test]
fn hook_input_is_versioned_and_bound_to_registered_event_role() {
    assert_eq!(ReplayPolicy::default(), ReplayPolicy::Never);
    let omitted_replay = serde_json::json!({
        "name": "prepare", "event": "before_run", "roles": ["coding"],
        "argv": ["/trusted/prepare"], "script_identity": "sha256:reviewed",
        "timeout_seconds": 30, "output_limit_bytes": 1024
    });
    assert_eq!(
        serde_json::from_value::<HookConfig>(omitted_replay)
            .unwrap()
            .replay,
        ReplayPolicy::Never
    );
    let mut config = ExtensionConfig::from_legacy_repository(&legacy());
    let hook = HookConfig {
        name: "prepare".into(),
        event: HookEvent::BeforeRun,
        roles: vec![HookRole::Coding],
        argv: vec!["/trusted/prepare".into()],
        script_identity: "sha256:reviewed".into(),
        timeout_seconds: 30,
        output_limit_bytes: 1024,
        replay: ReplayPolicy::Never,
    };
    config.hooks.push(hook.clone());
    let mut allowed = Capabilities::legacy_codex(config.model.model.clone());
    allowed.hooks.push(hook.clone());
    let frozen = config.freeze(&allowed).unwrap();
    let input = HookInvocation {
        identity: identity(frozen.config_id.clone()),
        event: HookEvent::BeforeRun,
        role: HookRole::Coding,
        workspace: "/work/project".into(),
        output_dir: "/work/outputs/hook-42".into(),
        deadline_at: "2026-09-23T12:00:00Z".into(),
        context: serde_json::Map::new(),
    };
    input.validate(&frozen, &hook).unwrap();
    let encoded = serde_json::to_vec(&input).unwrap();
    assert_eq!(
        serde_json::from_slice::<HookInvocation>(&encoded).unwrap(),
        input
    );
    let mut wrong = input.clone();
    wrong.role = HookRole::Repair;
    assert_eq!(
        wrong.validate(&frozen, &hook),
        Err(ProtocolError::UnsupportedCapability("hook event/role"))
    );
    wrong = input;
    wrong.identity.protocol_version = 2;
    assert_eq!(
        wrong.validate(&frozen, &hook),
        Err(ProtocolError::UnsupportedVersion(2))
    );
}

#[test]
fn response_version_identity_shape_and_size_are_checked() {
    let config = ExtensionConfig::from_legacy_repository(&legacy());
    let frozen = config
        .freeze(&Capabilities::legacy_codex(config.model.model.clone()))
        .unwrap();
    let expected = identity(frozen.config_id.clone());
    expected.validate(&frozen, false).unwrap();
    let success = HookResult {
        identity: expected.clone(),
        outcome: HookOutcome::Success { artifacts: vec![] },
    };
    let bytes = serde_json::to_vec(&success).unwrap();
    assert_eq!(parse_hook_result(&bytes, &expected), Ok(success.clone()));
    let mut mixed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    mixed["error"] = serde_json::json!({"code":"x","message":"wrong","evidence_ref":null});
    assert_eq!(
        parse_hook_result(&serde_json::to_vec(&mixed).unwrap(), &expected),
        Err(ProtocolError::InvalidResult("inconsistent result status"))
    );
    let mut extra: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    extra["unreviewed_field"] = serde_json::json!(true);
    assert_eq!(
        parse_hook_result(&serde_json::to_vec(&extra).unwrap(), &expected),
        Err(ProtocolError::InvalidResult("unknown result field"))
    );
    let mut traversal = success.clone();
    traversal.outcome = HookOutcome::Success {
        artifacts: vec![codexsymphony_server::extension_contract::ArtifactRef {
            path: "../secret".into(),
            kind: "log".into(),
        }],
    };
    assert_eq!(
        parse_hook_result(&serde_json::to_vec(&traversal).unwrap(), &expected),
        Err(ProtocolError::InvalidResult("invalid artifact reference"))
    );
    let mut late = success.clone();
    late.identity.attempt = 2;
    assert_eq!(
        parse_hook_result(&serde_json::to_vec(&late).unwrap(), &expected),
        Err(ProtocolError::IdentityMismatch("result identity"))
    );
    late.identity.protocol_version = 2;
    assert_eq!(
        parse_hook_result(&serde_json::to_vec(&late).unwrap(), &expected),
        Err(ProtocolError::UnsupportedVersion(2))
    );
    assert_eq!(
        parse_hook_result(&vec![b'x'; 65_537], &expected),
        Err(ProtocolError::InvalidResult("result too large"))
    );
    assert_eq!(
        parse_hook_result(b"{\"status\":\"timeout\"}", &expected),
        Err(ProtocolError::InvalidResult("inconsistent result status"))
    );
    let missing_run = InvocationIdentity {
        run_id: None,
        ..expected
    };
    assert_eq!(
        missing_run.validate(&frozen, false),
        Err(ProtocolError::InvalidConfig("invalid invocation identity"))
    );
    missing_run.validate(&frozen, true).unwrap();
}
