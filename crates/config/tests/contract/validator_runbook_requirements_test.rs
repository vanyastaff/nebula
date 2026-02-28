#[test]
fn runbook_docs_cover_validation_failure_and_recovery_steps() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");

    let config_reliability = std::fs::read_to_string(
        repo_root
            .join("docs")
            .join("crates")
            .join("config")
            .join("RELIABILITY.md"),
    )
    .expect("config reliability doc should be readable");
    let config_interactions = std::fs::read_to_string(
        repo_root
            .join("docs")
            .join("crates")
            .join("config")
            .join("INTERACTIONS.md"),
    )
    .expect("config interactions doc should be readable");

    assert!(
        config_reliability.contains("validator rejection")
            || config_reliability.contains("validation failure"),
        "reliability doc must include validation failure triggers"
    );
    assert!(
        config_reliability.contains("preserve last-known-good active snapshot"),
        "reliability doc must include last-known-good recovery behavior"
    );
    assert!(
        config_interactions.contains("downstream consumer requirements")
            || config_interactions.contains("consumer CI"),
        "interactions doc must include downstream contract requirements"
    );
}
