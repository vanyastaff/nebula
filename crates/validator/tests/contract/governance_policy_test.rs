#[test]
fn decisions_document_declares_additive_minor_policy() {
    let decisions = include_str!("../../../../docs/crates/validator/DECISIONS.md");
    assert!(
        decisions.contains("minor releases"),
        "DECISIONS must mention minor release policy"
    );
    assert!(
        decisions.contains("additive"),
        "DECISIONS must enforce additive-only guidance for minor releases"
    );
}

#[test]
fn api_document_declares_major_break_conditions() {
    let api = include_str!("../../../../docs/crates/validator/API.md");
    assert!(api.contains("major bump required"));
    assert!(api.contains("error code"));
    assert!(api.contains("field-path"));
}
