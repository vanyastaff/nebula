use super::helpers::fixture_path;
use nebula_config::core::error::ContractErrorCategory;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CategoryFixture {
    categories: Vec<CategoryPair>,
}

#[derive(Debug, Deserialize)]
struct CategoryPair {
    config: String,
    validator: String,
}

#[test]
fn config_and_validator_category_names_stay_compatible() {
    let raw = std::fs::read_to_string(fixture_path("compat/validator_contract_v1.json"))
        .expect("validator contract fixture should exist");
    let fixture: CategoryFixture =
        serde_json::from_str(&raw).expect("validator contract fixture should parse");

    for pair in fixture.categories {
        assert_eq!(
            pair.config, pair.validator,
            "config and validator category names must stay in lockstep"
        );
    }

    let config_categories = [
        ContractErrorCategory::SourceLoadFailed.as_str(),
        ContractErrorCategory::MergeFailed.as_str(),
        ContractErrorCategory::ValidationFailed.as_str(),
        ContractErrorCategory::MissingPath.as_str(),
        ContractErrorCategory::TypeMismatch.as_str(),
        ContractErrorCategory::InvalidValue.as_str(),
        ContractErrorCategory::WatcherFailed.as_str(),
    ];
    let validator_categories = [
        "source_load_failed",
        "merge_failed",
        "validation_failed",
        "missing_path",
        "type_mismatch",
        "invalid_value",
        "watcher_failed",
    ];

    assert_eq!(config_categories, validator_categories);
}
