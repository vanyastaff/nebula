use nebula_validator::foundation::error::codes;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
struct ErrorRegistry {
    version: String,
    artifact: String,
    error_codes: Vec<RegistryCode>,
    categories: Vec<RegistryCategory>,
    change_policy: ChangePolicy,
}

#[derive(Debug, Deserialize)]
struct RegistryCode {
    code: String,
    meaning: String,
    stability: String,
}

#[derive(Debug, Deserialize)]
struct RegistryCategory {
    name: String,
    owner: String,
    cross_crate_contract: bool,
}

#[derive(Debug, Deserialize)]
struct ChangePolicy {
    minor_rule: String,
    major_rule: String,
    migration_authority: String,
}

fn load_error_registry() -> ErrorRegistry {
    let raw = include_str!("../fixtures/compat/error_registry_v1.json");
    serde_json::from_str(raw).expect("error registry JSON must be valid")
}

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

#[test]
fn registry_has_required_metadata_and_policy() {
    let registry = load_error_registry();
    assert_eq!(registry.version, "1.0.0");
    assert_eq!(registry.artifact, "validator_error_registry");
    assert_eq!(registry.change_policy.minor_rule, "additive_only");
    assert_eq!(
        registry.change_policy.major_rule,
        "semantic_change_or_removal_requires_migration_mapping"
    );
    assert_eq!(
        registry.change_policy.migration_authority,
        "docs/crates/validator/MIGRATION.md"
    );
}

#[test]
fn registry_contains_all_canonical_error_codes() {
    let registry = load_error_registry();
    let actual: HashSet<&str> = registry
        .error_codes
        .iter()
        .map(|entry| entry.code.as_str())
        .collect();

    let expected = [
        codes::REQUIRED,
        codes::MIN_LENGTH,
        codes::MAX_LENGTH,
        codes::INVALID_FORMAT,
        codes::TYPE_MISMATCH,
        codes::OUT_OF_RANGE,
        codes::EXACT_LENGTH,
        codes::LENGTH_RANGE,
        codes::CUSTOM,
    ];

    for code in expected {
        assert!(
            actual.contains(code),
            "registry missing canonical code: {code}"
        );
    }
}

#[test]
fn registry_has_no_duplicate_codes_or_categories() {
    let registry = load_error_registry();

    let mut codes_seen = HashSet::new();
    for entry in &registry.error_codes {
        assert!(
            !entry.code.is_empty() && !entry.meaning.is_empty() && !entry.stability.is_empty(),
            "registry code entries must be non-empty"
        );
        assert!(
            codes_seen.insert(entry.code.as_str()),
            "duplicate error code found: {}",
            entry.code
        );
    }

    let mut categories_seen = HashSet::new();
    for category in &registry.categories {
        assert!(
            !category.name.is_empty() && !category.owner.is_empty(),
            "registry category entries must be non-empty"
        );
        assert!(
            category.cross_crate_contract,
            "all v1 categories must be marked as cross-crate contract"
        );
        assert!(
            categories_seen.insert(category.name.as_str()),
            "duplicate category found: {}",
            category.name
        );
    }
}

#[test]
fn docs_reference_canonical_registry_artifact() {
    let api = include_str!("../../../../docs/crates/validator/API.md");
    let decisions = include_str!("../../../../docs/crates/validator/DECISIONS.md");
    let strategy = include_str!("../../../../docs/crates/validator/TEST_STRATEGY.md");

    assert!(api.contains("error_registry_v1.json"));
    assert!(decisions.contains("error_registry_v1.json"));
    assert!(strategy.contains("error_registry_v1.json"));
}
