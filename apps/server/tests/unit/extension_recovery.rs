use super::*;

#[test]
fn validation_without_stop_proof_cannot_be_recovered() {
    assert!(stopped(Some(json!({}))).is_err());
    let directory = std::env::temp_dir().join(format!(
        "recovery-stop-{}",
        crate::process::new_identity().unwrap()
    ));
    let context = json!({"call":{"identity":{"protocol_version":1,"requirement_id":1,"revision":1,"run_id":"source","resource_id":"workspace","invocation_id":"validation","attempt":1,"config_id":"frozen"},"controlled_config_digest":"frozen","operation":"validate","extension_id":"validator","implementation_digest":"approved","candidate":null,"environment_digest":"environment","policy_digest":"policy","deadline_unix_ms":1,"required_checks":[]},"environment_contract":"contract","checkout":"/unused","directory":directory});
    assert!(stopped(Some(context.clone())).is_err());
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("identity.json"), "invalid identity").unwrap();
    assert!(stopped(Some(context)).is_err());
}

#[test]
fn invalid_adaptation_is_rejected_before_authorization() {
    let constraint = serde_json::from_value(json!({"id":"scope","version":"1","source":"review","reason":"unsupported","instruction":"named functions","paths":["../outside"],"code_scope":"changed_production","release_condition":"collector proof"})).unwrap();
    let decision = Decision {
        request_id: "bad-scope".into(),
        version: 1,
        revision: 1,
        validation_id: "original".into(),
        reason: "reviewed adaptation".into(),
        action: Action::AdaptCode {
            constraints: vec![constraint],
        },
    };
    assert!(validate(&decision).is_err());
}
