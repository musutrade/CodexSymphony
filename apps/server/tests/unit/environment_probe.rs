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
