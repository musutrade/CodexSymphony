use super::*;

#[test]
fn missing_startup_identity_requests_stop_without_restarting() {
    let directory = std::env::temp_dir().join(format!("validation-startup-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let key = RunKey {
        run_id: "validation".into(),
        request_id: "request".into(),
        incarnation: "first".into(),
    };
    check_startup(&directory, &key, Instant::now()).unwrap();
    assert!(!directory.join("stop.json").exists());
    let expired = Instant::now() - Duration::from_secs(11);
    assert!(check_startup(&directory, &key, expired).is_err());
    let stopped: RunKey = process::read(&directory.join("stop.json")).unwrap();
    assert_eq!(stopped, key);
    std::fs::write(directory.join("identity.json"), b"identity").unwrap();
    check_startup(&directory, &key, expired).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
