#[test]
fn governance_docs_require_additive_minor_policy_for_config_validator_contract() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let config_decisions = std::fs::read_to_string(
        repo_root
            .join("docs")
            .join("crates")
            .join("config")
            .join("DECISIONS.md"),
    )
    .expect("config decisions doc should be readable");
    let validator_decisions = std::fs::read_to_string(
        repo_root
            .join("docs")
            .join("crates")
            .join("validator")
            .join("DECISIONS.md"),
    )
    .expect("validator decisions doc should be readable");

    assert!(
        config_decisions.contains("minor releases remain additive")
            || config_decisions.contains("minor releases are additive"),
        "config governance must document additive minor policy"
    );
    assert!(
        validator_decisions.contains("minor releases are additive only")
            || validator_decisions.contains("additive-only minor evolution"),
        "validator governance must document additive minor policy"
    );
}
