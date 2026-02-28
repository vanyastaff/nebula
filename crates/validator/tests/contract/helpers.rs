use nebula_validator::foundation::ValidationError;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct FixtureCase {
    pub id: String,
    pub scenario: String,
    pub input: Value,
    pub expected: FixtureExpectation,
}

#[derive(Debug, Deserialize)]
pub struct FixtureExpectation {
    pub pass: bool,
    pub error_code: Option<String>,
    pub field_path: Option<String>,
}

pub fn load_contract_fixture() -> Vec<FixtureCase> {
    let raw = include_str!("../fixtures/compat/minor_contract_v1.json");
    serde_json::from_str(raw).expect("compat fixture JSON must be valid")
}

pub fn assert_error_contract(
    error: &ValidationError,
    expected_code: Option<&str>,
    expected_field: Option<&str>,
) {
    if let Some(code) = expected_code {
        assert_eq!(error.code.as_ref(), code, "unexpected error code");
    }
    if let Some(field) = expected_field {
        assert_eq!(error.field.as_deref(), Some(field), "unexpected field path");
    }
}

pub fn assert_no_secrets(text: &str) {
    let forbidden = [
        "super-secret",
        "p@ssw0rd",
        "api-token-123",
        "bearer_verysecret",
    ];
    for needle in forbidden {
        assert!(
            !text.contains(needle),
            "sensitive token leaked in diagnostic text: {needle}"
        );
    }
}
