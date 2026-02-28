#[test]
fn migration_docs_require_old_to_new_mapping_for_breaking_contract_changes() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let config_migration = std::fs::read_to_string(
        repo_root
            .join("docs")
            .join("crates")
            .join("config")
            .join("MIGRATION.md"),
    )
    .expect("config migration doc should be readable");
    let validator_migration = std::fs::read_to_string(
        repo_root
            .join("docs")
            .join("crates")
            .join("validator")
            .join("MIGRATION.md"),
    )
    .expect("validator migration doc should be readable");

    assert!(
        config_migration.contains("old -> new") || config_migration.contains("Old Behavior"),
        "config migration doc must include explicit old->new mapping guidance"
    );
    assert!(
        validator_migration.contains("Old Behavior")
            && validator_migration.contains("New Behavior"),
        "validator migration doc must include explicit old->new mapping table"
    );
}
