use super::*;
use serde_json::json;

fn selection() -> Selection {
    Selection {
        config: ModelConfig {
            provider: "openai".into(),
            model: Some("model-a".into()),
            effort: Some("low".into()),
        },
        reason: "reviewed task complexity".into(),
    }
}
fn registration() -> Registration {
    Registration {
        version: "deployment-1".into(),
        repositories: vec![1],
        agent: AgentCapability {
            name: "codex".into(),
            models: vec![selection().config],
            reliable_stop: true,
            resume: true,
            cancel: true,
            structured_events: true,
            usage_reporting: true,
        },
    }
}
#[test]
fn precedence_identity_scope_and_no_fallback() {
    let default = selection();
    let mut explicit = default.clone();
    explicit.config.model = Some("model-b".into());
    explicit.config.effort = Some("high".into());
    let mut deployed = registration();
    deployed.agent.models.push(explicit.config.clone());
    let frozen = freeze(Some(&explicit), Some(&default), 1, 4, Some(&deployed))
        .unwrap()
        .unwrap();
    assert_eq!(frozen.selection, explicit);
    assert_eq!(frozen.source, "requirement_override");
    assert_eq!(frozen.repository_version, 4);
    let inherited = freeze(None, Some(&default), 1, 4, Some(&deployed))
        .unwrap()
        .unwrap();
    assert_eq!(inherited.source, "project_default");
    assert!(freeze(None, None, 1, 4, None).unwrap().is_none());
    assert!(freeze(Some(&explicit), None, 1, 4, None).is_err());
    assert!(freeze(Some(&explicit), None, 2, 4, Some(&deployed)).is_err());
    deployed.agent.models.clear();
    assert!(freeze(Some(&explicit), None, 1, 4, Some(&deployed)).is_err());
    let response = json!({"model":"model-b","modelProvider":"openai","reasoningEffort":"high"});
    check_response(&frozen, &response).unwrap();
    for field in ["model", "modelProvider", "reasoningEffort"] {
        let mut wrong = response.clone();
        wrong[field] = json!("wrong");
        assert!(check_response(&frozen, &wrong).is_err());
    }
    assert!(check_response(&frozen, &json!({})).is_err());
}
#[test]
fn malformed_choices_and_unsupported_supervision_fail_closed() {
    for field in ["reason", "provider", "model", "effort"] {
        let mut value = serde_json::to_value(selection()).unwrap();
        if field == "reason" {
            value[field] = json!("");
        } else {
            value["config"][field] = json!("");
        }
        let invalid: Selection = serde_json::from_value(value).unwrap();
        assert!(validate(&invalid).is_err());
    }
    let mut invalid = selection();
    invalid.config.model = None;
    assert!(validate(&invalid).is_err());
    invalid = selection();
    invalid.config.effort = None;
    assert!(validate(&invalid).is_err());
    for field in ["version", "name", "reliable_stop", "cancel"] {
        let mut value = serde_json::to_value(registration()).unwrap();
        match field {
            "version" => value[field] = json!(""),
            "name" => value["agent"][field] = json!("other"),
            _ => value["agent"][field] = json!(false),
        }
        let invalid: Registration = serde_json::from_value(value).unwrap();
        assert!(invalid.admit(1, &selection().config).is_err());
    }
}
