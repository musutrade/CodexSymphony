use super::*;
#[test]
fn protocol_timestamp_rejects_pre_epoch_and_overflow() {
    use std::time::{Duration, UNIX_EPOCH};
    assert_eq!(
        unix_ms(UNIX_EPOCH + Duration::from_millis(123)).unwrap(),
        123
    );
    assert!(unix_ms(UNIX_EPOCH - Duration::from_millis(1)).is_err());
    assert!(unix_ms(UNIX_EPOCH + Duration::from_secs(i64::MAX as u64 / 1000 + 1)).is_err());
}

mod drain {
    use super::super::*;
    use crate::{
        controlled_contract::{Operation, Registration},
        environment_host::Profile,
        execution::{ProcessIdentity, RunKey},
    };
    use std::collections::BTreeMap;

    // These tests never call `begin_drain`: the gate is process-global and
    // this binary is shared. `tests/environment_drain.rs` covers the gate.
    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "environment-drain-{}",
                process::new_identity().unwrap()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn profile(timeout_seconds: u64) -> Profile {
        Profile {
            registration: Registration {
                id: "fixture".into(),
                implementation_digest: "a".repeat(64),
                operations: vec![Operation::EnvironmentCheck],
                scope_ref: "all".into(),
                config_ref: "fixture-config".into(),
                credential_provider_ref: None,
            },
            extensions: vec![],
            executable: "/nonexistent/probe".into(),
            approved_plans: vec![],
            resource_root: "/nonexistent".into(),
            timeout_seconds,
        }
    }

    fn registry(timeouts: &[u64]) -> Registry {
        Registry {
            evidence_root: "/nonexistent/evidence".into(),
            profiles: timeouts
                .iter()
                .enumerate()
                .map(|(index, timeout)| (format!("profile-{index}"), profile(*timeout)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn receipt(pid: u32) -> Receipt {
        Receipt {
            key: RunKey {
                run_id: "run".into(),
                request_id: "request".into(),
                incarnation: "incarnation".into(),
            },
            process: ProcessIdentity {
                pid,
                group: pid,
                start_ticks: 1,
                boot_id: "boot".into(),
            },
        }
    }

    /// A launched probe directory with its supervisor identity and an
    /// optional quiescent receipt.
    fn launched(root: &Path, name: &str, quiescent: Option<Receipt>) -> PathBuf {
        let directory = root.join(name);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("input.json"), "{}").unwrap();
        process::durable_write(&directory.join("identity.json"), &receipt(1)).unwrap();
        std::fs::write(directory.join("launched"), "").unwrap();
        if let Some(stopped) = quiescent {
            process::durable_write(&directory.join("quiescent.json"), &stopped).unwrap();
        }
        directory
    }

    #[test]
    fn limit_is_longest_registered_deadline_plus_stop_window() {
        assert_eq!(drain_limit(&registry(&[15])), Duration::from_secs(35));
        assert_eq!(drain_limit(&registry(&[7, 15, 3])), Duration::from_secs(35));
        assert_eq!(drain_limit(&registry(&[600, 1])), Duration::from_secs(620));
        assert_eq!(drain_limit(&registry(&[])), STOP_WINDOW);
    }

    #[tokio::test]
    async fn settles_only_when_every_launched_probe_is_quiescent() {
        let root = Temporary::new();
        let limit = Duration::from_secs(1);
        settled(&root.0.join("missing"), limit).await.unwrap();
        settled(&root.0, limit).await.unwrap();
        launched(&root.0, "stopped", Some(receipt(1)));
        std::fs::create_dir(root.0.join("unrelated")).unwrap();
        settled(&root.0, limit).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_or_mismatched_outcome_is_never_drained() {
        for quiescent in [None, Some(receipt(2))] {
            let root = Temporary::new();
            launched(&root.0, "stopped", Some(receipt(1)));
            let unknown = launched(&root.0, "unknown", quiescent);
            let before: Vec<_> = ["input.json", "identity.json", "launched"]
                .iter()
                .map(|name| std::fs::read(unknown.join(name)).unwrap())
                .collect();
            let error = settled(&root.0, Duration::from_secs(1))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("outcome unknown"), "{error}");
            // Evidence is preserved exactly; nothing is written on failure.
            let after: Vec<_> = ["input.json", "identity.json", "launched"]
                .iter()
                .map(|name| std::fs::read(unknown.join(name)).unwrap())
                .collect();
            assert_eq!(before, after);
            assert!(!unknown.join("host-recovery-proof.json").exists());
        }
    }

    #[tokio::test]
    async fn in_flight_probe_past_the_limit_times_out() {
        let root = Temporary::new();
        let held = PROBES.lock().await;
        let error = settled(&root.0, Duration::from_millis(50))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("timed out"), "{error}");
        drop(held);
        settled(&root.0, Duration::from_secs(1)).await.unwrap();
    }
}
