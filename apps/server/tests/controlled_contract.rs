use codexsymphony_server::controlled_contract::*;
use codexsymphony_server::extension_contract::*;

fn registration() -> Registration {
    Registration {
        id: "project-python-local".into(),
        implementation_digest: "a".repeat(64),
        operations: vec![
            Operation::EnvironmentCheck,
            Operation::Validate,
            Operation::Submit,
        ],
        scope_ref: "local-repository".into(),
        config_ref: "approved-config".into(),
        credential_provider_ref: None,
    }
}

fn config() -> ControlledConfig {
    ControlledConfig {
        protocol_version: 1,
        environment: EnvironmentBinding {
            repository_revision: "repo-revision-1".into(),
            contract_digest: "b".repeat(64),
            host_profile_ref: "python-no-db-no-cache".into(),
            role: "test".into(),
        },
        extensions: vec![registration()],
    }
}

fn frozen() -> FrozenConfig {
    ExtensionConfig {
        protocol_version: 1,
        agent: "codex".into(),
        model: ModelConfig {
            provider: "codex".into(),
            model: None,
            effort: None,
        },
        delivery: DeliveryMode::GithubPr,
        hooks: vec![],
        decision: None,
    }
    .freeze(&Capabilities::legacy_codex(None))
    .unwrap()
}

fn call() -> Call {
    Call {
        identity: InvocationIdentity {
            protocol_version: 1,
            requirement_id: 1,
            revision: 2,
            run_id: Some("run-1".into()),
            resource_id: "workspace-1".into(),
            invocation_id: "validation-1".into(),
            attempt: 1,
            config_id: frozen().config_id,
        },
        controlled_config_digest: config().freeze(&[registration()]).unwrap(),
        operation: Operation::Validate,
        extension_id: registration().id,
        implementation_digest: registration().implementation_digest,
        candidate: Some(SourceIdentity {
            commit: "commit-1".into(),
            tree: "tree-1".into(),
        }),
        environment_digest: "c".repeat(64),
        policy_digest: "d".repeat(64),
        deadline_unix_ms: 1000,
        required_checks: vec!["project-tests".into()],
    }
}

fn evaluation() -> Evaluation {
    Evaluation {
        call: call(),
        verdict: Verdict::Pass,
        checks: vec![CheckResult {
            id: "project-tests".into(),
            verdict: Verdict::Pass,
            evidence: vec![EvidenceRef {
                artifact_id: "preserved-output-1".into(),
                sha256: "e".repeat(64),
            }],
        }],
    }
}

#[test]
fn vendor_and_language_independent_configuration_roundtrips() {
    let config = config();
    config.validate(&[registration()]).unwrap();
    config
        .require("project-python-local", &Operation::Submit)
        .unwrap();
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(config, serde_json::from_str(&json).unwrap());
    assert!(!json.contains("github"));
    assert!(!json.contains("rust"));
    assert!(!json.contains("token"));
    assert!(config.require("missing", &Operation::Submit).is_err());
    assert!(
        config
            .require("project-python-local", &Operation::Observe)
            .is_err()
    );
}

#[test]
fn unknown_versions_capabilities_and_unreviewed_changes_are_rejected() {
    let mut c = config();
    c.protocol_version = 99;
    assert_eq!(
        c.validate(&[registration()]),
        Err(ProtocolError::UnsupportedVersion(99))
    );
    assert!(serde_json::from_str::<Operation>("\"publish_anything\"").is_err());
    let mut c = config();
    c.extensions[0].scope_ref = "other-repository".into();
    assert!(c.validate(&[registration()]).is_err());
    let mut c = config();
    c.extensions.push(registration());
    assert!(c.validate(&[registration()]).is_err());
    for field in [
        "repository_revision",
        "contract_digest",
        "host_profile_ref",
        "role",
    ] {
        let mut json = serde_json::to_value(config()).unwrap();
        json["environment"][field] = "".into();
        assert!(
            serde_json::from_value::<ControlledConfig>(json)
                .unwrap()
                .validate(&[registration()])
                .is_err()
        );
    }
}

#[test]
fn malformed_approved_registrations_still_fail() {
    for field in [
        "id",
        "implementation_digest",
        "scope_ref",
        "config_ref",
        "credential_provider_ref",
    ] {
        let mut value = serde_json::to_value(registration()).unwrap();
        value[field] = "".into();
        let r: Registration = serde_json::from_value(value).unwrap();
        let mut c = config();
        c.extensions = vec![r.clone()];
        assert!(c.validate(&[r]).is_err());
    }
    let mut r = registration();
    r.operations.clear();
    let mut c = config();
    c.extensions = vec![r.clone()];
    assert!(c.validate(&[r]).is_err());
    let mut r = registration();
    r.credential_provider_ref = Some("deployment-provider".into());
    let mut c = config();
    c.extensions = vec![r.clone()];
    c.validate(&[r]).unwrap();
}

#[test]
fn call_requires_reviewed_identity_source_and_deadline() {
    call()
        .validate(&frozen(), &config(), &[registration()])
        .unwrap();
    for field in [
        "controlled_config_digest",
        "implementation_digest",
        "environment_digest",
        "policy_digest",
        "extension_id",
    ] {
        let mut value = serde_json::to_value(call()).unwrap();
        value[field] = "wrong".into();
        assert!(
            serde_json::from_value::<Call>(value)
                .unwrap()
                .validate(&frozen(), &config(), &[registration()])
                .is_err()
        );
    }
    let mut c = call();
    c.identity.revision = 0;
    assert!(c.validate(&frozen(), &config(), &[registration()]).is_err());
    let mut c = call();
    c.deadline_unix_ms = 0;
    assert!(c.validate(&frozen(), &config(), &[registration()]).is_err());
    let mut c = call();
    c.candidate = None;
    assert!(c.validate(&frozen(), &config(), &[registration()]).is_err());
    c.operation = Operation::EnvironmentCheck;
    c.validate(&frozen(), &config(), &[registration()]).unwrap();
    let mut c = call();
    c.required_checks.push("project-tests".into());
    assert!(c.validate(&frozen(), &config(), &[registration()]).is_err());
}

#[test]
fn results_bind_exact_candidate_environment_policy_attempt_and_operation() {
    let expected = call();
    evaluation().check_pass(&expected, 999).unwrap();
    for field in [
        "controlled_config_digest",
        "implementation_digest",
        "environment_digest",
        "policy_digest",
        "extension_id",
    ] {
        let mut value = serde_json::to_value(evaluation()).unwrap();
        value["call"][field] = "f".repeat(64).into();
        assert!(
            serde_json::from_value::<Evaluation>(value)
                .unwrap()
                .check_pass(&expected, 1)
                .is_err()
        );
    }
    let mut e = evaluation();
    e.call.identity.attempt += 1;
    assert!(e.check_pass(&expected, 1).is_err());
    let mut e = evaluation();
    e.call.candidate.as_mut().unwrap().tree = "other".into();
    assert!(e.check_pass(&expected, 1).is_err());
    let mut e = evaluation();
    e.call.operation = Operation::Submit;
    assert!(e.check_pass(&expected, 1).is_err());
    assert!(evaluation().check_pass(&expected, 1000).is_err());
}

#[test]
fn partial_results_unknowns_duplicates_and_missing_evidence_cannot_pass() {
    let expected = call();
    let mut e = evaluation();
    e.checks.clear();
    assert!(e.check_pass(&expected, 1).is_err());
    let mut e = evaluation();
    e.checks.push(e.checks[0].clone());
    assert!(e.check_pass(&expected, 1).is_err());
    for verdict in [Verdict::Fail, Verdict::Unknown] {
        let mut e = evaluation();
        e.verdict = verdict.clone();
        assert!(e.check_pass(&expected, 1).is_err());
        let mut e = evaluation();
        e.checks[0].verdict = verdict;
        assert!(e.check_pass(&expected, 1).is_err());
    }
    let mut e = evaluation();
    e.checks[0].evidence.clear();
    assert!(e.check_pass(&expected, 1).is_err());
    let mut e = evaluation();
    e.checks[0].evidence[0].sha256 = "not-a-digest".into();
    assert!(e.check_pass(&expected, 1).is_err());
    let mut e = evaluation();
    e.checks[0].evidence[0].artifact_id.clear();
    assert!(e.check_pass(&expected, 1).is_err());
    let mut e = evaluation();
    let mut extra = e.checks[0].clone();
    extra.id = "extra".into();
    extra.verdict = Verdict::Unknown;
    e.checks.push(extra);
    assert!(e.check_pass(&expected, 1).is_err());
}
