use std::fmt::Write as _;

use super::{VerificationError, validate_manifest_source, verify_documents, verify_file};

const ACTIVE_PROFILE_MANIFEST: &str = include_str!("../../gates/runtime-repair-red-v1.toml");
const NEXTEST_CONFIG: &str = include_str!("../../../../.config/nextest.toml");
const EXPECTED_RED_WORKFLOW: &str =
    include_str!("../../../../.github/workflows/runtime-repair-red.yml");
const SCENARIO_TARGET: &str =
    include_str!("../../../../apps/server/tests/runtime_repair_red_scenarios.rs");
const COMPONENT_C7_TARGET: &str =
    include_str!("../../../../apps/server/tests/runtime_repair_red_scenarios/component_c7.rs");

const EMPTY_TEST_MANIFEST: &str = r#"
manifest_version = 1
profile = "runtime-repair-red"
package = "nebula-server"
feature = "runtime-repair-red"
test_binary = "runtime_repair_red_scenarios"
expected_failures = []
"#;

const ACTIVE_TEST_MANIFEST: &str = r#"
manifest_version = 1
profile = "runtime-repair-red"
package = "nebula-server"
feature = "runtime-repair-red"
test_binary = "runtime_repair_red_scenarios"

[[expected_failures]]
test_name = "c0::restart_resume"
reason_code = "c0-split-control-path"

[[expected_failures]]
test_name = "c1::same_key_cancel"
reason_code = "c1-ambiguous-acceptance"
"#;

// Test-only parser fixture. It is synthetic verifier input and is never
// product RED evidence or consumed by the runtime-repair workflow.
fn positive_junit_fixture() -> String {
    junit_report(
        Counts::new(2, 2, 0, 0),
        Counts::new(2, 2, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
        ],
    )
}

/// Every manifest entry must name a test that actually exists.
///
/// The count is a lower bound rather than an exact number: entries are removed
/// as their defect is repaired, so pinning the exact count would turn every
/// repair into a failing gate. What must not drift is the other direction — a
/// manifest naming a test that does not exist would silently excuse a case that
/// never ran.
#[test]
fn active_profile_manifest_is_valid_and_names_all_genuine_cases() {
    let manifest = validate_manifest_source(ACTIVE_PROFILE_MANIFEST).expect("manifest is valid");
    assert!(
        !manifest.expected_failures.is_empty(),
        "an empty manifest would make the verifier vacuous"
    );
    for expected in &manifest.expected_failures {
        assert!(
            SCENARIO_TARGET.contains(&format!("async fn {}", expected.test_name))
                || COMPONENT_C7_TARGET.contains(&format!(
                    "async fn {}",
                    expected
                        .test_name
                        .strip_prefix("component_c7::")
                        .unwrap_or(&expected.test_name)
                )),
            "manifest case `{}` must name a real test",
            expected.test_name
        );
    }
}

#[test]
fn dedicated_profile_is_serial_and_retry_free() {
    let config: toml::Value = toml::from_str(NEXTEST_CONFIG).expect("nextest config is TOML");
    let profile = config
        .get("profile")
        .and_then(|profiles| profiles.get("runtime-repair-red"))
        .expect("dedicated expected-RED profile exists");

    assert_eq!(
        profile
            .get("test-threads")
            .and_then(toml::Value::as_integer),
        Some(1)
    );
    assert_eq!(
        profile.get("retries").and_then(toml::Value::as_integer),
        Some(0)
    );
    assert_eq!(
        profile.get("default-filter").and_then(toml::Value::as_str),
        Some("binary(=runtime_repair_red_scenarios)")
    );
}

#[test]
fn workflow_does_not_force_ignored_or_mask_infrastructure_failures() {
    assert!(!EXPECTED_RED_WORKFLOW.contains("--run-ignored"));
    assert!(!EXPECTED_RED_WORKFLOW.contains("continue-on-error"));
    assert!(
        !EXPECTED_RED_WORKFLOW.contains("\n    paths:"),
        "runtime changes outside the harness must not bypass expected-RED reconciliation"
    );
    assert!(EXPECTED_RED_WORKFLOW.contains("nextest_status=0"));
    assert!(EXPECTED_RED_WORKFLOW.contains("--nextest-exit-code \"$nextest_status\""));
    assert!(EXPECTED_RED_WORKFLOW.contains(".expected_failure_count > 0"));
    assert!(EXPECTED_RED_WORKFLOW.contains("--lib"));
    assert!(EXPECTED_RED_WORKFLOW.contains("runtime_repair_red::"));
    assert!(EXPECTED_RED_WORKFLOW.contains("steps.verify_red.outcome == 'success'"));
}

#[test]
fn active_target_contains_no_fake_or_suppressed_test() {
    for source in [SCENARIO_TARGET, COMPONENT_C7_TARGET] {
        for forbidden_source in [
            "#[ignore]",
            "#[should_panic]",
            "fn sentinel",
            "fn fake",
            "fn placeholder",
            "tokio::time::sleep",
            "std::thread::sleep",
        ] {
            assert!(
                !source.contains(forbidden_source),
                "scenario target contains forbidden source `{forbidden_source}`"
            );
        }
    }

    let manifest = validate_manifest_source(ACTIVE_PROFILE_MANIFEST).expect("manifest is valid");
    // The manifest is a *subset* of the tests in the target, not a mirror of
    // it. A repaired scenario stays in the suite as ordinary conformance
    // coverage and loses its manifest entry, so requiring equality here would
    // make every repair fail this guard.
    //
    // The direction that still matters is that the manifest cannot name more
    // cases than exist — that would mean an entry excusing a test that never
    // ran. Each entry is separately checked to name a real test in
    // `active_profile_manifest_is_valid_and_names_all_genuine_cases`.
    let selected_tests = SCENARIO_TARGET.matches("#[tokio::test]").count()
        + COMPONENT_C7_TARGET.matches("#[tokio::test").count();
    assert!(
        manifest.expected_failures.len() <= selected_tests,
        "manifest names {} cases but the target defines {selected_tests} tests",
        manifest.expected_failures.len()
    );
    assert_eq!(
        SCENARIO_TARGET.matches("EXPECTED_RED:").count(),
        1,
        "one product-root helper owns its exact marker emission"
    );
    assert_eq!(
        COMPONENT_C7_TARGET.matches("EXPECTED_RED:").count(),
        1,
        "one component-only helper owns its exact marker emission"
    );
}

#[test]
fn empty_active_set_cannot_verify_junit_red_evidence() {
    let error = verify_documents(
        EMPTY_TEST_MANIFEST,
        100,
        r#"<testsuites tests="0" failures="0" errors="0"></testsuites>"#,
    )
    .expect_err("empty expected set must fail closed");
    assert!(matches!(error, VerificationError::EmptyExpectedFailureSet));
}

#[test]
fn exact_expected_failures_and_markers_are_accepted() {
    let summary = verify_documents(ACTIVE_TEST_MANIFEST, 100, &positive_junit_fixture())
        .expect("test-only positive fixture verifies");
    assert_eq!(summary.expected_failure_count, 2);
    assert_eq!(summary.verified_failure_count, 2);
}

#[test]
fn only_raw_nextest_test_run_failed_exit_is_accepted() {
    for exit_code in [0, 1, 101] {
        let error = verify_documents(ACTIVE_TEST_MANIFEST, exit_code, &positive_junit_fixture())
            .expect_err("non-test-failure exit must be rejected");
        assert!(matches!(
            error,
            VerificationError::UnexpectedNextestExit { actual } if actual == exit_code
        ));
    }
}

#[test]
fn missing_junit_file_is_rejected() {
    let manifest = validate_manifest_source(ACTIVE_TEST_MANIFEST).expect("manifest is valid");
    let temporary = tempfile::tempdir().expect("temporary directory");
    let missing_path = temporary.path().join("missing.junit.xml");
    let error =
        verify_file(&manifest, 100, &missing_path).expect_err("missing JUnit must be rejected");
    assert!(matches!(
        error,
        VerificationError::JunitRead { path, .. } if path == missing_path
    ));
}

#[test]
fn malformed_junit_is_rejected() {
    assert_invalid_junit("<testsuites><testsuite>");
}

#[test]
fn passing_case_is_rejected_even_when_counts_are_self_consistent() {
    let report = junit_report(
        Counts::new(2, 1, 0, 0),
        Counts::new(2, 1, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            CaseFixture::new("c1::same_key_cancel", ""),
        ],
    );
    assert_invalid_junit(&report);
}

/// A repaired scenario that regresses must fail the gate.
///
/// Promotion removes a case's manifest entry, so the verifier no longer expects
/// it to fail. The risk that introduces is the opposite of the original one: a
/// case that is silently allowed to fail because nothing lists it. A repaired
/// case is therefore required to have *passed*, not merely to be absent from
/// the manifest.
#[test]
fn a_promoted_case_that_fails_again_is_rejected() {
    let report = junit_report(
        // Two failures where the manifest expects one: the promoted case
        // regressed.
        Counts::new(3, 2, 0, 0),
        Counts::new(3, 2, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
            failing_case("promoted::already_repaired", "EXPECTED_RED:some-reason"),
        ],
    );
    assert_invalid_junit(&report);
}

/// A promoted scenario that passes alongside the expected failures is accepted.
///
/// This is the shape every repair produces, so it must reconcile cleanly —
/// otherwise the only way to keep the gate green would be to delete the
/// repaired scenario, which is the evidence deletion the verifier exists to
/// prevent.
#[test]
fn a_promoted_passing_case_reconciles_with_the_expected_failures() {
    let report = junit_report(
        Counts::new(3, 2, 0, 0),
        Counts::new(3, 2, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
            CaseFixture::new("promoted::already_repaired", ""),
        ],
    );
    let summary = verify_documents(ACTIVE_TEST_MANIFEST, 100, &report)
        .expect("a repaired case that passes must reconcile");
    assert_eq!(
        summary.verified_failure_count, 2,
        "only the manifested cases count as verified failures"
    );
}

#[test]
fn skipped_case_is_rejected() {
    let report = junit_report(
        Counts::new(2, 1, 0, 1),
        Counts::new(2, 1, 0, 1),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            CaseFixture::new("c1::same_key_cancel", "<skipped/>"),
        ],
    );
    assert_invalid_junit(&report);
}

#[test]
fn execution_error_case_is_rejected() {
    let report = junit_report(
        Counts::new(2, 1, 1, 0),
        Counts::new(2, 1, 1, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            CaseFixture::new(
                "c1::same_key_cancel",
                "<error type=\"execution error\">could not start test</error>",
            ),
        ],
    );
    assert_invalid_junit(&report);
}

#[test]
fn timeout_failure_marker_is_rejected() {
    let report = junit_report(
        Counts::new(2, 2, 0, 0),
        Counts::new(2, 2, 0, 0),
        &[
            CaseFixture::new(
                "c0::restart_resume",
                "<failure type=\"test timeout\">terminated after 120s</failure>\
                 <system-err>EXPECTED_RED:c0-split-control-path</system-err>",
            ),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
        ],
    );
    assert_invalid_junit(&report);
}

/// Red→green: a real product failure whose panic text mentions a timeout is
/// not a harness timeout.
///
/// The classifier used to scan the free-form `message` attribute as well as
/// `type`. These scenarios assert on lifecycle waits, so a genuine defect
/// routinely surfaces as "resume timed out waiting for ..." — indistinguishable
/// from a nextest kill under the old rule, which rejected the entire run and
/// reported an infrastructure fault for a real product gap.
#[test]
fn genuine_failure_whose_message_mentions_a_timeout_is_still_accepted() {
    let report = junit_report(
        Counts::new(2, 2, 0, 0),
        Counts::new(2, 2, 0, 0),
        &[
            CaseFixture::new(
                "c0::restart_resume",
                "<failure type=\"test failure\" message=\"resume timed out waiting for \
                 the control queue\">expected product gap</failure>\
                 <system-err>EXPECTED_RED:c0-split-control-path</system-err>",
            ),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
        ],
    );

    let summary = verify_documents(ACTIVE_TEST_MANIFEST, 100, &report)
        .expect("a genuine failure must not be reclassified as a harness timeout");
    assert_eq!(summary.expected_failure_count, 2);
    assert_eq!(summary.verified_failure_count, 2);
}

#[test]
fn failure_body_cannot_supply_the_expected_red_marker() {
    let report = junit_report(
        Counts::new(2, 2, 0, 0),
        Counts::new(2, 2, 0, 0),
        &[
            CaseFixture::new(
                "c0::restart_resume",
                "<failure type=\"test failure\">\
                 EXPECTED_RED:c0-split-control-path\
                 </failure>",
            ),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
        ],
    );
    assert_invalid_junit(&report);
}

#[test]
fn retry_rerun_and_flaky_elements_are_rejected() {
    for forbidden_element in [
        "<retry/>",
        "<rerunFailure type=\"test failure\">retry</rerunFailure>",
        "<rerunError type=\"execution error\">retry</rerunError>",
        "<flakyFailure type=\"test failure\">retry</flakyFailure>",
        "<flakyError type=\"execution error\">retry</flakyError>",
    ] {
        let report = junit_report(
            Counts::new(2, 2, 0, 0),
            Counts::new(2, 2, 0, 0),
            &[
                CaseFixture::new(
                    "c0::restart_resume",
                    format!(
                        "<failure type=\"test failure\">expected</failure>\
                         {forbidden_element}\
                         <system-err>EXPECTED_RED:c0-split-control-path</system-err>"
                    ),
                ),
                failing_case(
                    "c1::same_key_cancel",
                    "EXPECTED_RED:c1-ambiguous-acceptance",
                ),
            ],
        );
        assert_invalid_junit(&report);
    }
}

#[test]
fn wrong_missing_and_duplicate_reason_markers_are_rejected() {
    for marker_output in [
        "EXPECTED_RED:wrong-reason",
        "ordinary panic without a marker",
        "EXPECTED_RED:c0-split-control-path\nEXPECTED_RED:c0-split-control-path",
    ] {
        let report = junit_report(
            Counts::new(2, 2, 0, 0),
            Counts::new(2, 2, 0, 0),
            &[
                failing_case("c0::restart_resume", marker_output),
                failing_case(
                    "c1::same_key_cancel",
                    "EXPECTED_RED:c1-ambiguous-acceptance",
                ),
            ],
        );
        assert_invalid_junit(&report);
    }
}

#[test]
fn missing_extra_and_duplicate_test_identities_are_rejected() {
    let missing = junit_report(
        Counts::new(1, 1, 0, 0),
        Counts::new(1, 1, 0, 0),
        &[failing_case(
            "c0::restart_resume",
            "EXPECTED_RED:c0-split-control-path",
        )],
    );
    assert_invalid_junit(&missing);

    let extra = junit_report(
        Counts::new(3, 3, 0, 0),
        Counts::new(3, 3, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
            failing_case("c2::unexpected", "EXPECTED_RED:unexpected-case"),
        ],
    );
    assert_invalid_junit(&extra);

    let duplicate = junit_report(
        Counts::new(2, 2, 0, 0),
        Counts::new(2, 2, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
        ],
    );
    assert_invalid_junit(&duplicate);
}

#[test]
fn root_aggregate_and_suite_testcase_counts_are_reconciled() {
    let aggregate_mismatch = junit_report(
        Counts::new(3, 2, 0, 0),
        Counts::new(2, 2, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
        ],
    );
    assert_invalid_junit(&aggregate_mismatch);

    let testcase_mismatch = junit_report(
        Counts::new(3, 2, 0, 0),
        Counts::new(3, 2, 0, 0),
        &[
            failing_case("c0::restart_resume", "EXPECTED_RED:c0-split-control-path"),
            failing_case(
                "c1::same_key_cancel",
                "EXPECTED_RED:c1-ambiguous-acceptance",
            ),
        ],
    );
    assert_invalid_junit(&testcase_mismatch);
}

#[test]
fn manifest_requires_sorted_unique_exact_test_identities() {
    let unsorted = ACTIVE_TEST_MANIFEST.replace(
        "test_name = \"c0::restart_resume\"",
        "test_name = \"c9::restart_resume\"",
    );
    let error = validate_manifest_source(&unsorted).expect_err("unsorted names are invalid");
    assert!(matches!(error, VerificationError::InvalidManifest { .. }));

    let duplicate = ACTIVE_TEST_MANIFEST.replace(
        "test_name = \"c1::same_key_cancel\"",
        "test_name = \"c0::restart_resume\"",
    );
    let error = validate_manifest_source(&duplicate).expect_err("duplicate names are invalid");
    assert!(matches!(error, VerificationError::InvalidManifest { .. }));
}

fn assert_invalid_junit(source: &str) {
    let error =
        verify_documents(ACTIVE_TEST_MANIFEST, 100, source).expect_err("fixture must be rejected");
    assert!(
        matches!(error, VerificationError::InvalidJunit { .. }),
        "unexpected error: {error}"
    );
}

#[derive(Clone, Copy)]
struct Counts {
    tests: usize,
    failures: usize,
    errors: usize,
    disabled: usize,
}

impl Counts {
    const fn new(tests: usize, failures: usize, errors: usize, disabled: usize) -> Self {
        Self {
            tests,
            failures,
            errors,
            disabled,
        }
    }
}

struct CaseFixture {
    name: String,
    body: String,
}

impl CaseFixture {
    fn new(name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            body: body.into(),
        }
    }
}

fn failing_case(name: &str, marker_output: &str) -> CaseFixture {
    CaseFixture::new(
        name,
        format!(
            "<failure type=\"test failure\">expected product gap</failure>\
             <system-err>{marker_output}</system-err>"
        ),
    )
}

fn junit_report(root: Counts, suite: Counts, cases: &[CaseFixture]) -> String {
    let mut testcases = String::new();
    for case in cases {
        write!(
            &mut testcases,
            "<testcase name=\"{}\" classname=\"nebula-server::runtime_repair_red_scenarios\">\
             {}\
             </testcase>",
            case.name, case.body
        )
        .expect("writing to a String cannot fail");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <testsuites name=\"runtime-repair-red\" tests=\"{}\" failures=\"{}\" errors=\"{}\" disabled=\"{}\">\
         <testsuite name=\"nebula-server::runtime_repair_red_scenarios\" tests=\"{}\" failures=\"{}\" errors=\"{}\" disabled=\"{}\">\
         {testcases}\
         </testsuite>\
         </testsuites>",
        root.tests,
        root.failures,
        root.errors,
        root.disabled,
        suite.tests,
        suite.failures,
        suite.errors,
        suite.disabled
    )
}
