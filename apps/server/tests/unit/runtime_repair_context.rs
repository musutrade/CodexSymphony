use super::*;
use crate::validation::{Candidate, TrustedIdentity, sha256};

fn evidence() -> ValidationEvidence {
    let output = format!("FIRST\n{}LAST\n", "原报告故障\n".repeat(6000));
    ValidationEvidence {
        candidate: Candidate {
            sha: "candidate".into(),
            tree: "tree".into(),
            immutable: true,
        },
        trusted: TrustedIdentity {
            command_sha256: "command".into(),
            config_sha256: "config".into(),
            protected_entry: "entry".into(),
            protected_entry_sha256: "entry-sha".into(),
            tool: "tool".into(),
            tool_version: "1".into(),
        },
        source_before: "source".into(),
        source_after: "source".into(),
        entry_before: "entry".into(),
        entry_after: "entry".into(),
        steps: vec![StepEvidence {
            id: "shell".into(),
            command: vec!["/gate-entry".into()],
            exit_code: Some(2),
            output_sha256: sha256(&output),
            output,
            log_ref: "logs/shell.txt".into(),
            consumer: "collector".into(),
            code_failure: true,
        }],
    }
}

#[test]
fn long_evidence_is_metadata_only_without_changing_original_failure() {
    let original = evidence();
    let repair = json!({"original_failure":"same-failure","evidence":original,"authorized_paths":["index.js"],"baseline":"original","instruction":"reviewed scope"});
    let before = repair.clone();
    let prompt = project(Some(repair.clone())).unwrap().unwrap();
    assert_eq!(repair, before);
    assert_eq!(prompt["original_failure"], before["original_failure"]);
    assert_eq!(prompt["authorized_paths"], before["authorized_paths"]);
    assert_eq!(prompt["baseline"], before["baseline"]);
    assert_eq!(prompt["instruction"], before["instruction"]);
    assert!(prompt["evidence"].is_null());
    assert!(
        serde_json::from_value::<ValidationEvidence>(prompt["evidence_metadata"].clone()).is_err()
    );
    let metadata = &prompt["evidence_metadata"];
    for field in [
        "candidate",
        "trusted",
        "source_before",
        "source_after",
        "entry_before",
        "entry_after",
    ] {
        assert_eq!(metadata[field], before["evidence"][field]);
    }
    let step = &metadata["steps"][0];
    for field in [
        "id",
        "command",
        "exit_code",
        "output_sha256",
        "log_ref",
        "consumer",
        "code_failure",
    ] {
        assert_eq!(step[field], before["evidence"]["steps"][0][field]);
    }
    assert_eq!(step["output_bytes"], original.steps[0].output.len());
    assert_eq!(
        step["verdict"],
        json!(crate::controlled_contract::Verdict::Fail)
    );
    assert!(step.get("output").is_none());
    assert!(!prompt.to_string().contains("原报告故障"));
    assert!(prompt.to_string().len() < 3000);
    assert_eq!(prompt["diagnostic_reading"]["output_in_prompt"], false);
}

#[test]
fn structured_business_fail_is_retained_as_fail_with_zero_exit() {
    let mut original = evidence();
    let step = &mut original.steps[0];
    step.id = "python".into();
    step.command.push("--symphony-feedback-v2".into());
    step.exit_code = Some(0);
    step.output = json!({"protocol_version":2,"check_id":"python","verdict":"fail","fault":null,"artifacts":[]}).to_string();
    step.output_sha256 = sha256(&step.output);
    let prompt = project(Some(json!({"evidence":original})))
        .unwrap()
        .unwrap();
    let step = &prompt["evidence_metadata"]["steps"][0];
    assert_eq!(step["exit_code"], 0);
    assert_eq!(step["verdict"], "fail");
    assert!(step.get("output").is_none());
}

#[test]
fn absence_and_nonvalidation_recovery_contexts_remain_unchanged() {
    assert_eq!(project(None).unwrap(), None);
    for repair in [
        json!({}),
        json!({"evidence":{"kind":"pre_merge_recovery","original_candidate":"original","operator_decision":{"paths":["reviewed"]}}}),
        json!({"failure":{"raw_output":"legacy failure"},"resolution":"approved"}),
    ] {
        assert_eq!(project(Some(repair.clone())).unwrap(), Some(repair));
    }
}

#[test]
fn malformed_validation_evidence_cannot_be_silently_inlined() {
    let error = project(Some(json!({"evidence":{"steps":[]}}))).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("linked repair evidence unavailable")
    );
}
