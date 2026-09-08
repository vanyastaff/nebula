use serde_json::{Value, json};

use super::{RequiredPostgresqlError, verify};

const POSTGRES_OBSERVED: &[u8] = include_bytes!("postgres_observed.json");
const DISABLED_OBSERVED: &[u8] = include_bytes!("disabled_observed.json");

fn observed() -> Value {
    let mut postgres = super::super::super::json::decode::<Value>(POSTGRES_OBSERVED).unwrap();
    let mut disabled = super::super::super::json::decode::<Value>(DISABLED_OBSERVED).unwrap();
    for field in ["contract", "producer_version", "scenario_inventory_version"] {
        assert_eq!(postgres[field], disabled[field]);
    }
    postgres["scenarios"]
        .as_array_mut()
        .unwrap()
        .append(disabled["scenarios"].as_array_mut().unwrap());
    postgres
}

#[test]
fn actual_feature_enabled_and_disabled_children_qualify_together() {
    assert_eq!(verify(&observed()), Ok(()));
}

#[test]
fn missing_feature_absence_observation_cannot_qualify() {
    let partial = super::super::super::json::decode::<Value>(POSTGRES_OBSERVED).unwrap();
    assert_eq!(verify(&partial), Err(RequiredPostgresqlError::Inventory));
}

#[test]
fn every_scenario_is_required_once_including_the_healthy_control() {
    let actual = observed();
    for index in 0..4 {
        let mut missing = actual.clone();
        missing["scenarios"].as_array_mut().unwrap().remove(index);
        assert_eq!(verify(&missing), Err(RequiredPostgresqlError::Inventory));
        let mut duplicate = actual.clone();
        duplicate["scenarios"][index] = actual["scenarios"][(index + 1) % 4].clone();
        assert_eq!(verify(&duplicate), Err(RequiredPostgresqlError::Inventory));
        let mut unknown = actual.clone();
        unknown["scenarios"][index]["scenario"] = json!("passed_test");
        assert_eq!(verify(&unknown), Err(RequiredPostgresqlError::Inventory));
    }
}

#[test]
fn process_identity_settings_and_exit_cannot_be_substituted() {
    let actual = observed();
    for index in 0..4 {
        let event = &actual["scenarios"][index]["events"][0];
        for (field, replacement) in [
            ("sequence", json!(1)),
            ("kind", json!("skipped")),
            ("test_case", json!("unrelated_test")),
            ("required_postgres", json!(false)),
            (
                "postgres_feature",
                json!(!event["postgres_feature"].as_bool().unwrap()),
            ),
            ("exit_code", json!(null)),
            ("exit_code", json!(124)),
            ("exit_code", json!(-9)),
            (
                "exit_code",
                json!(if event["exit_code"] == 0 { 101 } else { 0 }),
            ),
        ] {
            let mut mutation = actual.clone();
            mutation["scenarios"][index]["events"][0][field] = replacement;
            assert_eq!(
                verify(&mutation),
                Err(RequiredPostgresqlError::Process),
                "case {index}: {field}"
            );
        }
        let mut empty = actual.clone();
        empty["scenarios"][index]["events"] = json!([]);
        assert_eq!(verify(&empty), Err(RequiredPostgresqlError::Process));
        let mut duplicated = actual.clone();
        duplicated["scenarios"][index]["events"]
            .as_array_mut()
            .unwrap()
            .push(event.clone());
        assert_eq!(verify(&duplicated), Err(RequiredPostgresqlError::Process));
    }
}

#[test]
fn reports_require_one_real_case_and_exact_outcome_counts() {
    let actual = observed();
    for index in 0..4 {
        let stdout = actual["scenarios"][index]["events"][0]["stdout"]
            .as_str()
            .unwrap();
        let replacements = [
            stdout.replace("running 1 test", "running 0 tests"),
            stdout.replace("running 1 test", "running 2 tests"),
            stdout.replace("create_get_roundtrip::case_3_postgres", "sentinel_test"),
            stdout.replace("0 ignored", "1 ignored"),
            stdout.replace("0 measured", "1 measured"),
            stdout.replace(" filtered out", " retried"),
            stdout.replace("test result:", "fake result:"),
            format!("{stdout}running 1 test\n"),
            "running 1 test\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s\n".to_owned(),
        ];
        for replacement in replacements {
            let mut mutation = actual.clone();
            mutation["scenarios"][index]["events"][0]["stdout"] = json!(replacement);
            assert_eq!(
                verify(&mutation),
                Err(RequiredPostgresqlError::Report),
                "case {index}"
            );
        }
        let (before_duration, _) = stdout.split_once("finished in ").unwrap();
        for duration in ["45.01", "NaN", "inf", "-1.00", "1e9", "0.001"] {
            let mut mutation = actual.clone();
            mutation["scenarios"][index]["events"][0]["stdout"] =
                json!(format!("{before_duration}finished in {duration}s\n"));
            assert_eq!(verify(&mutation), Err(RequiredPostgresqlError::Report));
        }
    }
}

#[test]
fn failure_exit_requires_the_correct_absence_cause_not_an_unrelated_panic() {
    let actual = observed();
    for index in 0..4 {
        let stderr = actual["scenarios"][index]["events"][0]["stderr"]
            .as_str()
            .unwrap();
        for replacement in [
            "skipping Postgres case".to_owned(),
            "unrelated assertion failed".to_owned(),
            format!("{stderr}extra panic diagnostic\n"),
        ] {
            let mut mutation = actual.clone();
            mutation["scenarios"][index]["events"][0]["stderr"] = json!(replacement);
            assert_eq!(verify(&mutation), Err(RequiredPostgresqlError::Diagnostic));
        }
        if !stderr.is_empty() {
            let mut wrong_test = actual.clone();
            wrong_test["scenarios"][index]["events"][0]["stderr"] =
                json!(stderr.replace("create_get_roundtrip::case_3_postgres", "different_test"));
            assert_eq!(
                verify(&wrong_test),
                Err(RequiredPostgresqlError::Diagnostic)
            );
        }
    }
    let mut swapped_causes = actual.clone();
    swapped_causes["scenarios"][0]["events"][0]["stderr"] =
        actual["scenarios"][1]["events"][0]["stderr"].clone();
    assert_eq!(
        verify(&swapped_causes),
        Err(RequiredPostgresqlError::Diagnostic)
    );
}

#[test]
fn fields_are_closed_bounded_and_do_not_accept_success_flags() {
    let actual = observed();
    for pointer in ["", "/scenarios/0", "/scenarios/0/events/0"] {
        for field in actual.pointer(pointer).unwrap().as_object().unwrap().keys() {
            let mut missing = actual.clone();
            missing
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(verify(&missing).is_err());
        }
        let mut extra = actual.clone();
        extra
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("passed".to_owned(), json!(true));
        assert_eq!(verify(&extra), Err(RequiredPostgresqlError::Shape));
    }
    for field in ["stdout", "stderr"] {
        for replacement in [
            json!(null),
            json!("x".repeat(4097)),
            json!("\u{1b}[32msuccess"),
        ] {
            let mut mutation = actual.clone();
            mutation["scenarios"][0]["events"][0][field] = replacement;
            assert_eq!(verify(&mutation), Err(RequiredPostgresqlError::Shape));
        }
    }
    for field in ["producer_version", "scenario_inventory_version"] {
        let mut mutation = actual.clone();
        mutation[field] = json!(99);
        assert_eq!(verify(&mutation), Err(RequiredPostgresqlError::Version));
    }
}
