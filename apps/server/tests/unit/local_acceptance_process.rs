use super::*;

#[test]
fn stop_deadline_does_not_renew_unknown_process_authority() {
    let directory = std::env::temp_dir().join(format!(
        "local-heartbeat-{}",
        process::new_identity().unwrap()
    ));
    std::fs::create_dir(&directory).unwrap();
    let key = RunKey {
        run_id: "acceptance".into(),
        request_id: "acceptance".into(),
        incarnation: "acceptance".into(),
    };
    heartbeat(&directory, &key, None).unwrap();
    assert!(!directory.join("start.json").exists());
    let receipt = crate::execution::Receipt {
        key: key.clone(),
        process: process::identity(std::process::id()).unwrap(),
    };
    process::durable_write(&directory.join("identity.json"), &receipt).unwrap();
    heartbeat(&directory, &key, Some(Instant::now())).unwrap();
    assert_eq!(
        process::read::<RunKey>(&directory.join("start.json")).unwrap(),
        key
    );
    let previous = std::fs::read(directory.join("storage-heartbeat.json")).unwrap();
    let old = Instant::now() - Duration::from_secs(21);
    assert!(
        heartbeat(&directory, &key, Some(old))
            .unwrap_err()
            .to_string()
            .contains("stop unknown")
    );
    assert_eq!(
        std::fs::read(directory.join("storage-heartbeat.json")).unwrap(),
        previous
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn replay_without_quiescence_stops_original_identity_without_launching() {
    let directory = std::env::temp_dir().join(format!(
        "local-unknown-{}",
        process::new_identity().unwrap()
    ));
    let key = RunKey {
        run_id: "original".into(),
        request_id: "original".into(),
        incarnation: "original".into(),
    };
    let job: Job = serde_json::from_value(serde_json::json!({
        "invocation":"original", "output_limit":1024, "checkouts":[],
        "plan":{"entry":"/must-not-execute","entry_sha256":"unknown","steps":[]},
        "binding":{"requirement":1,"revision":1,"authorization":1,"input_sha256":"original","versions":[],"required":[],"trusted":{"command_sha256":"unknown","config_sha256":"unknown","protected_entry":"/must-not-execute","protected_entry_sha256":"unknown","tool":"unknown","tool_version":"unknown"}}
    })).unwrap();
    assert!(
        start_or_reconcile(&directory, &job, &key, false)
            .unwrap_err()
            .to_string()
            .contains("outcome unknown")
    );
    assert_eq!(
        process::read::<RunKey>(&directory.join("stop.json")).unwrap(),
        key
    );
    assert!(!directory.join("launch.json").exists());
    assert!(!directory.join("job.json").exists());
    std::fs::remove_dir_all(directory).unwrap();
}
