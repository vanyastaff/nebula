use nebula_validator::{
    foundation::{Validate, ValidateExt},
    validators::{
        alphabetic, alphanumeric, contains, date, date_time, ends_with, exact_length, hostname,
        ip_addr, ipv4, ipv6, is_false, is_true, length_range, lowercase, max_length, min_length,
        not_empty, numeric, starts_with, time, uppercase, uuid,
    },
};

use super::helpers::{assert_error_contract, load_contract_fixture, load_named_fixture};

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

#[test]
fn boolean_fixtures_preserve_contract() {
    let fixtures = load_named_fixture("boolean");

    for case in &fixtures {
        let input = case.input.as_bool().unwrap_or_else(|| {
            panic!(
                "fixture {} ({}) must use bool input",
                case.id, case.scenario
            )
        });

        let result = if case.id.contains("true") {
            is_true().validate(&input)
        } else {
            is_false().validate(&input)
        };

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err(&format!("fixture {} expected failure", case.id));
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}

#[test]
fn pattern_fixtures_preserve_contract() {
    let fixtures = load_named_fixture("pattern");

    for case in &fixtures {
        let input = case.input.as_str().unwrap_or_else(|| {
            panic!(
                "fixture {} ({}) must use string input",
                case.id, case.scenario
            )
        });

        let result = if case.id.contains("alphanumeric") {
            alphanumeric().validate(input)
        } else if case.id.contains("alphabetic") {
            alphabetic().validate(input)
        } else if case.id.contains("numeric") {
            numeric().validate(input)
        } else if case.id.contains("lowercase") {
            lowercase().validate(input)
        } else if case.id.contains("uppercase") {
            uppercase().validate(input)
        } else if case.id.contains("contains") {
            contains("world").validate(input)
        } else if case.id.contains("starts-with") {
            starts_with("hello").validate(input)
        } else if case.id.contains("ends-with") {
            ends_with("world").validate(input)
        } else {
            panic!("unknown pattern fixture: {}", case.id);
        };

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err(&format!("fixture {} expected failure", case.id));
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}

#[test]
fn network_fixtures_preserve_contract() {
    let fixtures = load_named_fixture("network");

    for case in &fixtures {
        let input = case.input.as_str().unwrap_or_else(|| {
            panic!(
                "fixture {} ({}) must use string input",
                case.id, case.scenario
            )
        });

        let result = if case.id.contains("ipv4") {
            ipv4().validate(input)
        } else if case.id.contains("ipv6") {
            ipv6().validate(input)
        } else if case.id.contains("ipaddr") {
            ip_addr().validate(input)
        } else if case.id.contains("hostname") {
            hostname().validate(input)
        } else {
            panic!("unknown network fixture: {}", case.id);
        };

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err(&format!("fixture {} expected failure", case.id));
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}

#[test]
fn temporal_fixtures_preserve_contract() {
    let fixtures = load_named_fixture("temporal");

    for case in &fixtures {
        let input = case.input.as_str().unwrap_or_else(|| {
            panic!(
                "fixture {} ({}) must use string input",
                case.id, case.scenario
            )
        });

        let result = if case.id.contains("datetime") {
            date_time().validate(input)
        } else if case.id.contains("date") {
            date().validate(input)
        } else if case.id.contains("time") {
            time().validate(input)
        } else if case.id.contains("uuid") {
            uuid().validate(input)
        } else {
            panic!("unknown temporal fixture: {}", case.id);
        };

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err(&format!("fixture {} expected failure", case.id));
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}

#[test]
fn length_fixtures_preserve_contract() {
    let fixtures = load_named_fixture("length");

    for case in &fixtures {
        let input = case.input.as_str().unwrap_or_else(|| {
            panic!(
                "fixture {} ({}) must use string input",
                case.id, case.scenario
            )
        });

        let result = if case.id.contains("not-empty") {
            not_empty().validate(input)
        } else if case.id.contains("exact") {
            exact_length(5).validate(input)
        } else if case.id.contains("range") {
            length_range(3, 10).expect("valid range").validate(input)
        } else if case.id.contains("min") {
            min_length(3).validate(input)
        } else if case.id.contains("max") {
            max_length(5).validate(input)
        } else {
            panic!("unknown length fixture: {}", case.id);
        };

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err(&format!("fixture {} expected failure", case.id));
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}

#[test]
fn field_path_fixtures_preserve_contract() {
    use nebula_validator::combinators::json_field;

    let fixtures = load_named_fixture("field_path");

    for case in &fixtures {
        let input = &case.input;

        let result = if case.id.contains("nested") || case.id.contains("missing") {
            let v = json_field::<_, str>("/user/name", min_length(3));
            v.validate(input)
        } else if case.id.contains("root") {
            let v = json_field::<_, str>("/email", not_empty());
            v.validate(input)
        } else {
            panic!("unknown field_path fixture: {}", case.id);
        };

        if case.expected.pass {
            assert!(
                result.is_ok(),
                "fixture {} ({}) expected pass but failed: {:?}",
                case.id,
                case.scenario,
                result.err()
            );
        } else {
            let err = result.expect_err(&format!("fixture {} expected failure", case.id));
            assert_error_contract(
                &err,
                case.expected.error_code.as_deref(),
                case.expected.field_path.as_deref(),
            );
        }
    }
}
