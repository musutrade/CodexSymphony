use super::*;

#[test]
fn proof_classification_never_accepts_an_unstartable_correction() {
    // An approved same-candidate generation is the only way past invalidation.
    assert_eq!(
        classify_proof(true, true, false, true).unwrap(),
        "revalidation_pending"
    );
    let interrupted = classify_proof(true, true, true, false).unwrap_err();
    assert!(interrupted.to_string().contains("approve a same-candidate"));
    assert_eq!(classify_proof(true, false, true, false).unwrap(), "current");
    // Legacy proof without an environment call has no deadline to expire.
    assert_eq!(
        classify_proof(false, false, false, false).unwrap(),
        "current"
    );
    let expired = classify_proof(true, false, false, false).unwrap_err();
    assert!(expired.to_string().contains("expired without interruption"));
}
