use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde::Deserialize;
use tempfile::TempDir;

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SemverPlan {
    schema_version: u8,
    scope: String,
    reason: String,
    package_count: usize,
    shard_count: usize,
    include: Vec<SemverShard>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SemverShard {
    shard: usize,
    packages: Vec<String>,
}

#[test]
fn selects_only_publishable_library_packages() {
    let repository = workspace_repository(&[
        package("crates/default", "fixture-default", "", TargetLayout::Lib),
        package(
            "crates/registry",
            "fixture-registry",
            "publish = [\"private\"]",
            TargetLayout::Lib,
        ),
        package(
            "crates/unpublished-array",
            "fixture-unpublished-array",
            "publish = []",
            TargetLayout::Lib,
        ),
        package(
            "crates/unpublished-bool",
            "fixture-unpublished-bool",
            "publish = false",
            TargetLayout::Lib,
        ),
        package("crates/bin", "fixture-bin", "", TargetLayout::Bin),
        package(
            "crates/proc-macro",
            "fixture-proc-macro",
            "",
            TargetLayout::ProcMacro,
        ),
        package("crates/mixed", "fixture-mixed", "", TargetLayout::Mixed),
    ]);
    let base = revision(repository.path());
    for package_path in [
        "crates/default",
        "crates/registry",
        "crates/unpublished-array",
        "crates/unpublished-bool",
        "crates/bin",
        "crates/proc-macro",
        "crates/mixed",
    ] {
        fs::write(
            repository.path().join(package_path).join("changed.txt"),
            "changed\n",
        )
        .expect("fixture change writes");
    }
    let head = commit_all(repository.path(), "change every target kind");

    let plan = semver_plan(repository.path(), &base, &head);

    assert_eq!(plan.schema_version, 1);
    assert_eq!(plan.scope, "diff");
    assert_eq!(plan.reason, "workspace-packages-changed");
    assert_eq!(plan.package_count, 3);
    assert_eq!(plan.shard_count, 2);
    assert_eq!(
        plan.include[0].packages,
        ["fixture-default", "fixture-registry"]
    );
    assert_eq!(plan.include[1].packages, ["fixture-mixed"]);
    assert_eq!(
        packages(&plan),
        vec!["fixture-default", "fixture-mixed", "fixture-registry"]
    );
}

#[test]
fn workflow_declares_unfiltered_events_and_dedicated_jobs() {
    let workflow = fs::read_to_string(workspace_root().join(".github/workflows/semver-checks.yml"))
        .expect("SemVer workflow is readable");

    assert!(
        yaml_mapping_value_at_path(&workflow, &["on", "pull_request"]).is_some(),
        "pull requests must trigger the required check"
    );
    assert!(
        yaml_mapping_value_at_path(&workflow, &["on", "pull_request", "paths"]).is_none(),
        "path filters can leave the required check pending"
    );
    assert_eq!(
        yaml_mapping_value_at_path(
            &workflow,
            &[
                "on",
                "workflow_dispatch",
                "inputs",
                "base_revision",
                "required"
            ],
        ),
        Some("true")
    );
    for job in ["select", "semver", "aggregate"] {
        assert!(
            yaml_mapping_value_at_path(&workflow, &["jobs", job]).is_some(),
            "workflow must declare `{job}`"
        );
    }
}

#[test]
fn moved_package_is_compared_by_stable_cargo_name() {
    let repository = workspace_repository(&[
        package("crates/old", "fixture-moved", "", TargetLayout::Lib),
        package(
            "crates/stationary",
            "fixture-stationary",
            "",
            TargetLayout::Lib,
        ),
    ]);
    let base = revision(repository.path());
    fs::rename(
        repository.path().join("crates/old"),
        repository.path().join("crates/new"),
    )
    .expect("package directory moves");
    replace_in_file(
        &repository.path().join("Cargo.toml"),
        "crates/old",
        "crates/new",
    );
    let head = commit_all(repository.path(), "move package without renaming it");

    let plan = semver_plan(repository.path(), &base, &head);

    assert_eq!(plan.scope, "full");
    assert_eq!(packages(&plan), vec!["fixture-moved", "fixture-stationary"]);
}

#[test]
fn new_and_renamed_packages_fail_without_partial_stdout() {
    let new_package_repository = workspace_repository(&[package(
        "crates/existing",
        "fixture-existing",
        "",
        TargetLayout::Lib,
    )]);
    let new_package_base = revision(new_package_repository.path());
    add_workspace_member(new_package_repository.path(), "crates/new");
    write_package(
        new_package_repository.path(),
        &package("crates/new", "fixture-new", "", TargetLayout::Lib),
    );
    cargo_generate_lockfile(new_package_repository.path());
    let new_package_head = commit_all(new_package_repository.path(), "add package");
    assert_missing_baseline_package(
        new_package_repository.path(),
        &new_package_base,
        &new_package_head,
        "fixture-new",
    );

    let renamed_repository = workspace_repository(&[package(
        "crates/item",
        "fixture-before",
        "",
        TargetLayout::Lib,
    )]);
    let renamed_base = revision(renamed_repository.path());
    replace_in_file(
        &renamed_repository.path().join("crates/item/Cargo.toml"),
        "fixture-before",
        "fixture-after",
    );
    cargo_generate_lockfile(renamed_repository.path());
    let renamed_head = commit_all(renamed_repository.path(), "rename package");
    assert_missing_baseline_package(
        renamed_repository.path(),
        &renamed_base,
        &renamed_head,
        "fixture-after",
    );
}

#[test]
fn deletion_selects_every_remaining_head_library() {
    let repository = workspace_repository(&[
        package("crates/alpha", "fixture-alpha", "", TargetLayout::Lib),
        package("crates/beta", "fixture-beta", "", TargetLayout::Lib),
        package("crates/deleted", "fixture-deleted", "", TargetLayout::Lib),
    ]);
    let base = revision(repository.path());
    fs::remove_dir_all(repository.path().join("crates/deleted"))
        .expect("deleted package directory removes");
    remove_workspace_member(repository.path(), "crates/deleted");
    cargo_generate_lockfile(repository.path());
    let head = commit_all(repository.path(), "delete package");

    let plan = semver_plan(repository.path(), &base, &head);

    assert_eq!(plan.scope, "full");
    assert_eq!(packages(&plan), vec!["fixture-alpha", "fixture-beta"]);
}

#[test]
fn merge_result_comparison_does_not_select_base_only_changes() {
    let repository = workspace_repository(&[package(
        "crates/alpha",
        "fixture-alpha",
        "",
        TargetLayout::Lib,
    )]);
    let root = revision(repository.path());
    git(repository.path(), &["switch", "-c", "feature", &root]);
    fs::write(
        repository.path().join("crates/alpha/changed.txt"),
        "feature\n",
    )
    .expect("feature change writes");
    commit_all(repository.path(), "change alpha on feature");

    git(repository.path(), &["switch", "main"]);
    add_workspace_member(repository.path(), "crates/beta");
    write_package(
        repository.path(),
        &package("crates/beta", "fixture-beta", "", TargetLayout::Lib),
    );
    cargo_generate_lockfile(repository.path());
    let base = commit_all(repository.path(), "add beta on base");
    git(
        repository.path(),
        &[
            "merge",
            "--no-ff",
            "-m",
            "synthetic merge result",
            "feature",
        ],
    );
    let merge_result = revision(repository.path());

    let plan = semver_plan(repository.path(), &base, &merge_result);

    assert_eq!(packages(&plan), vec!["fixture-alpha"]);
}

#[test]
fn invalid_revisions_fail_without_stdout() {
    let repository = workspace_repository(&[package(
        "crates/item",
        "fixture-item",
        "",
        TargetLayout::Lib,
    )]);
    let valid_revision = revision(repository.path());
    for (base, head) in [
        ("not-a-base-revision", valid_revision.as_str()),
        (valid_revision.as_str(), "not-a-head-revision"),
    ] {
        let output = run_semver_plan(repository.path(), base, head);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("git diff failed"));
    }
}

#[test]
fn output_is_deterministic_and_entry_limit_fails_closed() {
    let repository = workspace_repository(&[
        package("crates/zeta", "fixture-zeta", "", TargetLayout::Lib),
        package("crates/alpha", "fixture-alpha", "", TargetLayout::Lib),
    ]);
    let base = revision(repository.path());
    fs::write(repository.path().join("unknown.config"), "changed\n")
        .expect("unowned change writes");
    let head = commit_all(repository.path(), "force a conservative full selection");
    let first = run_semver_plan(repository.path(), &base, &head);
    let second = run_semver_plan(repository.path(), &base, &head);
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        packages(&parse_successful_plan(&first)),
        vec!["fixture-alpha", "fixture-zeta"]
    );

    let package_paths = (0..257)
        .map(|index| format!("crates/p{index:03}"))
        .collect::<Vec<_>>();
    let package_names = (0..257)
        .map(|index| format!("fixture-p{index:03}"))
        .collect::<Vec<_>>();
    let specifications = package_paths
        .iter()
        .zip(&package_names)
        .map(|(path, name)| package(path, name, "", TargetLayout::Lib))
        .collect::<Vec<_>>();
    let oversized_repository = workspace_repository(&specifications);
    let oversized_base = revision(oversized_repository.path());
    fs::write(
        oversized_repository.path().join("unknown.config"),
        "changed\n",
    )
    .expect("unowned change writes");
    let oversized_head = commit_all(
        oversized_repository.path(),
        "force an oversized full selection",
    );
    let oversized = run_semver_plan(
        oversized_repository.path(),
        &oversized_base,
        &oversized_head,
    );
    assert!(!oversized.status.success());
    assert!(oversized.stdout.is_empty());
    assert!(String::from_utf8_lossy(&oversized.stderr).contains("maximum is 256"));
}

#[test]
fn eighteen_packages_form_two_deterministic_round_robin_shards() {
    let package_paths = (0..18)
        .map(|index| format!("crates/p{index:02}"))
        .collect::<Vec<_>>();
    let package_names = (0..18)
        .map(|index| format!("fixture-p{index:02}"))
        .collect::<Vec<_>>();
    let specifications = package_paths
        .iter()
        .zip(&package_names)
        .map(|(path, name)| package(path, name, "", TargetLayout::Lib))
        .collect::<Vec<_>>();
    let repository = workspace_repository(&specifications);
    let base = revision(repository.path());
    fs::write(repository.path().join("unknown.config"), "changed\n")
        .expect("unowned change writes");
    let head = commit_all(repository.path(), "select every fixture package");

    let first = run_semver_plan(repository.path(), &base, &head);
    let second = run_semver_plan(repository.path(), &base, &head);
    assert_eq!(first.stdout, second.stdout);
    let plan = parse_successful_plan(&first);

    assert_eq!(plan.scope, "full");
    assert_eq!(plan.package_count, 18);
    assert_eq!(plan.shard_count, 2);
    assert_eq!(plan.include[0].shard, 0);
    assert_eq!(plan.include[1].shard, 1);
    assert_eq!(
        plan.include[0].packages,
        package_names.iter().step_by(2).cloned().collect::<Vec<_>>()
    );
    assert_eq!(
        plan.include[1].packages,
        package_names
            .iter()
            .skip(1)
            .step_by(2)
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(packages(&plan), package_names);
}

#[test]
fn workflow_pins_revisions_tools_matrix_and_fail_closed_aggregation() {
    let workflow = fs::read_to_string(workspace_root().join(".github/workflows/semver-checks.yml"))
        .expect("SemVer workflow is readable");

    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["permissions", "contents"]),
        Some("read")
    );
    assert!(yaml_mapping_value_at_path(&workflow, &["on", "push"]).is_none());
    assert!(yaml_mapping_value_at_path(&workflow, &["on", "merge_group"]).is_none());
    assert!(workflow.contains(
        "BASE_REVISION: ${{ github.event_name == 'pull_request' && github.event.pull_request.base.sha || inputs.base_revision }}"
    ));
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["env", "HEAD_REVISION"]),
        Some("${{ github.sha }}")
    );
    assert!(!workflow.contains("pull_request.head.sha"));
    assert!(workflow.contains("synthetic merge commit"));

    for job in ["select", "semver", "aggregate"] {
        let job_source = workflow_job(&workflow, job);
        assert_eq!(job_source.matches("actions/checkout@").count(), 1, "{job}");
        assert!(
            job_source.contains("ref: ${{ env.HEAD_REVISION }}"),
            "{job}"
        );
        assert!(job_source.contains("fetch-depth: 0"), "{job}");
        assert!(job_source.contains("persist-credentials: false"), "{job}");
        assert!(job_source.contains("git rev-parse HEAD"), "{job}");
        assert!(job_source.contains("${BASE_REVISION}^{commit}"), "{job}");
    }
    for action in workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("uses: "))
        .map(|use_clause| {
            use_clause
                .split_whitespace()
                .next()
                .expect("action use exists")
        })
    {
        let (_, revision) = action.rsplit_once('@').expect("action is versioned");
        assert_eq!(revision.len(), 40, "action is not SHA-pinned: {action}");
        assert!(
            revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "action is not SHA-pinned: {action}"
        );
    }

    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "select", "timeout-minutes"]),
        Some("5")
    );
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "semver", "timeout-minutes"]),
        Some("12")
    );
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "semver", "strategy", "fail-fast"],),
        Some("false")
    );
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "semver", "strategy", "max-parallel"],),
        Some("2")
    );
    assert!(workflow.contains("matrix: ${{ fromJSON(needs.select.outputs.plan) }}"));
    assert!(workflow.contains("command -v tar >/dev/null"));
    assert!(workflow.contains("cargo-semver-checks@0.50.0"));
    assert!(workflow.contains("cargo-semver-checks 0.50.0"));
    assert!(workflow.contains(
        "cargo semver-checks check-release --package \"$package\" --baseline-rev \"$BASE_REVISION\""
    ));
    assert!(!workflow.contains("--workspace"));
    assert!(!workflow.contains("rust-cache"));
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "aggregate", "name"]),
        Some("cargo-semver-checks")
    );
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "aggregate", "if"]),
        Some("always()")
    );
    assert_eq!(
        yaml_mapping_value_at_path(&workflow, &["jobs", "aggregate", "timeout-minutes"]),
        Some("2")
    );

    let matching_plan = r#"{"schema_version":1,"scope":"diff","reason":"changed","package_count":3,"shard_count":2,"include":[{"shard":0,"packages":["fixture-alpha","fixture-gamma"]},{"shard":1,"packages":["fixture-beta"]}]}"#;
    let matching_output = run_selector_script(&workflow, matching_plan);
    assert!(
        matching_output.status.success(),
        "selector rejected a matching plan: {}",
        String::from_utf8_lossy(&matching_output.stderr)
    );

    let mismatched_plan = r#"{"schema_version":1,"scope":"diff","reason":"changed","package_count":2,"shard_count":2,"include":[{"shard":0,"packages":["fixture-alpha"]},{"shard":1,"packages":[]}]}"#;
    let mismatched_output = run_selector_script(&workflow, mismatched_plan);
    assert!(!mismatched_output.status.success());

    let incorrectly_sharded_plan = r#"{"schema_version":1,"scope":"diff","reason":"changed","package_count":3,"shard_count":2,"include":[{"shard":0,"packages":["fixture-alpha","fixture-beta"]},{"shard":1,"packages":["fixture-gamma"]}]}"#;
    let incorrectly_sharded_output = run_selector_script(&workflow, incorrectly_sharded_plan);
    assert!(!incorrectly_sharded_output.status.success());

    let shard_calls = run_shard_script(&workflow, r#"["fixture-alpha","fixture-gamma"]"#);
    assert!(
        shard_calls.status.success(),
        "shard script failed: {}",
        String::from_utf8_lossy(&shard_calls.stderr)
    );
    assert_eq!(
        String::from_utf8(shard_calls.stdout).expect("captured cargo arguments are UTF-8"),
        "semver-checks check-release --package fixture-alpha --baseline-rev base\nsemver-checks check-release --package fixture-gamma --baseline-rev base\n"
    );

    let aggregator = workflow_step_script(
        workflow_job(&workflow, "aggregate"),
        "Require the matching planner and matrix outcomes",
    );
    for (planner, package_count, shard_count, matrix, should_succeed) in [
        ("success", "0", "0", "skipped", true),
        ("success", "1", "1", "success", true),
        ("success", "18", "2", "success", true),
        ("failure", "0", "0", "skipped", false),
        ("success", "", "0", "skipped", false),
        ("success", "01", "1", "success", false),
        ("success", "1", "0", "skipped", false),
        ("success", "0", "1", "success", false),
        ("success", "1", "2", "success", false),
        ("success", "2", "1", "success", false),
        ("success", "2", "2", "skipped", false),
        ("success", "0", "0", "success", false),
        ("success", "2", "2", "failure", false),
        ("success", "257", "2", "success", false),
        ("cancelled", "2", "2", "cancelled", false),
    ] {
        let status = Command::new("bash")
            .args(["-c", &aggregator])
            .env("PLANNER_RESULT", planner)
            .env("PACKAGE_COUNT", package_count)
            .env("SHARD_COUNT", shard_count)
            .env("MATRIX_RESULT", matrix)
            .status()
            .expect("aggregator script runs");
        assert_eq!(
            status.success(),
            should_succeed,
            "planner={planner}, package_count={package_count:?}, shard_count={shard_count:?}, matrix={matrix}"
        );
    }
}

fn run_shard_script(workflow: &str, packages_json: &str) -> Output {
    let worker = workflow_step_script(
        workflow_job(workflow, "semver"),
        "Check shard packages against baseline",
    );
    let script = format!("cargo() {{ printf '%s\\n' \"$*\"; }}\n{worker}");
    Command::new("bash")
        .args(["-c", &script])
        .env("PACKAGES_JSON", packages_json)
        .env("BASE_REVISION", "base")
        .output()
        .expect("shard script runs")
}

fn run_selector_script(workflow: &str, plan: &str) -> Output {
    let selector = workflow_step_script(
        workflow_job(workflow, "select"),
        "Build SemVer package plan",
    );
    let script = format!("cargo() {{ printf '%s\\n' \"$SEMVER_PLAN\"; }}\n{selector}");
    let github_output = tempfile::NamedTempFile::new().expect("GitHub output file creates");
    Command::new("bash")
        .args(["-c", &script])
        .env("SEMVER_PLAN", plan)
        .env("BASE_REVISION", "base")
        .env("HEAD_REVISION", "head")
        .env("GITHUB_OUTPUT", github_output.path())
        .output()
        .expect("selector script runs")
}

#[derive(Clone, Copy)]
enum TargetLayout {
    Lib,
    Bin,
    ProcMacro,
    Mixed,
}

struct PackageSpec<'a> {
    path: &'a str,
    name: &'a str,
    package_manifest: &'a str,
    target_layout: TargetLayout,
}

const fn package<'a>(
    path: &'a str,
    name: &'a str,
    package_manifest: &'a str,
    target_layout: TargetLayout,
) -> PackageSpec<'a> {
    PackageSpec {
        path,
        name,
        package_manifest,
        target_layout,
    }
}

fn workspace_repository(packages: &[PackageSpec<'_>]) -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository is available");
    let members = packages
        .iter()
        .map(|package| format!("  \"{}\",", package.path))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        repository.path().join("Cargo.toml"),
        format!(
            "[workspace]\nmembers = [\n{members}\n]\nresolver = \"3\"\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n"
        ),
    )
    .expect("workspace manifest writes");
    for package in packages {
        write_package(repository.path(), package);
    }
    cargo_generate_lockfile(repository.path());
    git(repository.path(), &["init", "-q", "-b", "main"]);
    git(
        repository.path(),
        &["config", "user.email", "semver-plan@example.invalid"],
    );
    git(
        repository.path(),
        &["config", "user.name", "SemVer Plan Test"],
    );
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "-qm", "fixture baseline"]);
    repository
}

fn write_package(repository: &Path, package: &PackageSpec<'_>) {
    let package_directory = repository.join(package.path);
    fs::create_dir_all(package_directory.join("src")).expect("package directory creates");
    let target_manifest = match package.target_layout {
        TargetLayout::ProcMacro => "\n[lib]\nproc-macro = true\n",
        TargetLayout::Lib | TargetLayout::Bin | TargetLayout::Mixed => "",
    };
    fs::write(
        package_directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{}\"\nversion.workspace = true\nedition.workspace = true\n{}{}\n",
            package.name, package.package_manifest, target_manifest
        ),
    )
    .expect("package manifest writes");
    if matches!(
        package.target_layout,
        TargetLayout::Lib | TargetLayout::ProcMacro | TargetLayout::Mixed
    ) {
        fs::write(
            package_directory.join("src/lib.rs"),
            "pub fn fixture() {}\n",
        )
        .expect("library source writes");
    }
    if matches!(
        package.target_layout,
        TargetLayout::Bin | TargetLayout::Mixed
    ) {
        fs::write(package_directory.join("src/main.rs"), "fn main() {}\n")
            .expect("binary source writes");
    }
}

fn add_workspace_member(repository: &Path, package_path: &str) {
    let manifest_path = repository.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path).expect("workspace manifest reads");
    let updated = source.replacen(
        "]\nresolver = \"3\"",
        &format!("  \"{package_path}\",\n]\nresolver = \"3\""),
        1,
    );
    assert_ne!(updated, source, "workspace members block is found");
    fs::write(manifest_path, updated).expect("workspace manifest writes");
}

fn remove_workspace_member(repository: &Path, package_path: &str) {
    let manifest_path = repository.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path).expect("workspace manifest reads");
    let member = format!("  \"{package_path}\",\n");
    let updated = source.replacen(&member, "", 1);
    assert_ne!(updated, source, "workspace member is found");
    fs::write(manifest_path, updated).expect("workspace manifest writes");
}

fn replace_in_file(path: &Path, before: &str, after: &str) {
    let source = fs::read_to_string(path).expect("fixture file reads");
    let updated = source.replacen(before, after, 1);
    assert_ne!(updated, source, "replacement source is found");
    fs::write(path, updated).expect("fixture file writes");
}

fn assert_missing_baseline_package(repository: &Path, base: &str, head: &str, package: &str) {
    let output = run_semver_plan(repository, base, head);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(&format!("selected head package `{package}` is absent"))
    );
}

fn semver_plan(repository: &Path, base: &str, head: &str) -> SemverPlan {
    parse_successful_plan(&run_semver_plan(repository, base, head))
}

fn run_semver_plan(repository: &Path, base: &str, head: &str) -> Output {
    xtask(
        repository,
        &[
            "ci-plan",
            "semver",
            "--base",
            base,
            "--head",
            head,
            "--comparison",
            "direct",
        ],
    )
}

fn parse_successful_plan(output: &Output) -> SemverPlan {
    assert!(
        output.status.success(),
        "SemVer planner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is a SemVer plan")
}

fn packages(plan: &SemverPlan) -> Vec<String> {
    let mut packages = plan
        .include
        .iter()
        .flat_map(|shard| shard.packages.iter().cloned())
        .collect::<Vec<_>>();
    packages.sort();
    packages
}

fn cargo_generate_lockfile(repository: &Path) {
    let output = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(repository)
        .output()
        .expect("cargo generate-lockfile runs");
    assert!(
        output.status.success(),
        "cargo generate-lockfile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn revision(repository: &Path) -> String {
    git_output(repository, &["rev-parse", "HEAD"])
        .trim()
        .to_owned()
}

fn commit_all(repository: &Path, message: &str) -> String {
    git(repository, &["add", "."]);
    git(repository, &["commit", "-qm", message]);
    revision(repository)
}

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git runs");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("git output is UTF-8")
}

fn xtask(repository: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nebula-xtask"))
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("xtask binary runs")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask is nested beneath the workspace root")
        .to_path_buf()
}

fn yaml_mapping_value_at_path<'a>(document: &'a str, path: &[&str]) -> Option<&'a str> {
    let mut parents: Vec<(usize, &str)> = Vec::new();

    for line in document.lines() {
        let trimmed = line.trim_start_matches(' ');
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let Some((raw_key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = raw_key.trim().trim_matches(['\'', '"']);
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            continue;
        }
        while parents
            .last()
            .is_some_and(|(parent_indent, _)| *parent_indent >= indent)
        {
            parents.pop();
        }
        parents.push((indent, key));
        if parents
            .iter()
            .map(|(_, parent_key)| *parent_key)
            .eq(path.iter().copied())
        {
            let value = raw_value.trim();
            return Some(if value.starts_with('#') { "" } else { value });
        }
    }
    None
}

fn workflow_job<'a>(workflow: &'a str, job: &str) -> &'a str {
    let marker = format!("  {job}:\n");
    let start = workflow.find(&marker).expect("workflow job exists");
    let remainder = &workflow[start + marker.len()..];
    let end = remainder
        .match_indices("\n  ")
        .find_map(|(index, _)| {
            let candidate = &remainder[index + 3..];
            candidate
                .split_once(':')
                .is_some_and(|(key, _)| !key.starts_with(' ') && !key.contains(char::is_whitespace))
                .then_some(index)
        })
        .unwrap_or(remainder.len());
    &remainder[..end]
}

fn workflow_step_script(job: &str, step_name: &str) -> String {
    let marker = format!("- name: {step_name}");
    let step = &job[job.find(&marker).expect("workflow step exists")..];
    let run = &step[step.find("run: |").expect("step has a block script") + "run: |".len()..];
    run.lines()
        .skip(1)
        .take_while(|line| line.starts_with("        ") || line.trim().is_empty())
        .map(|line| line.strip_prefix("        ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}
