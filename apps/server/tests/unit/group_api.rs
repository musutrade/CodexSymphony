use super::*;

#[test]
fn authorization_retains_integration_only_repositories_and_excludes_unrelated_ones() {
    let document: crate::draft::Document =
        serde_json::from_str(include_str!("../fixtures/group-draft.json")).unwrap();
    let mut review: Review =
        serde_json::from_str(include_str!("../fixtures/group-review.json")).unwrap();
    assert!(uses_repository(
        &document,
        &review,
        document.children[0].repository_id.unwrap()
    ));
    assert!(!uses_repository(&document, &review, 999));
    review.items[0].integration = Some(crate::integration::Authorization {
        configuration_sha256: "a".repeat(64),
        repositories: vec![crate::integration::Repository {
            repository_id: 999,
            repository_version: 1,
            selection: crate::integration::Selection::Fixed {
                sha: "b".repeat(40),
            },
            repair_scope: "reviewed integration dependency".into(),
        }],
    });
    assert!(uses_repository(&document, &review, 999));
    assert!(!uses_repository(&document, &review, 1000));
}
