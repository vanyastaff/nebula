//! Hook routing contracts. Cargo is replaced by a recording process; no build runs here.

#![cfg(unix)]

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const OWNER: &str = "fixture-author-surface";
const OWNER_SOURCE: &str = "crates/surface/src/lib.rs";
const FIRST_MANIFEST: &str = "crates/surface/tests/fixtures/first/Cargo.toml";
const FIRST_POSITIVE: &str = "crates/surface/tests/fixtures/first/src/bin/positive.rs";
const SECOND_MANIFEST: &str = "crates/surface/tests/fixtures/second/Cargo.toml";
const SECOND_NEGATIVE: &str = "crates/surface/tests/fixtures/second/src/bin/negative.rs";
const STANDALONE_MANIFEST: &str = "tools/independent probe/Cargo.toml";
const STANDALONE_SOURCE: &str = "tools/independent probe/src/lib.rs";
const CONTRACT: &str = "external_contract";
const CLIPPY: &str = "pre-commit-clippy-changed.sh";
const FMT: &str = "pre-commit-fmt-check.sh";

#[test]
fn clippy_routes_fixtures_to_one_owner_contract_without_direct_fixture_builds() {
    let paths = [FIRST_POSITIVE, SECOND_NEGATIVE, FIRST_POSITIVE];
    let run = run_hook(CLIPPY, &paths, &fixture_plan(), Failure::None);
    assert_success(&run);

    assert_eq!(
        run.calls,
        vec![
            planner_call(&paths),
            arguments(&[
                "clippy",
                "--locked",
                "-p",
                OWNER,
                "--all-targets",
                "-q",
                "--",
                "-D",
                "warnings",
            ]),
            arguments(&[
                "nextest",
                "run",
                "--locked",
                "-p",
                OWNER,
                "--test",
                CONTRACT,
                "-j",
                "1",
                "--no-tests=fail",
            ]),
        ],
        "fixture changes must lint their actual owner and execute its contract once; \
         direct fixture clippy cannot prepare unpublished dependencies or negative probes"
    );
}

#[test]
fn clippy_propagates_the_owner_contract_failure() {
    let run = run_hook(
        CLIPPY,
        &[FIRST_POSITIVE],
        &fixture_plan(),
        Failure::Contract,
    );
    assert_eq!(
        run.output.status.code(),
        Some(100),
        "a failing owner contract must fail the hook, not disappear from selection: {run:?}"
    );
    assert_eq!(
        run.calls
            .iter()
            .filter(|call| call.first().is_some_and(|argument| argument == "nextest"))
            .count(),
        1,
        "the failing contract must actually execute"
    );
}

#[test]
fn fmt_checks_negative_sources_even_when_only_a_positive_source_changed() {
    let mut plan = fixture_plan();
    plan["fixtures"].as_array_mut().unwrap().truncate(1);
    let run = run_hook(FMT, &[FIRST_POSITIVE], &plan, Failure::Formatting);

    assert_eq!(
        run.output.status.code(),
        Some(1),
        "unformatted negative fixture source must still fail formatting: {run:?}"
    );
    assert!(
        run.calls.contains(&arguments(&[
            "fmt",
            "--manifest-path",
            FIRST_MANIFEST,
            "--",
            "--check",
        ])),
        "format the complete standalone fixture, not only staged or positive sources: {run:?}"
    );
    assert!(
        String::from_utf8_lossy(&run.output.stderr).contains("unformatted negative fixture source"),
        "the failure must come from checking the negative source, not an unrelated command"
    );
}

#[test]
fn hooks_preserve_ordinary_standalone_checks_and_paths_with_spaces() {
    let plan = json!({
        "schema_version": 1,
        "packages": [],
        "standalone_manifests": [STANDALONE_MANIFEST],
        "fixtures": [],
    });
    for (script, expected) in [
        (
            FMT,
            arguments(&[
                "fmt",
                "--manifest-path",
                STANDALONE_MANIFEST,
                "--",
                "--check",
            ]),
        ),
        (
            CLIPPY,
            arguments(&[
                "clippy",
                "--manifest-path",
                STANDALONE_MANIFEST,
                "--all-targets",
                "-q",
                "--",
                "-D",
                "warnings",
            ]),
        ),
    ] {
        let run = run_hook(script, &[STANDALONE_SOURCE], &plan, Failure::None);
        assert_success(&run);
        let checks = run
            .calls
            .iter()
            .filter(|call| call.first().is_none_or(|argument| argument != "xtask"))
            .collect::<Vec<_>>();
        assert_eq!(checks, [&expected], "standalone coverage changed: {run:?}");
    }
}

#[test]
fn hooks_reject_unknown_or_malformed_plan_shapes_before_running_checks() {
    let mut unknown_version = fixture_plan();
    unknown_version["schema_version"] = json!(2);
    let mut unknown_field = fixture_plan();
    unknown_field["skip_contracts"] = json!(true);
    let mut malformed_fixture = fixture_plan();
    malformed_fixture["fixtures"][0]["test_target"] = json!([CONTRACT]);

    for script in [FMT, CLIPPY] {
        for plan in [&unknown_version, &unknown_field, &malformed_fixture] {
            let paths = [OWNER_SOURCE, FIRST_POSITIVE];
            let run = run_hook(script, &paths, plan, Failure::None);
            assert!(
                !run.output.status.success(),
                "invalid plan must fail closed without partial validation: {run:?}"
            );
            assert_eq!(run.calls, [planner_call(&paths)]);
        }
    }
}

#[test]
fn hooks_propagate_planner_failure_without_falling_back_to_manifest_guessing() {
    for script in [FMT, CLIPPY] {
        let paths = [OWNER_SOURCE, FIRST_POSITIVE];
        let run = run_hook(script, &paths, &fixture_plan(), Failure::Planner);
        assert_eq!(run.output.status.code(), Some(23), "{run:?}");
        assert_eq!(run.calls, [planner_call(&paths)]);
    }
}

#[test]
fn hooks_reject_multiple_json_documents_without_starting_checks() {
    let plan = fixture_plan();
    let stream = format!("{plan}\n{plan}");
    for script in [FMT, CLIPPY] {
        let paths = [FIRST_POSITIVE];
        let run = run_hook_json(script, &paths, &stream, Failure::None);
        assert!(
            !run.output.status.success(),
            "JSON stream accepted: {run:?}"
        );
        assert_eq!(run.calls, [planner_call(&paths)]);
    }
}

#[test]
fn hooks_reject_control_characters_in_package_names() {
    for name in control_identifiers(OWNER) {
        let plan = json!({
            "schema_version": 1,
            "packages": [name],
            "standalone_manifests": [],
            "fixtures": [],
        });
        assert_rejected_before_checks(&plan);
    }
}

#[test]
fn hooks_reject_control_characters_in_test_target_names() {
    for name in control_identifiers(CONTRACT) {
        let mut plan = fixture_plan();
        plan["fixtures"][0]["test_target"] = json!(name);
        assert_rejected_before_checks(&plan);
    }
}

#[test]
fn hooks_propagate_each_jq_extraction_failure_before_starting_checks() {
    let mut plan = fixture_plan();
    plan["standalone_manifests"] = json!([STANDALONE_MANIFEST]);
    for (script, extraction_count) in [(FMT, 2), (CLIPPY, 3)] {
        for extraction in 1..=extraction_count {
            let paths = [FIRST_POSITIVE, STANDALONE_SOURCE];
            let run = run_hook(script, &paths, &plan, Failure::Extraction(extraction));
            assert_eq!(
                run.output.status.code(),
                Some(42),
                "jq extraction {extraction} must fail {script}, even after partial output: {run:?}"
            );
            assert!(
                String::from_utf8_lossy(&run.output.stderr)
                    .contains("injected jq extraction failure"),
                "the intended extraction must actually fail: {run:?}"
            );
            assert_eq!(run.calls, [planner_call(&paths)]);
        }
    }
}

#[test]
fn hooks_preserve_empty_plan_arrays_without_blank_arguments() {
    let plan = json!({
        "schema_version": 1,
        "packages": [],
        "standalone_manifests": [],
        "fixtures": [],
    });
    for script in [FMT, CLIPPY] {
        let paths = [OWNER_SOURCE];
        let run = run_hook(script, &paths, &plan, Failure::None);
        assert_success(&run);
        assert_eq!(run.calls, [planner_call(&paths)]);
    }
}

fn control_identifiers(name: &str) -> Vec<String> {
    ["\n", "\r", "\t", "\0", "\u{007f}", "\u{0085}"]
        .map(|control| format!("{name}{control}"))
        .into_iter()
        .chain([format!("{name}\n{name}"), format!("\n{name}")])
        .collect()
}

fn assert_rejected_before_checks(plan: &Value) {
    for script in [FMT, CLIPPY] {
        let paths = [FIRST_POSITIVE];
        let run = run_hook(script, &paths, plan, Failure::None);
        assert!(
            !run.output.status.success(),
            "invalid identifier accepted: {run:?}"
        );
        assert_eq!(run.calls, [planner_call(&paths)]);
    }
}

#[test]
fn empty_or_non_rust_input_does_not_start_cargo() {
    for script in [FMT, CLIPPY] {
        for paths in [&[][..], &["README.md"][..]] {
            let run = run_hook(script, paths, &fixture_plan(), Failure::None);
            assert_success(&run);
            assert!(run.calls.is_empty(), "unexpected Cargo invocation: {run:?}");
        }
    }
}

fn fixture_plan() -> Value {
    json!({
        "schema_version": 1,
        "packages": [OWNER],
        "standalone_manifests": [],
        "fixtures": [
            {"manifest_path": FIRST_MANIFEST, "owner": OWNER, "test_target": CONTRACT},
            {"manifest_path": SECOND_MANIFEST, "owner": OWNER, "test_target": CONTRACT},
        ],
    })
}

#[derive(Debug, Clone, Copy)]
enum Failure {
    None,
    Planner,
    Contract,
    Formatting,
    Extraction(usize),
}

#[derive(Debug)]
struct HookRun {
    output: Output,
    calls: Vec<Vec<String>>,
}

fn run_hook(script: &str, paths: &[&str], plan: &Value, failure: Failure) -> HookRun {
    run_hook_json(script, paths, &plan.to_string(), failure)
}

fn run_hook_json(script: &str, paths: &[&str], plan: &str, failure: Failure) -> HookRun {
    let fixture = fixture_repo();
    let shim_dir = fixture.path().join("command-shims");
    fs::create_dir(&shim_dir).expect("command-shim directory is writable");
    let shim = shim_dir.join("cargo");
    fs::write(
        &shim,
        r#"#!/usr/bin/env bash
set -euo pipefail
{
  printf '%s\n' '__CALL__'
  printf '%s\n' "$@"
} >> "$NEBULA_TEST_CARGO_LOG"
case "${1:-}" in
  xtask)
    [[ "${2:-}" == "pre-commit-plan" ]] || exit 97
    [[ "$NEBULA_TEST_FAILURE" != planner ]] || exit 23
    printf '%s\n' "$NEBULA_TEST_PRE_COMMIT_PLAN"
    ;;
  nextest)
    [[ "$NEBULA_TEST_FAILURE" != contract ]] || exit 100
    ;;
  fmt)
    if [[ "$NEBULA_TEST_FAILURE" == formatting && "${2:-}" == --manifest-path && "${3:-}" == "$NEBULA_TEST_FIXTURE_MANIFEST" ]]; then
      printf '%s\n' 'unformatted negative fixture source' >&2
      exit 1
    fi
    ;;
  clippy) ;;
  *) printf 'unexpected fake Cargo command: %s\n' "${1:-}" >&2; exit 98 ;;
esac
"#,
    )
    .expect("recording Cargo shim is writable");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))
        .expect("recording Cargo shim can become executable");

    let log = fixture.path().join("cargo-calls.log");
    let path = env::join_paths(
        std::iter::once(shim_dir.clone())
            .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
    )
    .expect("shim PATH contains valid components");
    let mut command = Command::new("bash");
    command
        .arg(workspace_root().join("scripts").join(script))
        .args(paths)
        .current_dir(fixture.path())
        .env("PATH", path)
        .env("CARGO", &shim)
        .env("NEBULA_TEST_CARGO_LOG", &log)
        .env("NEBULA_TEST_PRE_COMMIT_PLAN", plan)
        .env("NEBULA_TEST_FIXTURE_MANIFEST", FIRST_MANIFEST)
        .env(
            "NEBULA_TEST_FAILURE",
            match failure {
                Failure::None | Failure::Extraction(_) => "none",
                Failure::Planner => "planner",
                Failure::Contract => "contract",
                Failure::Formatting => "formatting",
            },
        );
    if let Failure::Extraction(index) = failure {
        install_failing_jq(&shim_dir, &mut command, index);
    }
    let output = command
        .output()
        .expect("existing pre-commit script starts with the recording Cargo shim");
    let recorded = match fs::read_to_string(log) {
        Ok(recorded) => recorded,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => panic!("Cargo call log must be readable: {error}"),
    };
    let calls = recorded
        .split("__CALL__\n")
        .filter(|call| !call.is_empty())
        .map(|call| call.lines().map(str::to_owned).collect())
        .collect();
    HookRun { output, calls }
}

fn install_failing_jq(shim_dir: &Path, command: &mut Command, extraction: usize) {
    let lookup = Command::new("bash")
        .args(["-c", "command -v jq"])
        .output()
        .expect("locate real jq before installing the extraction-failure shim");
    assert!(lookup.status.success(), "hook tests require jq");
    let real_jq = String::from_utf8(lookup.stdout).expect("jq path is UTF-8");
    let shim = shim_dir.join("jq");
    fs::write(
        &shim,
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == -r ]]; then
  count=0
  [[ ! -f "$NEBULA_TEST_JQ_COUNT" ]] || count="$(<"$NEBULA_TEST_JQ_COUNT")"
  count=$((count + 1))
  printf '%s\n' "$count" > "$NEBULA_TEST_JQ_COUNT"
  if [[ "$count" == "$NEBULA_TEST_JQ_FAIL_EXTRACTION" ]]; then
    printf '%s\n' 'partial-extraction-must-not-run'
    printf '%s\n' 'injected jq extraction failure' >&2
    exit 42
  fi
fi
exec "$NEBULA_TEST_REAL_JQ" "$@"
"#,
    )
    .expect("write jq extraction-failure shim");
    fs::set_permissions(shim, fs::Permissions::from_mode(0o755))
        .expect("jq extraction-failure shim can become executable");
    command
        .env("NEBULA_TEST_REAL_JQ", real_jq.trim())
        .env("NEBULA_TEST_JQ_COUNT", shim_dir.join("jq-count"))
        .env("NEBULA_TEST_JQ_FAIL_EXTRACTION", extraction.to_string());
}

fn fixture_repo() -> TempDir {
    let fixture = tempfile::tempdir().expect("isolated hook fixture can be created");
    write_fixture(
        fixture.path(),
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/surface\"]\nresolver = \"3\"\n",
    );
    write_fixture(
        fixture.path(),
        "crates/surface/Cargo.toml",
        &format!("[package]\nname = \"{OWNER}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n"),
    );
    write_fixture(fixture.path(), OWNER_SOURCE, "pub fn author() {}\n");
    write_fixture(
        fixture.path(),
        "crates/surface/tests/external_contract.rs",
        "// Synthetic test target for ownership lookup; all Cargo calls are intercepted.\n",
    );
    for (manifest, name) in [
        (FIRST_MANIFEST, "first-isolated-consumer"),
        (SECOND_MANIFEST, "second-isolated-consumer"),
    ] {
        write_fixture(
            fixture.path(),
            manifest,
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 [package.metadata.nebula.fixture]\nowner = \"{OWNER}\"\ntest-target = \"{CONTRACT}\"\n\
                 [workspace]\n"
            ),
        );
        let directory = Path::new(manifest)
            .parent()
            .expect("fixture manifest has a parent");
        write_fixture(
            fixture.path(),
            directory.join("src/bin/positive.rs"),
            "fn main() {}\n",
        );
        write_fixture(
            fixture.path(),
            directory.join("src/bin/negative.rs"),
            "fn main( ) { let _ = MissingRemovedType; }\n",
        );
    }
    write_fixture(
        fixture.path(),
        STANDALONE_MANIFEST,
        "[package]\nname = \"independent-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n",
    );
    write_fixture(fixture.path(), STANDALONE_SOURCE, "pub fn probe() {}\n");
    fixture
}

fn write_fixture(root: &Path, relative: impl AsRef<Path>, source: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture file has a parent"))
        .expect("fixture parent directories are writable");
    fs::write(path, source).expect("fixture file is writable");
}

fn planner_call(paths: &[&str]) -> Vec<String> {
    let mut call = arguments(&["xtask", "pre-commit-plan", "--"]);
    call.extend(paths.iter().map(|path| (*path).to_owned()));
    call
}

fn arguments(arguments: &[&str]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives under tools in the workspace")
        .to_path_buf()
}

fn assert_success(run: &HookRun) {
    assert!(
        run.output.status.success(),
        "hook unexpectedly failed: {run:?}"
    );
}
