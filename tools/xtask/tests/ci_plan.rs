use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::Deserialize;
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Deserialize)]
struct Plan {
    schema_version: u8,
    scope: String,
    reason: String,
    count: usize,
    include: Vec<PlanEntry>,
}

#[derive(Debug, Deserialize)]
struct PlanEntry {
    package: String,
    test_features: Vec<String>,
}

#[derive(Clone, Copy)]
struct PackageSpec<'a> {
    path: &'a str,
    name: &'a str,
    extra_manifest: &'a str,
}

#[test]
fn nested_package_change_selects_owner_and_all_reverse_dependents() {
    let fixture = fixture_repo();
    let base = git_output(fixture.path(), &["rev-parse", "HEAD"]);

    fs::write(
        fixture.path().join("crates/parent/macros/src/lib.rs"),
        "pub fn changed() {}\n",
    )
    .expect("fixture source is writable");
    git(fixture.path(), &["add", "."]);
    git(fixture.path(), &["commit", "-qm", "change nested package"]);
    let head = git_output(fixture.path(), &["rev-parse", "HEAD"]);

    let output = xtask(
        fixture.path(),
        &[
            "ci-plan",
            "diff",
            "--base",
            base.trim(),
            "--head",
            head.trim(),
            "--comparison",
            "direct",
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    let packages = plan["include"]
        .as_array()
        .expect("include is an array")
        .iter()
        .map(|entry| entry["package"].as_str().expect("package is a string"))
        .collect::<Vec<_>>();

    assert_eq!(
        packages,
        vec!["fixture-app", "fixture-parent", "fixture-parent-macros"]
    );
    assert!(!packages.contains(&"fixture-examples"));
}

#[test]
fn full_fixture_is_sorted_and_uses_only_declared_test_features() {
    let repo = workspace_repo(&[
        PackageSpec {
            path: "crates/zeta",
            name: "fixture-zeta",
            extra_manifest: "",
        },
        PackageSpec {
            path: "crates/alpha",
            name: "fixture-alpha",
            extra_manifest: r#"
[features]
fast = []
slow = []

[package.metadata.nebula.ci]
test-features = ["slow", "fast", "slow"]
"#,
        },
    ]);

    let output = xtask(repo.path(), &["ci-plan", "full"]);
    let plan = successful_plan(&output);

    assert_eq!(plan.schema_version, 1);
    assert_eq!(plan.scope, "full");
    assert_eq!(plan.reason, "full-request");
    assert_eq!(plan.count, 2);
    assert_eq!(packages(&plan), vec!["fixture-alpha", "fixture-zeta"]);
    assert_eq!(plan.include[0].test_features, vec!["fast", "slow"]);
}

#[test]
fn live_workspace_full_equals_cargo_metadata_and_has_no_retired_telemetry() {
    let root = workspace_root();
    let output = xtask(&root, &["ci-plan", "full"]);
    let plan = successful_plan(&output);
    let metadata_output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--all-features"])
        .current_dir(&root)
        .output()
        .expect("cargo metadata runs");
    assert!(metadata_output.status.success());
    let metadata: Value =
        serde_json::from_slice(&metadata_output.stdout).expect("metadata is JSON");
    let members = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members is an array");
    let id_to_name = metadata["packages"]
        .as_array()
        .expect("packages is an array")
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package ID").to_owned(),
                package["name"].as_str().expect("package name").to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut expected = members
        .iter()
        .map(|id| {
            id_to_name
                .get(id.as_str().expect("member ID"))
                .expect("member package exists")
                .as_str()
        })
        .collect::<Vec<_>>();
    expected.sort_unstable();

    let actual = packages(&plan);
    assert_eq!(plan.count, expected.len());
    assert_eq!(actual, expected);
    assert!(!actual.contains(&"nebula-telemetry"));
    assert!(actual.contains(&"nebula-xtask"));
    assert_entry_features(&plan, "nebula-engine", &["rotation"]);
    assert_entry_features(&plan, "nebula-storage", &["credential-in-memory"]);
}

#[test]
fn nested_owner_wins_and_app_and_examples_are_independent() {
    for (path, expected) in [
        ("apps/demo/src/lib.rs", vec!["fixture-app"]),
        ("examples/src/lib.rs", vec!["fixture-examples"]),
    ] {
        let repo = fixture_repo();
        let base = revision(repo.path());
        fs::write(repo.path().join(path), "pub fn changed() {}\n")
            .expect("fixture source is writable");
        let head = commit_all(repo.path(), "change independent package");
        let plan = diff_plan(repo.path(), &base, &head, "direct");
        assert_eq!(packages(&plan), expected);
    }
}

#[test]
fn diamond_reverse_dependencies_are_deduplicated_and_sorted() {
    let repo = workspace_repo(&[
        package("crates/base", "fixture-base", ""),
        package(
            "crates/left",
            "fixture-left",
            dependency("fixture-base", "../base"),
        ),
        package(
            "crates/right",
            "fixture-right",
            dependency("fixture-base", "../base"),
        ),
        package(
            "apps/demo",
            "fixture-app",
            r#"
[dependencies]
fixture-left = { path = "../../crates/left" }
fixture-right = { path = "../../crates/right" }
"#,
        ),
        package("crates/sibling", "fixture-sibling", ""),
    ]);
    let base = revision(repo.path());
    change_source(repo.path(), "crates/base", "diamond");
    let head = commit_all(repo.path(), "change diamond base");

    let plan = diff_plan(repo.path(), &base, &head, "direct");
    assert_eq!(
        packages(&plan),
        vec![
            "fixture-app",
            "fixture-base",
            "fixture-left",
            "fixture-right"
        ]
    );
}

#[test]
fn normal_dev_build_optional_and_target_dependencies_are_reverse_edges() {
    let dependency_names = ["normal", "dev", "build", "optional", "target"];
    let mut specs = dependency_names
        .iter()
        .map(|name| {
            package(
                Box::leak(format!("crates/{name}").into_boxed_str()),
                Box::leak(format!("fixture-{name}").into_boxed_str()),
                "",
            )
        })
        .collect::<Vec<_>>();
    specs.push(package(
        "crates/consumer",
        "fixture-consumer",
        r#"
[dependencies]
fixture-normal = { path = "../normal" }
fixture-optional = { path = "../optional", optional = true }

[dev-dependencies]
fixture-dev = { path = "../dev" }

[build-dependencies]
fixture-build = { path = "../build" }

[target.'cfg(target_os = "none")'.dependencies]
fixture-target = { path = "../target" }

[features]
with-optional = ["dep:fixture-optional"]
"#,
    ));
    let repo = workspace_repo(&specs);
    let base = revision(repo.path());

    for name in dependency_names {
        git(repo.path(), &["checkout", "-q", "--detach", &base]);
        change_source(repo.path(), &format!("crates/{name}"), name);
        let head = commit_all(repo.path(), &format!("change {name} dependency"));
        let plan = diff_plan(repo.path(), &base, &head, "direct");
        let mut expected = vec!["fixture-consumer".to_owned(), format!("fixture-{name}")];
        expected.sort();
        assert_eq!(
            packages(&plan),
            expected,
            "dependency kind for {name} was not included"
        );
    }
}

#[test]
fn rename_selects_both_sides_and_deletion_forces_full() {
    let repo = fixture_repo();
    fs::write(
        repo.path().join("crates/parent/src/old.rs"),
        "pub fn old() {}\n",
    )
    .expect("fixture file writes");
    let base = commit_all(repo.path(), "add rename source");
    fs::rename(
        repo.path().join("crates/parent/src/old.rs"),
        repo.path().join("crates/parent/src/renamed.rs"),
    )
    .expect("fixture file renames");
    let renamed = commit_all(repo.path(), "rename parent source");
    let rename_plan = diff_plan(repo.path(), &base, &renamed, "direct");
    assert_eq!(
        packages(&rename_plan),
        vec!["fixture-app", "fixture-parent"]
    );

    git(repo.path(), &["checkout", "-q", "--detach", &base]);
    fs::write(repo.path().join("obsolete.txt"), "obsolete\n").expect("root file writes");
    let with_obsolete = commit_all(repo.path(), "add obsolete file");
    fs::remove_file(repo.path().join("obsolete.txt")).expect("root file deletes");
    let deleted = commit_all(repo.path(), "delete obsolete file");
    let delete_plan = diff_plan(repo.path(), &with_obsolete, &deleted, "direct");
    assert_eq!(delete_plan.scope, "full");
    assert!(delete_plan.reason.starts_with("deleted-path:"));
    assert_eq!(delete_plan.count, 4);
}

#[test]
fn unchanged_cross_package_copy_selects_both_owners_and_reverse_dependents() {
    let repo = fixture_repo();
    let source = repo.path().join("crates/parent/src/lib.rs");
    fs::write(
        &source,
        r#"pub fn parent_alpha() -> &'static str { "alpha" }
pub fn parent_beta() -> &'static str { "beta" }
pub fn parent_gamma() -> &'static str { "gamma" }
pub fn parent_delta() -> &'static str { "delta" }
"#,
    )
    .expect("copy source writes");
    let base = commit_all(repo.path(), "add stable copy source");

    fs::copy(&source, repo.path().join("examples/src/copied_parent.rs"))
        .expect("unchanged source copies across packages");
    let head = commit_all(repo.path(), "copy unchanged source across packages");

    let plan = diff_plan(repo.path(), &base, &head, "direct");

    assert_eq!(
        packages(&plan),
        vec!["fixture-app", "fixture-examples", "fixture-parent"]
    );
    assert!(!packages(&plan).contains(&"fixture-parent-macros"));
}

#[test]
fn merge_base_and_direct_comparisons_have_distinct_semantics() {
    let repo = workspace_repo(&[
        package("crates/alpha", "fixture-alpha", ""),
        package("crates/beta", "fixture-beta", ""),
    ]);
    let root = revision(repo.path());

    git(repo.path(), &["checkout", "-qb", "feature", &root]);
    change_source(repo.path(), "crates/alpha", "feature");
    let feature = commit_all(repo.path(), "feature changes alpha");

    git(repo.path(), &["checkout", "-qb", "base-tip", &root]);
    change_source(repo.path(), "crates/beta", "base");
    let base_tip = commit_all(repo.path(), "base changes beta");

    let merge_base = diff_plan(repo.path(), &base_tip, &feature, "merge-base");
    let direct = diff_plan(repo.path(), &base_tip, &feature, "direct");
    assert_eq!(packages(&merge_base), vec!["fixture-alpha"]);
    assert_eq!(packages(&direct), vec!["fixture-alpha", "fixture-beta"]);
}

#[test]
fn bootstrap_unknown_and_excluded_fuzz_paths_force_full() {
    for (path, expected_reason) in [
        (".github/workflows/test-matrix.yml", "bootstrap-change:"),
        ("mystery.config", "unowned-path:"),
        (
            "crates/parent/fuzz/fuzz_targets/input.rs",
            "excluded-fuzz-change:",
        ),
    ] {
        let repo = fixture_repo();
        let base = revision(repo.path());
        let target = repo.path().join(path);
        fs::create_dir_all(target.parent().expect("test path has a parent"))
            .expect("test path parent is creatable");
        fs::write(&target, "changed\n").expect("test path is writable");
        let head = commit_all(repo.path(), "add full-scope path");
        let plan = diff_plan(repo.path(), &base, &head, "direct");
        assert_eq!(plan.scope, "full", "path: {path}");
        assert!(plan.reason.starts_with(expected_reason), "path: {path}");
        assert_eq!(plan.count, 4);
    }
}

#[test]
fn docs_only_is_empty_but_package_changes_union_and_full_overrides_win() {
    let repo = fixture_repo();
    let base = revision(repo.path());
    fs::write(repo.path().join("README.md"), "documentation\n").expect("README writes");
    let docs_head = commit_all(repo.path(), "docs only");
    let docs_plan = diff_plan(repo.path(), &base, &docs_head, "direct");
    assert_eq!(docs_plan.scope, "diff");
    assert_eq!(docs_plan.reason, "docs-assets-only");
    assert_eq!(docs_plan.count, 0);

    git(repo.path(), &["checkout", "-q", "--detach", &base]);
    fs::write(repo.path().join("README.md"), "documentation\n").expect("README writes");
    change_source(repo.path(), "crates/parent", "package-and-docs");
    let union_head = commit_all(repo.path(), "package and docs");
    let union_plan = diff_plan(repo.path(), &base, &union_head, "direct");
    assert_eq!(packages(&union_plan), vec!["fixture-app", "fixture-parent"]);

    git(repo.path(), &["checkout", "-q", "--detach", &base]);
    change_source(repo.path(), "crates/parent", "package-and-unknown");
    fs::write(repo.path().join("unknown.bin"), "unknown\n").expect("unknown file writes");
    let full_head = commit_all(repo.path(), "package and unknown");
    let full_plan = diff_plan(repo.path(), &base, &full_head, "direct");
    assert_eq!(full_plan.scope, "full");
    assert!(full_plan.reason.starts_with("unowned-path:"));
}

#[test]
fn package_local_documentation_selects_owner_and_reverse_dependents() {
    let repo = fixture_repo();
    let base = revision(repo.path());
    fs::write(
        repo.path().join("crates/parent/README.md"),
        "package documentation\n",
    )
    .expect("package README writes");
    let head = commit_all(repo.path(), "package documentation");

    let plan = diff_plan(repo.path(), &base, &head, "direct");

    assert_eq!(plan.scope, "diff");
    assert_eq!(plan.reason, "workspace-packages-changed");
    assert_eq!(packages(&plan), vec!["fixture-app", "fixture-parent"]);
}

#[test]
fn missing_sha_selects_full_but_invalid_nonempty_ref_is_an_error() {
    let repo = fixture_repo();
    let missing = xtask(repo.path(), &["ci-plan", "diff"]);
    let missing_plan = successful_plan(&missing);
    assert_eq!(missing_plan.scope, "full");
    assert_eq!(missing_plan.reason, "missing-diff-sha");

    let invalid = xtask(
        repo.path(),
        &[
            "ci-plan",
            "diff",
            "--base",
            "definitely-not-a-ref",
            "--head",
            "HEAD",
            "--comparison",
            "direct",
        ],
    );
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("git diff failed"));
}

#[test]
fn malformed_or_unknown_feature_metadata_fails_without_partial_stdout() {
    for extra_manifest in [
        r#"
[package.metadata.nebula.ci]
test-features = "fast"
"#,
        r#"
[features]
fast = []

[package.metadata.nebula.ci]
test-features = ["unknown"]
"#,
        r#"
[package.metadata.nebula]
ci = "not-an-object"
"#,
        r#"
[features]
fast = []

[package.metadata.nebula.ci]
test-feature = ["fast"]
"#,
        r#"
[features]
fast = []

[package.metadata.nebula.ci]
test-features = ["fast"]
unexpected-policy = true
"#,
    ] {
        let repo = workspace_repo(&[package("crates/item", "fixture-item", extra_manifest)]);
        let output = xtask(repo.path(), &["ci-plan", "full"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn unrelated_package_metadata_outside_nebula_ci_is_ignored() {
    let repo = workspace_repo(&[package(
        "crates/item",
        "fixture-item",
        r#"
[package.metadata.nebula.ci]

[package.metadata.integration]
display-name = "Fixture"
arbitrary-policy = true
"#,
    )]);

    let plan = successful_plan(&xtask(repo.path(), &["ci-plan", "full"]));

    assert_eq!(packages(&plan), vec!["fixture-item"]);
    assert!(plan.include[0].test_features.is_empty());
}

#[test]
fn entry_limit_accepts_256_and_rejects_257_without_partial_stdout() {
    let specs = (0..257)
        .map(|index| {
            package(
                Box::leak(format!("crates/p{index:03}").into_boxed_str()),
                Box::leak(format!("fixture-p{index:03}").into_boxed_str()),
                "",
            )
        })
        .collect::<Vec<_>>();

    let at_limit = workspace_repo(&specs[..256]);
    let accepted = xtask(at_limit.path(), &["ci-plan", "full"]);
    assert_eq!(successful_plan(&accepted).count, 256);

    let over_limit = workspace_repo(&specs);
    let rejected = xtask(over_limit.path(), &["ci-plan", "full"]);
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("maximum is 256"));
}

#[test]
fn outer_cli_help_is_successful_and_writes_to_stdout() {
    let output = xtask(&workspace_root(), &["--help"]);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("Nebula repository automation"));
    assert!(stdout.contains("Usage: nebula-xtask <COMMAND>"));
}

#[test]
fn outer_cli_version_is_successful_and_writes_to_stdout() {
    let output = xtask(&workspace_root(), &["--version"]);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version is UTF-8"),
        format!("nebula-xtask {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn outer_cli_invalid_usage_preserves_clap_exit_code_and_stderr() {
    let output = xtask(&workspace_root(), &["not-a-command"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("usage error is UTF-8");
    assert!(stderr.contains("unrecognized subcommand 'not-a-command'"));
    assert!(stderr.contains("Usage: nebula-xtask <COMMAND>"));
}

#[test]
fn output_is_stable_and_xtask_has_no_nebula_product_dependencies() {
    let root = workspace_root();
    let first = xtask(&root, &["ci-plan", "full"]);
    let second = xtask(&root, &["ci-plan", "full"]);
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);

    let metadata = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(&root)
        .output()
        .expect("cargo metadata runs");
    assert!(metadata.status.success());
    let value: Value = serde_json::from_slice(&metadata.stdout).expect("metadata is JSON");
    let xtask_package = value["packages"]
        .as_array()
        .expect("packages is an array")
        .iter()
        .find(|package| package["name"] == "nebula-xtask")
        .expect("xtask package is present");
    let product_dependencies = xtask_package["dependencies"]
        .as_array()
        .expect("dependencies is an array")
        .iter()
        .filter_map(|dependency| dependency["name"].as_str())
        .filter(|name| name.starts_with("nebula-") && *name != "nebula-xtask")
        .collect::<Vec<_>>();
    assert!(product_dependencies.is_empty(), "{product_dependencies:?}");
}

#[test]
fn planner_requires_an_existing_current_lockfile_without_mutation() {
    let repo = fixture_repo();
    let lockfile = repo.path().join("Cargo.lock");
    fs::remove_file(&lockfile).expect("fixture lockfile removes");

    let output = xtask(repo.path(), &["ci-plan", "full"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!lockfile.exists());
}

#[test]
fn cargo_alias_runs_xtask_with_a_locked_dependency_graph() {
    let config = fs::read_to_string(workspace_root().join(".cargo/config.toml"))
        .expect("Cargo config is readable");

    assert!(config.contains("xtask = \"run --locked --quiet -p nebula-xtask --\""));
}

#[test]
fn workflow_push_always_reaches_the_metadata_selector() {
    let workflow = fs::read_to_string(workspace_root().join(".github/workflows/test-matrix.yml"))
        .expect("test-matrix workflow is readable");

    let push_value = yaml_mapping_value_at_path(&workflow, &["on", "push"])
        .expect("workflow declares on.push as a block mapping");
    assert!(
        push_value.is_empty(),
        "on.push must remain a block mapping so its filter policy is inspectable"
    );
    assert!(
        yaml_mapping_value_at_path(&workflow, &["on", "push", "paths-ignore"]).is_none(),
        "on.push.paths-ignore can bypass the sole Cargo-metadata selector"
    );
    assert!(
        yaml_mapping_value_at_path(&workflow, &["on", "push", "paths"]).is_none(),
        "on.push.paths can bypass the sole Cargo-metadata selector"
    );
}

#[cfg(unix)]
#[test]
fn pre_push_resolves_a_local_tracking_branch_to_its_commit() {
    let repo = pre_push_repo();
    let main_sha = revision(repo.path());
    git(repo.path(), &["switch", "-qc", "feature"]);
    git(repo.path(), &["config", "branch.feature.remote", "."]);
    git(
        repo.path(),
        &["config", "branch.feature.merge", "refs/heads/main"],
    );
    fs::write(repo.path().join("feature.txt"), "feature\n").expect("feature file writes");
    commit_all(repo.path(), "feature change");

    let (output, calls) = run_pre_push(repo.path(), &one_package_plan());

    assert!(
        output.status.success(),
        "pre-push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        calls,
        vec![
            vec![
                "xtask".to_owned(),
                "ci-plan".to_owned(),
                "diff".to_owned(),
                "--base".to_owned(),
                main_sha,
                "--head".to_owned(),
                "HEAD".to_owned(),
                "--comparison".to_owned(),
                "merge-base".to_owned(),
            ],
            vec![
                "nextest".to_owned(),
                "run".to_owned(),
                "-p".to_owned(),
                "fixture-package".to_owned(),
                "--features".to_owned(),
                "fast,slow".to_owned(),
                "--profile".to_owned(),
                "agent".to_owned(),
                "--no-tests=pass".to_owned(),
            ],
            vec![
                "check".to_owned(),
                "-p".to_owned(),
                "fixture-package".to_owned(),
                "--all-features".to_owned(),
                "--all-targets".to_owned(),
                "--quiet".to_owned(),
            ],
            vec![
                "doc".to_owned(),
                "-p".to_owned(),
                "fixture-package".to_owned(),
                "--no-deps".to_owned(),
                "--quiet".to_owned(),
            ],
        ]
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("main...HEAD"));
    assert!(
        !stdout.contains("./main"),
        "Git's local-upstream syntax must not be reconstructed as ./main"
    );
}

#[cfg(unix)]
#[test]
fn pre_push_uses_the_verified_origin_main_commit_as_fallback() {
    let repo = pre_push_repo();
    let main_sha = revision(repo.path());
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/main", &main_sha],
    );
    git(repo.path(), &["switch", "-qc", "feature"]);

    let (output, calls) = run_pre_push(repo.path(), &empty_plan());

    assert!(
        output.status.success(),
        "pre-push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        calls,
        vec![vec![
            "xtask".to_owned(),
            "ci-plan".to_owned(),
            "diff".to_owned(),
            "--base".to_owned(),
            main_sha,
            "--head".to_owned(),
            "HEAD".to_owned(),
            "--comparison".to_owned(),
            "merge-base".to_owned(),
        ]]
    );
}

#[cfg(unix)]
#[test]
fn pre_push_without_any_resolvable_base_uses_the_full_plan() {
    let repo = pre_push_repo();
    git(repo.path(), &["switch", "-qc", "feature"]);

    let (output, calls) = run_pre_push(repo.path(), &empty_plan());

    assert!(
        output.status.success(),
        "pre-push failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        calls,
        vec![vec![
            "xtask".to_owned(),
            "ci-plan".to_owned(),
            "full".to_owned(),
        ]]
    );
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

#[cfg(unix)]
fn pre_push_repo() -> TempDir {
    let temp = tempfile::tempdir().expect("temporary directory is available");
    git(temp.path(), &["init", "-q", "-b", "main"]);
    git(
        temp.path(),
        &["config", "user.email", "pre-push@example.invalid"],
    );
    git(temp.path(), &["config", "user.name", "Pre Push Test"]);
    fs::write(temp.path().join("README"), "baseline\n").expect("baseline file writes");
    commit_all(temp.path(), "baseline");
    temp
}

#[cfg(unix)]
fn run_pre_push(repo: &Path, plan: &str) -> (Output, Vec<Vec<String>>) {
    let shim_dir = repo.join("command-shims");
    fs::create_dir(&shim_dir).expect("shim directory creates");
    let cargo_shim = shim_dir.join("cargo");
    fs::write(
        &cargo_shim,
        r#"#!/usr/bin/env bash
set -euo pipefail
{
  printf '%s\n' '__CALL__'
  printf '%s\n' "$@"
} >> "$NEBULA_TEST_CARGO_LOG"
if [[ "${1:-}" == "xtask" ]]; then
  printf '%s\n' "$NEBULA_TEST_CI_PLAN"
fi
"#,
    )
    .expect("cargo shim writes");
    let mut permissions = fs::metadata(&cargo_shim)
        .expect("cargo shim metadata is available")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cargo_shim, permissions).expect("cargo shim becomes executable");

    let cargo_log = repo.join("cargo-calls.log");
    let path = env::join_paths(
        std::iter::once(shim_dir).chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
    )
    .expect("shim PATH joins");
    let output = Command::new("bash")
        .arg(workspace_root().join("scripts/pre-push-crate-diff.sh"))
        .current_dir(repo)
        .env("PATH", path)
        .env("NEBULA_TEST_CARGO_LOG", &cargo_log)
        .env("NEBULA_TEST_CI_PLAN", plan)
        .output()
        .expect("pre-push script runs");
    let log = fs::read_to_string(cargo_log).expect("cargo shim log is readable");
    let calls = log
        .split("__CALL__\n")
        .filter(|call| !call.is_empty())
        .map(|call| call.lines().map(str::to_owned).collect())
        .collect();

    (output, calls)
}

fn one_package_plan() -> String {
    serde_json::json!({
        "schema_version": 1,
        "scope": "diff",
        "reason": "workspace-packages-changed",
        "count": 1,
        "include": [{
            "package": "fixture-package",
            "test_features": ["fast", "slow"]
        }]
    })
    .to_string()
}

fn empty_plan() -> String {
    serde_json::json!({
        "schema_version": 1,
        "scope": "diff",
        "reason": "unowned-docs-assets-only",
        "count": 0,
        "include": []
    })
    .to_string()
}

fn fixture_repo() -> TempDir {
    let temp = tempfile::tempdir().expect("temporary directory is available");
    copy_tree(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nested"),
        temp.path(),
    );
    cargo_generate_lockfile(temp.path());
    git(temp.path(), &["init", "-q", "-b", "main"]);
    git(
        temp.path(),
        &["config", "user.email", "ci-plan@example.invalid"],
    );
    git(temp.path(), &["config", "user.name", "CI Plan Test"]);
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "fixture baseline"]);
    temp
}

fn workspace_repo(specs: &[PackageSpec<'_>]) -> TempDir {
    let temp = tempfile::tempdir().expect("temporary directory is available");
    let members = specs
        .iter()
        .map(|spec| format!("  \"{}\",", spec.path))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[workspace]\nmembers = [\n{members}\n]\nresolver = \"3\"\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n"
        ),
    )
    .expect("workspace manifest writes");
    for spec in specs {
        let package_dir = temp.path().join(spec.path);
        fs::create_dir_all(package_dir.join("src")).expect("package directory creates");
        fs::write(
            package_dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion.workspace = true\nedition.workspace = true\n{}\n",
                spec.name, spec.extra_manifest
            ),
        )
        .expect("package manifest writes");
        fs::write(package_dir.join("src/lib.rs"), "pub fn fixture() {}\n")
            .expect("package source writes");
    }
    cargo_generate_lockfile(temp.path());
    git(temp.path(), &["init", "-q", "-b", "main"]);
    git(
        temp.path(),
        &["config", "user.email", "ci-plan@example.invalid"],
    );
    git(temp.path(), &["config", "user.name", "CI Plan Test"]);
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "fixture baseline"]);
    temp
}

fn cargo_generate_lockfile(repo: &Path) {
    let output = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(repo)
        .output()
        .expect("cargo generate-lockfile runs");
    assert!(
        output.status.success(),
        "cargo generate-lockfile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const fn package<'a>(path: &'a str, name: &'a str, extra_manifest: &'a str) -> PackageSpec<'a> {
    PackageSpec {
        path,
        name,
        extra_manifest,
    }
}

fn dependency(name: &str, path: &str) -> &'static str {
    Box::leak(format!("\n[dependencies]\n{name} = {{ path = \"{path}\" }}\n").into_boxed_str())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask is nested under tools")
        .to_path_buf()
}

fn successful_plan(output: &Output) -> Plan {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is a plan")
}

fn packages(plan: &Plan) -> Vec<&str> {
    plan.include
        .iter()
        .map(|entry| entry.package.as_str())
        .collect()
}

fn assert_entry_features(plan: &Plan, package: &str, expected: &[&str]) {
    let entry = plan
        .include
        .iter()
        .find(|entry| entry.package == package)
        .expect("plan entry exists");
    assert_eq!(
        entry
            .test_features
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        expected
    );
}

fn revision(repo: &Path) -> String {
    git_output(repo, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", message]);
    revision(repo)
}

fn change_source(repo: &Path, package_path: &str, marker: &str) {
    fs::write(
        repo.join(package_path).join("src/lib.rs"),
        format!("pub fn fixture_{marker}() {{}}\n"),
    )
    .expect("package source writes");
}

fn diff_plan(repo: &Path, base: &str, head: &str, comparison: &str) -> Plan {
    successful_plan(&xtask(
        repo,
        &[
            "ci-plan",
            "diff",
            "--base",
            base,
            "--head",
            head,
            "--comparison",
            comparison,
        ],
    ))
}

fn copy_tree(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source).expect("fixture directory is readable") {
        let entry = entry.expect("fixture entry is readable");
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("fixture type is readable")
            .is_dir()
        {
            fs::create_dir_all(&target).expect("fixture directory is creatable");
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("fixture file is copyable");
        }
    }
}

fn xtask(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nebula-xtask"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("xtask binary runs")
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git runs");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("git output is UTF-8")
}
