use super::*;
use crate::diagnostics::tests::binding;
use serde_json::json;

#[test]
fn registered_reports_respect_the_shared_manifest_limit_and_keep_omission_visible() {
    let root = std::env::temp_dir().join(crate::process::new_identity().unwrap());
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("report.md"), "failure\n").unwrap();
    let limits = diagnostic_store::Limits {
        file: 1024,
        call: 4096,
        task: 8192,
        count: 64,
        expires_at: 100,
    };
    let artifact = crate::extension_contract::ArtifactRef {
        path: "report.md".into(),
        kind: "report".into(),
    };
    let mut captures = Vec::new();
    assert!(
        append(
            &root,
            &binding(),
            &vec![artifact.clone(); 65],
            &limits,
            &mut captures
        )
        .is_err()
    );
    assert!(captures.is_empty());
    for n in 0..62 {
        captures.push(
            crate::diagnostic_capture::missing(
                &binding(),
                &format!("absent-{n}"),
                "not generated",
                100,
            )
            .unwrap(),
        );
    }
    append(
        &root,
        &binding(),
        &[artifact.clone(), artifact],
        &limits,
        &mut captures,
    )
    .unwrap();
    assert_eq!(captures.len(), 64);
    assert_eq!(
        captures[62].artifact.availability,
        crate::diagnostics::Availability::Available
    );
    assert_eq!(captures[63].artifact.purpose, "diagnostic-manifest-limit");
    omitted(&binding(), &limits, &mut captures).unwrap();
    assert_eq!(captures.len(), 64);
}

#[test]
fn invalid_or_unselected_envelopes_never_register_arbitrary_reports() {
    let root = std::env::temp_dir().join(crate::process::new_identity().unwrap());
    std::fs::create_dir(&root).unwrap();
    let limits = diagnostic_store::Limits {
        file: 1024,
        call: 4096,
        task: 8192,
        count: 64,
        expires_at: 100,
    };
    let mut step = crate::validation_runner::Step {
        id: "check".into(),
        command: vec![
            "/gate-entry".into(),
            crate::extension_feedback::SELECTOR.into(),
        ],
        timeout_seconds: 1,
        code_failure: true,
    };
    let valid = json!({"protocol_version":2,"check_id":"check","verdict":"fail","fault":null,"artifacts":[{"kind":"report","path":"report.md"}]});
    let mut captures = Vec::new();
    for (field, value) in [
        ("protocol_version", json!(1)),
        ("check_id", json!("other")),
        ("artifacts", json!([{"kind":"report","path":"../outside"}])),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        feedback_artifacts(
            &root,
            &binding(),
            &step,
            &serde_json::to_vec(&invalid).unwrap(),
            &limits,
            &mut captures,
        )
        .unwrap();
        assert!(captures.is_empty());
    }
    feedback_artifacts(
        &root,
        &binding(),
        &step,
        b"{malformed",
        &limits,
        &mut captures,
    )
    .unwrap();
    assert!(captures.is_empty());
    step.command.pop();
    feedback_artifacts(
        &root,
        &binding(),
        &step,
        &serde_json::to_vec(&valid).unwrap(),
        &limits,
        &mut captures,
    )
    .unwrap();
    assert!(captures.is_empty());
    lifecycle_artifacts(
        &root,
        &binding().identity,
        b"{malformed",
        &binding(),
        &limits,
        &mut captures,
    )
    .unwrap();
    assert!(captures.is_empty());
}
