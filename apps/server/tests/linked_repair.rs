//! Pure authorization checks using independently executed validation commands.
use codexsymphony_server::{
    linked_repair::{Scope, failed_code},
    validation::{self, ValidationEvidence},
    validation_runner,
};
#[path = "support/validation_runner.rs"]
mod source;
use serde_json::json;
fn failure() -> (std::path::PathBuf, ValidationEvidence) {
    failure_printing("AssertionError: reviewed source invariant\n")
}
/// Executes a trusted failing check that prints `output` through the real runner.
fn failure_printing(output: &str) -> (std::path::PathBuf, ValidationEvidence) {
    let (root, repo, mut plan) = source::fixture();
    std::fs::write(
        &plan.entry,
        format!("#!/bin/sh\ncat <<'GH90_OUTPUT'\n{output}GH90_OUTPUT\nexit 1\n"),
    )
    .unwrap();
    plan.entry_sha256 = validation::sha256(std::fs::read(&plan.entry).unwrap());
    let candidate = validation_runner::candidate(&repo).unwrap();
    let trusted = plan.identity().unwrap();
    let steps = validation_runner::execute(&repo, &root.join("result"), &candidate, &plan).unwrap();
    (
        root,
        ValidationEvidence {
            source_before: candidate.tree.clone(),
            source_after: candidate.tree.clone(),
            entry_before: trusted.protected_entry_sha256.clone(),
            entry_after: trusted.protected_entry_sha256.clone(),
            candidate,
            trusted,
            steps,
        },
    )
}
#[test]
fn exact_paths_and_native_failure_required_no_model_interpretation() {
    let (root, mut evidence) = failure();
    let required = vec!["test".into()];
    let scope = Scope::parse(
        &json!({"schema":"linked-repair/v1","checks":{"test":["source"]}}).to_string(),
    )
    .unwrap();
    assert!(failed_code(&evidence, &required));
    assert_eq!(scope.paths(&evidence, &required).unwrap(), vec!["source"]);
    assert!(Scope::parse("repair anything necessary").is_err());
    assert!(Scope::parse(r#"{"schema":"other","checks":{}}"#).is_err());
    for path in [
        "../secret",
        "/etc/passwd",
        ".git/config",
        "a//b",
        "a\\b",
        "a/./b",
        "a\nb",
    ] {
        assert!(
            Scope::parse(
                &json!({"schema":"linked-repair/v1","checks":{"test":[path]}}).to_string()
            )
            .is_err()
        );
    }
    assert!(
        Scope::parse(&json!({"schema":"linked-repair/v1","checks":{"test":[]}}).to_string())
            .is_err()
    );
    assert!(scope.paths(&evidence, &["missing".into()]).is_err());
    let wrong = Scope::parse(
        &json!({"schema":"linked-repair/v1","checks":{"other":["source"]}}).to_string(),
    )
    .unwrap();
    assert!(wrong.paths(&evidence, &required).is_err());
    evidence.steps[0].output_sha256 = "wrong".into();
    assert!(scope.paths(&evidence, &required).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn unknown_security_infrastructure_and_no_failure_cannot_start_code() {
    let (root, original) = failure();
    let required = vec!["test".into()];
    let scope = Scope::parse(
        &json!({"schema":"linked-repair/v1","checks":{"test":["source"]}}).to_string(),
    )
    .unwrap();
    for text in [
        "unknown",
        "permission denied; AssertionError",
        "connection refused; AssertionError",
        "HTTP 429; AssertionError",
        "FAIL independent security contract: policy changed",
    ] {
        let mut evidence = original.clone();
        evidence.steps[0].output = text.into();
        evidence.steps[0].output_sha256 = validation::sha256(text);
        assert!(!failed_code(&evidence, &required));
        assert!(scope.paths(&evidence, &required).is_err());
    }
    for exit in [None, Some(0)] {
        let mut evidence = original.clone();
        evidence.steps[0].exit_code = exit;
        assert!(!failed_code(&evidence, &required));
        assert!(scope.paths(&evidence, &required).is_err());
    }
    let mut evidence = original;
    evidence.steps[0].code_failure = false;
    assert!(!failed_code(&evidence, &required));
    assert!(scope.paths(&evidence, &required).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn independent_contract_failure_is_classified_only_with_trusted_code_flag() {
    let (root, mut evidence) = failure();
    let required = vec!["test".into()];
    for output in [
        "cargo tests passed\nFAIL independent GH-88 acceptance contract: gh88-b05.json must be ready\n",
        "cargo tests passed\nFAIL independent GH-88 integration contract: both repositories must be ready\n",
    ] {
        evidence.steps[0].output = output.into();
        evidence.steps[0].output_sha256 = validation::sha256(output);
        assert!(failed_code(&evidence, &required));
        evidence.steps[0].code_failure = false;
        assert!(!failed_code(&evidence, &required));
        evidence.steps[0].code_failure = true;
    }
    std::fs::remove_dir_all(root).unwrap();
}

// Recorded GH-90 R3 integration output: the Node unit phase passes, then the
// trusted mixed check calls a method on a module that exports a bare function.
const R3_NODE_OUTPUT: &str = "\u{2714} wireVersion returns version 1 (1.138262ms)\n\u{2139} tests 1\n\u{2139} pass 1\n\u{2139} fail 0\n[eval]:12\n assert.equal(local.wireVersion(),2,'AssertionError: mixed contract requires Node wire version 2');\n                    ^\n\nTypeError: local.wireVersion is not a function\n    at [eval]:12:21\n    at runScriptInThisContext (node:internal/vm:219:10)\n\nNode.js v24.18.0\n";

#[test]
fn native_node_not_a_function_failure_authorizes_only_reviewed_paths() {
    let (root, evidence) = failure_printing(R3_NODE_OUTPUT);
    let required = vec!["test".into()];
    assert_eq!(evidence.steps[0].output, R3_NODE_OUTPUT);
    assert_eq!(evidence.steps[0].exit_code, Some(1));
    let scope = Scope::parse(
        &json!({"schema":"linked-repair/v1","checks":{"test":["index.js","test/index.test.js"]}})
            .to_string(),
    )
    .unwrap();
    assert!(failed_code(&evidence, &required));
    assert_eq!(
        scope.paths(&evidence, &required).unwrap(),
        vec!["index.js", "test/index.test.js"]
    );

    let mut untrusted = evidence.clone();
    untrusted.steps[0].code_failure = false;
    assert!(!failed_code(&untrusted, &required));
    assert!(scope.paths(&untrusted, &required).is_err());

    let mut tampered = evidence.clone();
    tampered.steps[0].output_sha256 = validation::sha256("other output");
    assert!(!failed_code(&tampered, &required));
    assert!(scope.paths(&tampered, &required).is_err());

    let mut rewritten = evidence.clone();
    rewritten.steps[0].output = R3_NODE_OUTPUT.replace("wireVersion is", "version is");
    assert!(!failed_code(&rewritten, &required));
    assert!(scope.paths(&rewritten, &required).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn type_error_cannot_override_infrastructure_or_match_loosely() {
    let required = vec!["test".into()];
    let scope = Scope::parse(
        &json!({"schema":"linked-repair/v1","checks":{"test":["index.js"]}}).to_string(),
    )
    .unwrap();
    for output in [
        "permission denied\nTypeError: local.wireVersion is not a function\n",
        "connection refused\nTypeError: local.wireVersion is not a function\n",
        "HTTP 429\nTypeError: local.wireVersion is not a function\n",
        "TypeError: Cannot read properties of undefined (reading 'wireVersion')\n",
        "    at TypeError: local.wireVersion is not a function\n",
        " assert.equal(x,2,'TypeError: local.wireVersion is not a function');\n",
    ] {
        let (root, evidence) = failure_printing(output);
        assert_eq!(evidence.steps[0].output, output);
        assert!(!failed_code(&evidence, &required), "{output:?}");
        assert!(scope.paths(&evidence, &required).is_err(), "{output:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn structured_failures_require_complete_evidence_and_all_failed_scopes() {
    let (root, legacy) = failure();
    let scope = Scope::parse(
        r#"{"schema":"linked-repair/v1","checks":{"test":["source"],"feedback":["attachment-source"]}}"#,
    ).unwrap();
    let mut structured = legacy.steps[0].clone();
    structured.id = "feedback".into();
    structured.command = vec!["/gate-entry".into(), "--symphony-feedback-v2".into()];
    structured.exit_code = Some(0);
    structured.output = json!({"protocol_version":2,"check_id":"feedback","verdict":"fail","fault":null,"artifacts":[]}).to_string();
    structured.output_sha256 = validation::sha256(&structured.output);
    let mut mixed = legacy.clone();
    mixed.steps.push(structured.clone());
    let required = vec!["test".into(), "feedback".into()];
    let original = mixed.clone();
    assert!(failed_code(&mixed, &required));
    assert_eq!(
        scope.paths(&mixed, &required).unwrap(),
        vec!["attachment-source", "source"]
    );
    assert_eq!(mixed, original);
    let mut structured_only = mixed.clone();
    structured_only.steps.remove(0);
    assert!(failed_code(&structured_only, &["feedback".into()]));
    assert_eq!(
        scope.paths(&structured_only, &["feedback".into()]).unwrap(),
        vec!["attachment-source"]
    );
    let unmapped =
        Scope::parse(r#"{"schema":"linked-repair/v1","checks":{"test":["source"]}}"#).unwrap();
    assert!(unmapped.paths(&mixed, &required).is_err());
    for output in [
        "{malformed".to_owned(),
        json!({"protocol_version":2,"check_id":"other","verdict":"fail","fault":null,"artifacts":[]}).to_string(),
        json!({"protocol_version":2,"check_id":"feedback","verdict":"unknown","fault":{"class":"resource","code":"timeout","message":"unavailable","owner":"host","scope":[],"resume_condition":"service restored"},"artifacts":[]}).to_string(),
    ] {
        let mut invalid=mixed.clone();invalid.steps[1].output=output;invalid.steps[1].output_sha256=validation::sha256(&invalid.steps[1].output);
        assert!(!failed_code(&invalid,&required));assert!(scope.paths(&invalid,&required).is_err());
    }
    let mut invalid = mixed.clone();
    invalid.steps[1].output_sha256 = "tampered".into();
    assert!(!failed_code(&invalid, &required));
    assert!(scope.paths(&invalid, &required).is_err());
    let mut invalid = mixed.clone();
    invalid.steps[1].code_failure = false;
    assert!(!failed_code(&invalid, &required));
    assert!(scope.paths(&invalid, &required).is_err());
    let mut invalid = mixed.clone();
    invalid.steps[1].command[1] = "--symphony-feedback-v3".into();
    assert!(!failed_code(&invalid, &required));
    assert!(scope.paths(&invalid, &required).is_err());
    let mut invalid = mixed.clone();
    invalid.steps[1].exit_code = Some(1);
    assert!(!failed_code(&invalid, &required));
    assert!(scope.paths(&invalid, &required).is_err());
    let mut invalid = mixed.clone();
    invalid.steps[1].output=json!({"protocol_version":2,"check_id":"feedback","verdict":"pass","fault":null,"artifacts":[]}).to_string();
    invalid.steps[1].output_sha256 = validation::sha256(&invalid.steps[1].output);
    assert!(failed_code(&invalid, &required));
    assert_eq!(scope.paths(&invalid, &required).unwrap(), vec!["source"]);
    assert!(!failed_code(&mixed, &["missing".into()]));
    std::fs::remove_dir_all(root).unwrap();
}
