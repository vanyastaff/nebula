#[test]
fn migration_docs_require_explicit_old_to_new_mapping() {
    let migration_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("crates")
        .join("config")
        .join("MIGRATION.md");
    let roadmap_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("crates")
        .join("config")
        .join("ROADMAP.md");

    let migration = std::fs::read_to_string(migration_path).expect("MIGRATION.md should be read");
    let roadmap = std::fs::read_to_string(roadmap_path).expect("ROADMAP.md should be read");

    assert!(
        migration.contains("Mapping template") || migration.contains("old -> new"),
        "migration docs must include explicit mapping guidance"
    );
    assert!(
        roadmap.contains("migration") && roadmap.contains("compatibility"),
        "roadmap must keep migration/compatibility milestones"
    );
}
