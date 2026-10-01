use super::*;
use crate::group_review::Item;
use serde_json::{Value, json};
use std::sync::{Mutex, MutexGuard};

// RUNTIME_CONFIG is process-global; serialize the tests that install it.
static ENV: Mutex<()> = Mutex::new(());

/// Restores the previous RUNTIME_CONFIG and removes its own file on drop.
struct Deployed {
    previous: Option<std::ffi::OsString>,
    path: std::path::PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for Deployed {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(previous) => std::env::set_var("RUNTIME_CONFIG", previous),
                None => std::env::remove_var("RUNTIME_CONFIG"),
            }
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn deployed(name: &str, version: &str, models: Value) -> Deployed {
    let lock = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var_os("RUNTIME_CONFIG");
    let registration = json!({"version":version,"repositories":[1],"agent":{"name":"codex","models":models,"reliable_stop":true,"resume":true,"cancel":true,"structured_events":true,"usage_reporting":true}});
    let config = json!({"settings":{"model_capabilities":registration,"startup_seconds":5,"response_seconds":5,"stall_seconds":5,"reservation":{"tokens":100,"turns":1,"model_seconds":30},"codex_config":""},"preparation_adapter":"/bin/true","preparation":{"launcher":["/bin/true"]}});
    let path =
        std::env::temp_dir().join(format!("model-review-{name}-{}.json", std::process::id()));
    std::fs::write(&path, config.to_string()).unwrap();
    unsafe {
        std::env::set_var("RUNTIME_CONFIG", &path);
    }
    Deployed {
        previous,
        path,
        _lock: lock,
    }
}

fn old() -> Value {
    json!({"provider":"openai","model":"old-model","effort":"low"})
}
fn new() -> Value {
    json!({"provider":"openai","model":"new-model","effort":"medium"})
}
fn selection(config: Value, reason: &str) -> Value {
    json!({"config":config,"reason":reason})
}
fn frozen(config: Value, version: &str) -> Value {
    json!({"selection":selection(config,"historical approval"),"source":"requirement_override","repository_id":1,"repository_version":1,"capability_version":version})
}

fn document() -> Document {
    let children: Vec<Value> = ["C1", "C2", "C3"].iter().enumerate().map(|(i, id)| json!({"id":id,"parent_id":"P1","kind":"code_change","order":i+1,"depends_on":[],"repository_id":1,"goal":"goal","acceptance_criteria":[{"id":"AC1","description":"observable"}],"validation_plan":"run test"})).collect();
    serde_json::from_value(json!({"schema":"codexsymphony-draft/v1","parent":{"id":"P1","goal":"g","scope":"s","acceptance_criteria":[{"id":"P-AC1","description":"d"}]},"children":children})).unwrap()
}

fn item(child: &str, selected: Option<Value>, frozen_model: Option<Value>) -> Value {
    let mut value = json!({"child_id":child,"revision":1,"repository_version":1,"integration":null,"budget":{"tokens":100,"turns":1,"model_seconds":30},"repair_scope":"scope","merged_baseline_review":"baseline","verification":[]});
    if let Some(selected) = selected {
        value["model_selection"] = selected;
    }
    if let Some(frozen_model) = frozen_model {
        value["frozen_model"] = frozen_model;
    }
    value
}

/// C1 is a completed item frozen under the retired registration; C3 is the
/// unstarted item whose reviewed model changes.
fn review() -> Review {
    let items = vec![
        item(
            "C1",
            Some(selection(old(), "historical approval")),
            Some(frozen(old(), "v1")),
        ),
        item("C2", None, None),
        item(
            "C3",
            Some(selection(new(), "reviewed new model")),
            Some(frozen(old(), "v1")),
        ),
    ];
    serde_json::from_value(json!({"parent_revision":1,"full_chain_acs":[],"coverage":[],"items":items,"group_budget":null,"semantic_review":"review"})).unwrap()
}

fn repositories(default: Option<Value>) -> Vec<RepositorySnapshot> {
    let mut repository = json!({"model":"fixture-model","project":"p","remote":"test/group","github_repository_id":123,"base_branch":"main","policy":{"allowed_checks":["cargo_test"],"max_timeout_seconds":60,"token_limit":1000,"turn_limit":20,"model_work_seconds":600,"gate_recovery_policy":"one_code_repair"},"revoked":false,"reason":"r"});
    if let Some(default) = default {
        repository["model_selection"] = default;
    }
    vec![RepositorySnapshot {
        id: 1,
        version: 1,
        repository: serde_json::from_value(repository).unwrap(),
    }]
}

fn find<'a>(review: &'a Review, child: &str) -> &'a Item {
    review.items.iter().find(|i| i.child_id == child).unwrap()
}
fn frozen_json(review: &Review, child: &str) -> Value {
    json!(find(review, child).frozen_model)
}
fn affected(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

#[test]
fn unchanged_items_keep_historical_models_absent_from_current_registration() {
    let _env = deployed("retained", "v2", json!([new()]));
    let repositories = repositories(None);
    let mut full = review();
    // The whole-group freeze is what rejected the GH-90 R3 queue edit.
    assert_eq!(
        group(&document(), &mut full, &repositories),
        Err("provider/model/effort is not deployed; fallback is forbidden")
    );
    let mut reviewed = review();
    group_selected(
        &document(),
        &mut reviewed,
        &repositories,
        &affected(&["C3"]),
    )
    .unwrap();
    assert_eq!(frozen_json(&reviewed, "C1"), frozen(old(), "v1"));
    let refreshed = find(&reviewed, "C3").frozen_model.as_ref().unwrap();
    assert_eq!(json!(refreshed.selection.config), new());
    assert_eq!(refreshed.capability_version, "v2");
    assert_eq!(refreshed.source, "requirement_override");
    assert_eq!(refreshed.repository_id, 1);
    assert!(find(&reviewed, "C2").frozen_model.is_none());
    verify_group_selected(&document(), &reviewed, &repositories, &affected(&["C3"])).unwrap();
}

#[test]
fn affected_items_cannot_keep_a_retired_model_or_stale_identity() {
    let _env = deployed("affected", "v2", json!([new()]));
    let repositories = repositories(None);
    // Re-reviewing the historical item itself demands current authority.
    let mut reviewed = review();
    assert!(
        group_selected(
            &document(),
            &mut reviewed,
            &repositories,
            &affected(&["C1"])
        )
        .is_err()
    );
    // A submitted stale identity for a changed item is never trusted.
    let stale = review();
    assert_eq!(
        verify_group_selected(&document(), &stale, &repositories, &affected(&["C3"])),
        Err("model choice changed after review; save and review again")
    );
}

#[test]
fn approval_rejects_registration_drift_for_affected_items_only() {
    let repositories = repositories(None);
    let mut reviewed = review();
    {
        let _env = deployed("drift-before", "v2", json!([new()]));
        group_selected(
            &document(),
            &mut reviewed,
            &repositories,
            &affected(&["C3"]),
        )
        .unwrap();
    }
    {
        let _env = deployed("drift-version", "v3", json!([new()]));
        assert_eq!(
            verify_group_selected(&document(), &reviewed, &repositories, &affected(&["C3"])),
            Err("model choice changed after review; save and review again")
        );
    }
    {
        // Removing the reviewed model must fail closed, not fall back.
        let _env = deployed("drift-model", "v2", json!([old()]));
        assert_eq!(
            verify_group_selected(&document(), &reviewed, &repositories, &affected(&["C3"])),
            Err("provider/model/effort is not deployed; fallback is forbidden")
        );
    }
    {
        let _env = deployed("drift-after", "v2", json!([new()]));
        verify_group_selected(&document(), &reviewed, &repositories, &affected(&["C3"])).unwrap();
    }
    // Nothing affected: historical C1 needs no current registration at approval.
    let _env = deployed("drift-none", "v9", json!([]));
    verify_group_selected(&document(), &reviewed, &repositories, &[]).unwrap();
}

#[test]
fn affected_items_without_selection_follow_default_or_clear_frozen_model() {
    let mut reviewed = review();
    // No selection and no repository default: a changed item drops any stale freeze.
    reviewed.items[1].frozen_model = Some(serde_json::from_value(frozen(old(), "v1")).unwrap());
    group_selected(
        &document(),
        &mut reviewed,
        &repositories(None),
        &affected(&["C2"]),
    )
    .unwrap();
    assert!(find(&reviewed, "C2").frozen_model.is_none());
    assert_eq!(frozen_json(&reviewed, "C1"), frozen(old(), "v1"));

    let _env = deployed("default", "v2", json!([new()]));
    let with_default = repositories(Some(selection(new(), "project default")));
    let mut defaulted = review();
    group_selected(
        &document(),
        &mut defaulted,
        &with_default,
        &affected(&["C2"]),
    )
    .unwrap();
    let applied = find(&defaulted, "C2").frozen_model.as_ref().unwrap();
    assert_eq!(applied.source, "project_default");
    assert_eq!(applied.capability_version, "v2");
    assert_eq!(
        json!(applied.selection),
        selection(new(), "project default")
    );
    assert_eq!(frozen_json(&defaulted, "C1"), frozen(old(), "v1"));
    assert_eq!(frozen_json(&defaulted, "C3"), frozen(old(), "v1"));
    // A repository default never reaches unchanged items.
    let mut untouched = review();
    group_selected(
        &document(),
        &mut untouched,
        &with_default,
        &affected(&["C3"]),
    )
    .unwrap();
    assert!(find(&untouched, "C2").frozen_model.is_none());
    assert_eq!(frozen_json(&untouched, "C1"), frozen(old(), "v1"));
}

#[test]
fn affected_item_must_belong_to_a_reviewed_child_and_repository() {
    let _env = deployed("unknown", "v2", json!([new()]));
    let mut reviewed = review();
    reviewed.items[2].child_id = "missing".into();
    assert_eq!(
        group_selected(
            &document(),
            &mut reviewed,
            &repositories(None),
            &affected(&["missing"])
        ),
        Err("model selection references an unknown child or repository")
    );
}
