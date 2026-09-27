use super::*;
pub(crate) fn binding() -> Binding {
    Binding {
        identity: InvocationIdentity {
            protocol_version: 2,
            requirement_id: 1,
            revision: 1,
            run_id: Some("source".into()),
            resource_id: "repo-1".into(),
            invocation_id: "call".into(),
            attempt: 1,
            config_id: "configuration".into(),
        },
        phase: "validation".into(),
        candidate: Some(Candidate {
            sha: "commit".into(),
            tree: "tree".into(),
            immutable: true,
        }),
        validation_id: Some("call".into()),
        generation: 1,
        implementation_digest: "implementation".into(),
        environment_digest: "actual-environment".into(),
        policy_digest: "policy".into(),
    }
}
#[test]
fn byte_ranges_reconstruct_export_and_do_not_split_utf8() {
    let text = "first failure\n中文错误\nlast failure".as_bytes();
    let mut captured =
        crate::diagnostic_capture::missing(&binding(), "report", "none", 100).unwrap();
    captured.artifact.availability = Availability::Available;
    captured.artifact.export_bytes = text.len() as u64;
    captured.artifact.export_sha256 = Some(crate::validation::sha256(text));
    let mut offset = 0;
    let mut full = Vec::new();
    loop {
        let page = chunk(captured.artifact.clone(), text, offset, 8).unwrap();
        assert_eq!(page.unit, "bytes");
        assert_eq!(page.offset, offset);
        full.extend_from_slice(page.text.as_bytes());
        offset = page.next;
        if page.end {
            break;
        }
    }
    assert_eq!(full, text);
    for (offset, limit) in [(u64::MAX, 4), (0, 0), (0, MAX_CHUNK + 1), (15, 4), (14, 1)] {
        assert!(
            chunk(captured.artifact.clone(), text, offset, limit).is_err(),
            "{offset}/{limit}"
        );
    }
    assert!(chunk(captured.artifact, text, 0, 4).is_ok());
}
#[test]
fn tampered_export_and_attempt_identity_never_alias() {
    let mut c = crate::diagnostic_capture::missing(&binding(), "log", "none", 100).unwrap();
    c.artifact.availability = Availability::Available;
    c.artifact.export_bytes = 4;
    c.artifact.export_sha256 = Some(crate::validation::sha256(b"real"));
    assert!(chunk(c.artifact, b"fake", 0, 4).is_err());
    let mut newer = binding();
    newer.identity.attempt = 2;
    assert_ne!(
        identifier(&binding(), "log").unwrap(),
        identifier(&newer, "log").unwrap()
    );
    newer.candidate.as_mut().unwrap().sha = "new commit".into();
    assert_ne!(
        identifier(&binding(), "log").unwrap(),
        identifier(&newer, "log").unwrap()
    );
}

#[test]
fn unavailable_states_and_size_mismatch_deny_even_an_empty_export() {
    let mut c = crate::diagnostic_capture::missing(&binding(), "log", "none", 100).unwrap();
    c.artifact.export_sha256 = Some(crate::validation::sha256(b""));
    for status in [
        Availability::Missing,
        Availability::Expired,
        Availability::Corrupt,
    ] {
        c.artifact.availability = status;
        assert!(chunk(c.artifact.clone(), b"", 0, 4).is_err());
    }
    c.artifact.availability = Availability::Available;
    c.artifact.export_bytes = 1;
    assert!(chunk(c.artifact.clone(), b"", 0, 4).is_err());
    c.artifact.export_bytes = 0;
    assert!(chunk(c.artifact, b"", 0, 4).unwrap().end);
}
