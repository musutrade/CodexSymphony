use super::*;

#[test]
fn error_classification_requires_matching_stop_receipts_before_timeout_or_cancel() {
    let root = std::env::temp_dir().join(process::new_identity().unwrap());
    fs::create_dir(&root).unwrap();
    let error = || Err("controlled probe error".into());
    assert_eq!(classify_result(error(), &root).0, "failed");
    let receipt = serde_json::json!({"key":{"run_id":"r","request_id":"q","incarnation":"i"},"process":{"pid":1,"group":1,"start_ticks":1,"boot_id":"fixture"}});
    process::durable_write(&root.join("identity.json"), &receipt).unwrap();
    process::durable_write(&root.join("timeout.json"), &true).unwrap();
    assert_eq!(classify_result(error(), &root).0, "unknown");
    process::durable_write(&root.join("quiescent.json"), &receipt).unwrap();
    assert_eq!(classify_result(error(), &root).0, "timeout");
    fs::remove_file(root.join("timeout.json")).unwrap();
    process::durable_write(&root.join("cancelled.json"), &true).unwrap();
    assert_eq!(classify_result(error(), &root).0, "cancelled");
    fs::remove_file(root.join("cancelled.json")).unwrap();
    assert_eq!(classify_result(error(), &root).0, "unknown");
    fs::remove_dir_all(root).unwrap();
}
