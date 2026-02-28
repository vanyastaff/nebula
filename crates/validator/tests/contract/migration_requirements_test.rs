#[test]
fn migration_doc_requires_mapping_for_breaking_changes() {
    let migration = include_str!("../../../../docs/crates/validator/MIGRATION.md");
    assert!(
        migration.contains("mapping"),
        "MIGRATION must include old->new mapping guidance"
    );
    assert!(
        migration.contains("major"),
        "MIGRATION must classify breaking changes as major"
    );
}

#[test]
fn roadmap_mentions_compatibility_checks_in_ci() {
    let roadmap = include_str!("../../../../docs/crates/validator/ROADMAP.md");
    assert!(
        roadmap.contains("compatibility tests"),
        "ROADMAP must mention compatibility tests"
    );
}
