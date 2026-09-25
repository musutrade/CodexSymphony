use super::*;
use crate::merge_test_support::fixture::source;

fn job() -> Job {
    let (root, checkout, plan) = source::fixture();
    Job {
        candidate: crate::validation_runner::candidate(&checkout).unwrap(),
        checkout,
        directory: root.join("validation"),
        plan,
        limit: 4096,
    }
}

#[tokio::test]
async fn unavailable_worker_and_lost_response_remain_errors() {
    assert!(
        dispatch(&Err("worker initialization failed".into()), job())
            .await
            .unwrap_err()
            .to_string()
            .contains("initialization")
    );
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    assert!(
        dispatch(&Ok(sender), job())
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (_, reply) = receiver.recv().unwrap();
        drop(reply);
    });
    assert!(dispatch(&Ok(sender), job()).await.is_err());
    worker.join().unwrap();
}
