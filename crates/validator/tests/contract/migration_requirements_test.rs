#[test]
fn migration_doc_requires_mapping_for_breaking_changes() {
    let migration = include_str!("../../docs/migration.md");
    assert!(
        migration.contains("mapping"),
        "MIGRATION must include old->new mapping guidance"
    );
    assert!(
        migration.contains("major"),
        "MIGRATION must classify breaking changes as major"
    );
}
