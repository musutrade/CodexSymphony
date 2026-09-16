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

#[test]
fn rejects_every_identity_and_evidence_failure() {
    let required = vec!["test".into()];
    let (original, c, t) = fixture();
    let mut cases = Vec::new();
    let mut e = original.clone();
    e.candidate.immutable = false;
    cases.push((e, ValidationError::CandidateMutable));
    let mut e = original.clone();
    e.candidate.sha = "other".into();
    cases.push((e, ValidationError::CandidateMismatch));
    let mut e = original.clone();
    e.trusted.command_sha256 = "other".into();
    cases.push((e, ValidationError::TrustedIdentityMismatch));
    let mut e = original.clone();
    e.source_after = "other".into();
    cases.push((e, ValidationError::SourceChanged));
    let mut e = original.clone();
    e.entry_after = "other".into();
    cases.push((e, ValidationError::ProtectedEntryChanged));
    let mut e = original.clone();
    e.steps.clear();
    cases.push((e, ValidationError::MissingStep));
    let mut e = original.clone();
    e.steps.push(e.steps[0].clone());
    cases.push((e, ValidationError::MissingStep));
    for which in 0..4 {
        let mut e = original.clone();
        match which {
            0 => e.steps[0].output.clear(),
            1 => e.steps[0].log_ref.clear(),
            2 => e.steps[0].consumer.clear(),
            _ => e.steps[0].command.clear(),
        }
        cases.push((e, ValidationError::MissingOutput));
    }
    for code in [None, Some(1)] {
        let mut e = original.clone();
        e.steps[0].exit_code = code;
        cases.push((e, ValidationError::ExitFailed));
    }
    for (e, error) in cases {
        assert_eq!(verify(&e, &c, &t, &required), Err(error));
    }
    assert_eq!(
        verify(&original, &c, &t, &[]),
        Err(ValidationError::MissingStep)
    );
    let mut mutable = c.clone();
    mutable.immutable = false;
    assert_eq!(
        verify(&original, &mutable, &t, &required),
        Err(ValidationError::CandidateMutable)
    );
}

#[test]
fn recovery_context_and_repair_boundaries() {
    assert_eq!(recover(Stage::Declaration, true, true), Stage::Validation);
    assert_eq!(
        recover(Stage::Validation, false, true),
        Stage::RepairReservation
    );
    assert_eq!(recover(Stage::Handoff, false, false), Stage::Handoff);
    let (e, _, _) = fixture();
    assert!(repair_context(&e, "absent", &[]).is_none());
    let c = repair_context(&e, "test", &["remaining".into()]).unwrap();
    assert_eq!(c.raw_output, "PASS");
    assert_eq!(c.remaining_acceptance, ["remaining"]);
    assert_eq!(
        classify_step_failure(&e.steps[0]),
        FailureKind::Infrastructure
    );
    let mut step = e.steps[0].clone();
    step.code_failure = true;
    assert_eq!(classify_step_failure(&step), FailureKind::Code);
    let mut ledger = RepairLedger::default();
    assert_eq!(ledger.reserve(FailureKind::Security), None);
    ledger.count = 1;
    assert_eq!(ledger.reserve(FailureKind::Code), None);
}
