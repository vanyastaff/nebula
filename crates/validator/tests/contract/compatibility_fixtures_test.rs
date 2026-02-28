use super::helpers::{assert_error_contract, load_contract_fixture};
use nebula_validator::foundation::{Validate, ValidateExt};
use nebula_validator::validators::{alphanumeric, max_length, min_length};

#[test]
fn contract_fixtures_preserve_minor_release_behavior() {
    let fixtures = load_contract_fixture();
    let username = min_length(3).and(max_length(20)).and(alphanumeric());

    for case in fixtures {
        let input = case.input.as_str().unwrap_or_else(|| {
            panic!(
                "fixture {} ({}) must use string input",
                case.id, case.scenario
            )
        });
        let result = username.validate(input);

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err("fixture expected failure");
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}
