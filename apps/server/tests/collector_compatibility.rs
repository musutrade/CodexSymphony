use codexsymphony_server::{
    contract::Repository, controlled_contract::Registration, environment_host::Profile,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

// Reference the pre-change wire contract, including its generated default and
// omission behavior. These tests compare externally observable JSON, not internals.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyProfile {
    registration: Registration,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extensions: Vec<Registration>,
    executable: PathBuf,
    approved_plans: Vec<String>,
    resource_root: PathBuf,
    timeout_seconds: u64,
}
fn profile() -> Value {
    json!({"registration":{"id":"host","implementation_digest":"abc","operations":["environment_check"],"scope_ref":"repo","config_ref":"plan","credential_provider_ref":null},
        "executable":"/host/tool","approved_plans":["plan"],"resource_root":"/host/resources","timeout_seconds":60})
}
fn compare(raw: &str) {
    let old = serde_json::from_str::<LegacyProfile>(raw);
    let new = serde_json::from_str::<Profile>(raw);
    assert_eq!(old.is_ok(), new.is_ok(), "{raw}");
    if let (Ok(old), Ok(new)) = (old, new) {
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
    }
}
#[test]
fn profile_preserves_missing_empty_nonempty_null_duplicate_and_unknown_fields() {
    let original = profile();
    assert!(
        serde_json::from_value::<Profile>(original.clone())
            .unwrap()
            .extensions
            .is_empty()
    );
    compare(&original.to_string());
    for value in [
        json!([]),
        json!([original["registration"].clone()]),
        Value::Null,
        json!(1),
        json!({}),
    ] {
        let mut p = original.clone();
        p["extensions"] = value;
        compare(&p.to_string());
    }
    let raw = original.to_string();
    compare(&raw.replacen('{', "{\"extensions\":[],\"extensions\":[],", 1));
    compare(&raw.replacen('{', "{\"unknown\":1,", 1));
    compare(&raw.replace("\"id\":\"host\"", "\"id\":\"host\",\"id\":\"duplicate\""));
    let sequence = json!([
        original["registration"],
        [],
        original["executable"],
        original["approved_plans"],
        original["resource_root"],
        original["timeout_seconds"]
    ]);
    compare(&sequence.to_string());
    for raw in ["null", "[]", "1", "{}"] {
        compare(raw);
    }
}
fn repository() -> Value {
    json!({"delivery":"local_git","project":"fixture","remote":"local-fixture","base_branch":"main",
        "policy":{"allowed_checks":["cargo_test"],"max_timeout_seconds":60,"token_limit":1000,"turn_limit":2,"model_work_seconds":60,"gate_recovery_policy":"bounded_v1"},
        "revoked":false,"reason":"fixture"})
}
#[test]
fn missing_repository_id_defaults_without_accepting_null_or_duplicates() {
    let input = repository();
    let decoded: Repository = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(decoded.github_repository_id, 0);
    for id in [json!(0), json!(42)] {
        let mut v = input.clone();
        v["github_repository_id"] = id.clone();
        assert_eq!(
            serde_json::from_value::<Repository>(v)
                .unwrap()
                .github_repository_id,
            id.as_i64().unwrap()
        );
    }
    for id in [Value::Null, json!("42"), json!([])] {
        let mut v = input.clone();
        v["github_repository_id"] = id;
        assert!(serde_json::from_value::<Repository>(v).is_err());
    }
    let raw = input.to_string().replacen(
        '{',
        "{\"github_repository_id\":1,\"github_repository_id\":2,",
        1,
    );
    assert!(serde_json::from_str::<Repository>(&raw).is_err());
    let raw = input.to_string().replacen('{', "{\"unknown\":1,", 1);
    assert!(serde_json::from_str::<Repository>(&raw).is_err());
}
