use super::*;

#[test]
fn startup_timeout_persists_stop_and_never_authorizes_start() {
    let directory = std::env::temp_dir().join(format!(
        "delivery-start-{}",
        process::new_identity().unwrap()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let key = RunKey {
        run_id: "run".into(),
        request_id: "request".into(),
        incarnation: "boot".into(),
    };
    check_startup(&directory, &key, Instant::now()).unwrap();
    heartbeat(&directory, &key).unwrap();
    assert!(!directory.join("start.json").exists());
    let expired = Instant::now() - Duration::from_secs(11);
    assert!(check_startup(&directory, &key, expired).is_err());
    let stopped: RunKey = process::read(&directory.join("stop.json")).unwrap();
    assert_eq!(stopped, key);
    process::durable_write(&directory.join("identity.json"), &key).unwrap();
    check_startup(&directory, &key, expired).unwrap();
    heartbeat(&directory, &key).unwrap();
    let started: RunKey = process::read(&directory.join("start.json")).unwrap();
    assert_eq!(started, key);
    check_stop(None).unwrap();
    check_stop(Some(Instant::now())).unwrap();
    assert!(check_stop(Some(Instant::now() - Duration::from_secs(21))).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn child_reaping_waits_for_exit() {
    let child = std::process::Command::new("sh")
        .args(["-c", "sleep 0.1; exit 7"])
        .spawn()
        .unwrap();
    let pid = child.id();
    reap(child).await;
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn launch_intent_rejects_oversize_replay_and_partial_writes() {
    let root = std::env::temp_dir().join(process::new_identity().unwrap());
    let mut job: Job = serde_json::from_value(serde_json::json!({
        "call": {"identity":{"protocol_version":1,"requirement_id":1,"revision":1,
            "run_id":"run","resource_id":"resource","invocation_id":"invocation","attempt":1,"config_id":"config"},
            "controlled_config_digest":"config","operation":"before_deliver","extension_id":"hook",
            "implementation_digest":"digest","candidate":null,"environment_digest":"environment",
            "policy_digest":"policy","deadline_unix_ms":1,"required_checks":[]},
        "checkout":root,"directory":root.join("job"),
        "candidate":{"sha":"commit","tree":"tree","immutable":true},
        "plan":{"entry":"/reviewed","entry_sha256":"digest","steps":[]}
    })).unwrap();
    job.call.policy_digest = "x".repeat(65536);
    assert!(
        persist_launch(&job)
            .unwrap_err()
            .to_string()
            .contains("exceeds limit")
    );
    assert!(!job.directory.join("input.json").exists());
    job.call.policy_digest = "policy".into();
    std::fs::create_dir(job.directory.join("hook.json")).unwrap();
    assert!(persist_launch(&job).is_err());
    assert!(job.directory.join("input.json").is_file());
    assert!(
        persist_launch(&job)
            .unwrap_err()
            .to_string()
            .contains("already has intent")
    );
    std::fs::remove_dir_all(&job.directory).unwrap();
    std::fs::create_dir_all(job.directory.join("hook-limit.json")).unwrap();
    assert!(persist_launch(&job).is_err());
    assert!(job.directory.join("hook.json").is_file());
    std::fs::remove_dir_all(&job.directory).unwrap();
    std::fs::write(&job.directory, "blocked directory").unwrap();
    assert!(persist_launch(&job).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
