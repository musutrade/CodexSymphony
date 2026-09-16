use codexsymphony_server::validation::*;
fn fixture() -> (ValidationEvidence, Candidate, TrustedIdentity) {
    let c = Candidate {
        sha: "a".into(),
        tree: "b".into(),
        immutable: true,
    };
    let t = TrustedIdentity {
        command_sha256: "c".into(),
        config_sha256: "d".into(),
        protected_entry: "gate".into(),
        protected_entry_sha256: "e".into(),
        tool: "gate".into(),
        tool_version: "1".into(),
    };
    let output: String = "PASS".into();
    let s = StepEvidence {
        id: "test".into(),
        command: vec!["test".into()],
        exit_code: Some(0),
        output: output.clone(),
        output_sha256: sha256(&output),
        log_ref: "log/1".into(),
        consumer: "handoff".into(),
        code_failure: false,
    };
    (
        ValidationEvidence {
            candidate: c.clone(),
            trusted: t.clone(),
            source_before: "s".into(),
            source_after: "s".into(),
            entry_before: "e".into(),
            entry_after: "e".into(),
            steps: vec![s],
        },
        c,
        t,
    )
}
#[test]
fn rejects_tampered_output() {
    let (mut e, c, t) = fixture();
    e.steps[0].output.push('x');
    assert_eq!(
        verify(&e, &c, &t, &["test".into()]),
        Err(ValidationError::OutputDigestMismatch)
    );
}
#[test]
fn verifies_fixed_candidate() {
    let (e, c, t) = fixture();
    assert!(verify(&e, &c, &t, &["test".into()]).is_ok());
}
#[test]
fn repair_is_once_and_infra_is_free() {
    let mut l = RepairLedger::new();
    assert_eq!(l.reserve(FailureKind::Infrastructure), None);
    assert_eq!(l.reserve(FailureKind::Code), Some(1));
    assert_eq!(l.reserve(FailureKind::Code), Some(1));
}
