use super::*;

fn installed() -> Installed {
    Installed {
        registration: Registration {
            id: "reviewed".into(),
            implementation_digest: "digest".into(),
            operations: Vec::from([Operation::BeforeDeliver]),
            scope_ref: "repository:1".into(),
            config_ref: "configuration".into(),
            credential_provider_ref: None,
        },
        stages: Vec::from([Stage::BeforeMerge]),
        plan: Plan {
            entry: "/missing-reviewed-entry".into(),
            entry_sha256: "digest".into(),
            steps: Vec::new(),
        },
    }
}

#[test]
fn registry_resolution_rejects_scope_and_registration_substitution() {
    let entries = [installed()];
    let registration = &entries[0].registration;
    assert!(installed_entry(&entries, registration, "repository:1@1").is_ok());
    assert!(installed_entry(&entries, registration, "repository:2@1").is_err());
    let mut changed = registration.clone();
    changed.config_ref = "different".into();
    assert!(installed_entry(&entries, &changed, "repository:1@1").is_err());
    let mut environment: crate::environment::Plan = serde_json::from_value(json!({
        "controlled":{"protocol_version":1,"environment":{"repository_revision":"repository:1@1",
            "contract_digest":"digest","host_profile_ref":"host","role":"test"},"extensions":[registration]},
        "host_profile_digest":"host","extension_id":"reviewed","lockfiles":[],"roles":{},"ci":false,"cache":null
    })).unwrap();
    assert_eq!(environment_scope(&environment).unwrap(), "repository:1@1");
    environment.extension_id = "missing".into();
    assert!(environment_scope(&environment).is_err());
}

#[test]
fn registry_loading_fails_closed_on_missing_or_malformed_inputs() {
    let environment = serde_json::from_value(json!({
        "controlled":{"protocol_version":1,"environment":{"repository_revision":"repository:1@1",
            "contract_digest":"digest","host_profile_ref":"host","role":"test"},"extensions":[]},
        "host_profile_digest":"host","extension_id":"reviewed","lockfiles":[],"roles":{},"ci":false,"cache":null
    })).unwrap();
    let previous = std::env::var_os("DELIVERY_HOOK_REGISTRY");
    unsafe {
        std::env::remove_var("DELIVERY_HOOK_REGISTRY");
    }
    assert!(
        load_registry(&environment, None)
            .err()
            .unwrap()
            .to_string()
            .contains("registry missing")
    );
    let path = std::env::temp_dir().join(crate::process::new_identity().unwrap());
    unsafe {
        std::env::set_var("DELIVERY_HOOK_REGISTRY", &path);
    }
    assert!(load_registry(&environment, None).is_err());
    std::fs::write(&path, "malformed").unwrap();
    assert!(load_registry(&environment, None).is_err());
    std::fs::write(&path, "[]").unwrap();
    assert!(
        load_registry(&environment, None)
            .err()
            .unwrap()
            .to_string()
            .contains("original validation context")
    );
    assert!(load_registry(&environment, Some(json!({}))).is_err());
    unsafe {
        match previous {
            Some(value) => std::env::set_var("DELIVERY_HOOK_REGISTRY", value),
            None => std::env::remove_var("DELIVERY_HOOK_REGISTRY"),
        }
    }
    std::fs::remove_file(path).unwrap();
}
