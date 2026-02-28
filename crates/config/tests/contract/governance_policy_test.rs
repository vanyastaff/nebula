#[test]
fn governance_docs_require_additive_minor_and_major_for_breaking_changes() {
    let decisions_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("crates")
        .join("config")
        .join("DECISIONS.md");
    let proposals_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("crates")
        .join("config")
        .join("PROPOSALS.md");

    let decisions = std::fs::read_to_string(decisions_path).expect("DECISIONS.md should be read");
    let proposals = std::fs::read_to_string(proposals_path).expect("PROPOSALS.md should be read");

    assert!(
        decisions.contains("major") && decisions.contains("precedence"),
        "decisions must document major-version rule for precedence/path changes"
    );
    assert!(
        proposals.contains("compatibility") || proposals.contains("migration"),
        "proposals must include compatibility governance notes"
    );
}
