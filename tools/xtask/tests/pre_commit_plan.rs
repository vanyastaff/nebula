//! Real CLI and Cargo-metadata ownership tests, independent of the hook recording double.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const FIXTURE: &str = "crates/container/author/tests/fixtures/consumer";
const OWNER: &str = "fixture-author-surface";
const TARGET: &str = "external_contract";

#[test]
fn cargo_ownership_selects_deepest_member_without_reverse_dependents() {
    let workspace = workspace_fixture();
    declare_fixture(workspace.path(), FIXTURE, &policy(None, TARGET));
    let source = format!("{FIXTURE}/src/main.rs");
    let output = planner(workspace.path(), &[&source, &source]);
    assert_eq!(
        successful_plan(&output),
        json!({
            "schema_version": 1,
            "packages": [OWNER],
            "standalone_manifests": [],
            "fixtures": [{
                "manifest_path": format!("{FIXTURE}/Cargo.toml"),
                "owner": OWNER,
                "test_target": TARGET,
            }],
        })
    );
    assert_eq!(output.stdout, planner(workspace.path(), &[&source]).stdout);

    let ci = xtask(workspace.path(), &["ci-plan", "full"]);
    assert_eq!(
        successful_plan(&ci),
        json!({
            "schema_version": 1,
            "scope": "full",
            "reason": "full-request",
            "count": 3,
            "include": [
                {"package": OWNER, "test_features": []},
                {"package": "fixture-container", "test_features": []},
                {"package": "fixture-dependent", "test_features": []},
            ],
        }),
        "pre-commit ownership must not change the existing ci-plan protocol"
    );
}

#[test]
fn optional_owner_assertion_must_match_cargo_not_another_valid_target_owner() {
    let workspace = workspace_fixture();
    declare_fixture(workspace.path(), FIXTURE, &policy(Some(OWNER), TARGET));
    let source = format!("{FIXTURE}/src/main.rs");
    assert_eq!(
        successful_plan(&planner(workspace.path(), &[&source]))["packages"],
        json!([OWNER])
    );

    declare_fixture(
        workspace.path(),
        FIXTURE,
        &policy(Some("fixture-container"), TARGET),
    );
    assert_failure(
        &planner(workspace.path(), &[&source]),
        &[
            "declares owner `fixture-container`",
            "Cargo ownership is `fixture-author-surface`",
        ],
    );
}

#[test]
fn nonexistent_integration_target_rejects_the_complete_plan() {
    let workspace = workspace_fixture();
    declare_fixture(workspace.path(), FIXTURE, &policy(None, "missing_contract"));
    assert_failure(
        &planner(
            workspace.path(),
            &[
                "crates/dependent/src/lib.rs",
                &format!("{FIXTURE}/src/main.rs"),
            ],
        ),
        &["nonexistent integration test `missing_contract`", OWNER],
    );
}

#[test]
fn malformed_fixture_declarations_fail_closed() {
    let workspace = workspace_fixture();
    for declaration in [
        "[package.metadata.nebula]\nfixture = false\n",
        "[package.metadata.nebula.fixture]\nowner = 7\ntest-target = \"external_contract\"\n",
        "[package.metadata.nebula.fixture]\ntest-target = [\"external_contract\"]\n",
        "[package.metadata.nebula.fixture]\nowner = \"fixture-author-surface\"\n",
        "[package.metadata.nebula.fixture]\ntest-target = \"external_contract\"\nunknown = true\n",
    ] {
        declare_fixture(workspace.path(), FIXTURE, declaration);
        assert_failure(
            &planner(workspace.path(), &[&format!("{FIXTURE}/src/main.rs")]),
            &["fixture policy", "cannot be decoded"],
        );
    }
}

#[test]
fn standalone_paths_with_spaces_preserve_direct_checks_without_resolving_dependencies() {
    let workspace = workspace_fixture();
    let standalone = "tools/independent probe";
    declare_fixture(workspace.path(), standalone, "");
    let output = planner(workspace.path(), &["tools/independent probe/src/main.rs"]);
    assert_eq!(
        successful_plan(&output),
        json!({
            "schema_version": 1,
            "packages": [],
            "standalone_manifests": ["tools/independent probe/Cargo.toml"],
            "fixtures": [],
        })
    );
}

#[cfg(windows)]
#[test]
fn windows_git_paths_keep_native_ownership_and_portable_manifest_output() {
    let workspace = workspace_fixture();
    declare_fixture(workspace.path(), FIXTURE, &policy(None, TARGET));
    declare_fixture(workspace.path(), "tools/independent probe", "");
    let source = format!("{FIXTURE}/src/main.rs");
    let native_manifest = Path::new(FIXTURE).join("Cargo.toml");
    assert!(native_manifest.to_str().unwrap().contains('\\'));
    assert_eq!(native_manifest, Path::new(&format!("{FIXTURE}/Cargo.toml")));

    let output = planner(
        workspace.path(),
        &[
            &source,
            "crates/container/author/src/lib.rs",
            "tools/independent probe/src/main.rs",
        ],
    );
    assert_eq!(
        successful_plan(&output),
        json!({
            "schema_version": 1,
            "packages": [OWNER],
            "standalone_manifests": ["tools/independent probe/Cargo.toml"],
            "fixtures": [{
                "manifest_path": format!("{FIXTURE}/Cargo.toml"),
                "owner": OWNER,
                "test_target": TARGET,
            }],
        })
    );

    for invalid in [
        source.replace('/', "\\"),
        "C:/outside.rs".to_owned(),
        "C:outside.rs".to_owned(),
        "//server/share/outside.rs".to_owned(),
    ] {
        assert_failure(
            &planner(workspace.path(), &[&invalid]),
            &["workspace-relative path"],
        );
    }
}

#[test]
fn a_declared_fixture_outside_workspace_ownership_is_not_redirected() {
    let workspace = workspace_fixture();
    declare_fixture(
        workspace.path(),
        "tools/orphan",
        &policy(Some(OWNER), TARGET),
    );
    assert_failure(
        &planner(workspace.path(), &["tools/orphan/src/main.rs"]),
        &["no unambiguous Cargo workspace owner"],
    );
}

#[cfg(unix)]
#[test]
fn symlinked_standalone_manifest_cannot_escape_the_canonical_workspace_root() {
    let workspace = workspace_fixture();
    let outside = tempfile::tempdir().expect("create standalone manifest outside the workspace");
    declare_fixture(outside.path(), "consumer", "");
    write(
        workspace.path(),
        "tools/escaped/src/main.rs",
        "fn main() {}\n",
    );
    std::os::unix::fs::symlink(
        outside.path().join("consumer/Cargo.toml"),
        workspace.path().join("tools/escaped/Cargo.toml"),
    )
    .expect("link the candidate manifest to an external file");

    assert_failure(
        &planner(workspace.path(), &["tools/escaped/src/main.rs"]),
        &["workspace-relative path", "tools/escaped/Cargo.toml"],
    );
}

#[test]
fn unsafe_paths_are_rejected_without_a_partial_plan() {
    let workspace = workspace_fixture();
    for path in [
        "../outside.rs",
        "/outside.rs",
        "crates/../outside.rs",
        "crates/bad\nname.rs",
        "crates/bad\\name.rs",
    ] {
        assert_failure(
            &planner(workspace.path(), &[path]),
            &["workspace-relative path"],
        );
    }
}

#[test]
fn live_sdk_declarations_bind_each_fixture_to_its_actual_owner_harness() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inventory = [
        ("derive_consumer", "src/main.rs", "derive_external_contract"),
        (
            "public_perimeter_consumer",
            "src/bin/positive.rs",
            "public_perimeter_external_contract",
        ),
        (
            "renamed_derive_consumer",
            "src/main.rs",
            "derive_external_contract",
        ),
        (
            "test_result_consumer",
            "src/bin/positive.rs",
            "test_result_external_contract",
        ),
    ];
    let paths = inventory
        .map(|(fixture, source, _)| format!("crates/sdk/tests/fixtures/{fixture}/{source}"));
    let output = planner(&root, &paths.iter().map(String::as_str).collect::<Vec<_>>());
    let expected = inventory.map(|(fixture, _, target)| {
        json!({
            "manifest_path": format!("crates/sdk/tests/fixtures/{fixture}/Cargo.toml"),
            "owner": "nebula-sdk",
            "test_target": target,
        })
    });
    assert_eq!(
        successful_plan(&output),
        json!({
            "schema_version": 1,
            "packages": ["nebula-sdk"],
            "standalone_manifests": [],
            "fixtures": expected,
        })
    );
}

fn workspace_fixture() -> TempDir {
    let directory = tempfile::tempdir().expect("create synthetic workspace");
    write(
        directory.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/container\", \"crates/container/author\", \"crates/dependent\"]\n",
    );
    for (path, name, dependencies) in [
        ("crates/container", "fixture-container", ""),
        ("crates/container/author", OWNER, ""),
        (
            "crates/dependent",
            "fixture-dependent",
            "[dependencies]\nfixture-author-surface = { path = \"../container/author\" }\n",
        ),
    ] {
        write(
            directory.path(),
            &format!("{path}/Cargo.toml"),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{dependencies}"
            ),
        );
        write(
            directory.path(),
            &format!("{path}/src/lib.rs"),
            "pub fn witness() {}\n",
        );
        write(
            directory.path(),
            &format!("{path}/tests/{TARGET}.rs"),
            "#[test]\nfn synthetic_target() {}\n",
        );
    }
    let output = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(directory.path())
        .output()
        .expect("generate synthetic workspace lockfile");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    directory
}

fn declare_fixture(root: &Path, directory: &str, declaration: &str) {
    write(
        root,
        &format!("{directory}/Cargo.toml"),
        &format!(
            "[package]\nname = \"fixture-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nintentionally-unpublished-fixture-dependency = \"=999.0.0\"\n\n[workspace]\n\n{declaration}"
        ),
    );
    write(root, &format!("{directory}/src/main.rs"), "fn main() {}\n");
}

fn policy(owner: Option<&str>, target: &str) -> String {
    let owner = owner.map_or_else(String::new, |owner| format!("owner = \"{owner}\"\n"));
    format!("[package.metadata.nebula.fixture]\n{owner}test-target = \"{target}\"\n")
}

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("fixture file has parent"))
        .expect("create fixture parent");
    fs::write(path, source).expect("write fixture file");
}

fn planner(root: &Path, paths: &[&str]) -> Output {
    let mut arguments = vec!["pre-commit-plan", "--"];
    arguments.extend(paths);
    xtask(root, &arguments)
}

fn xtask(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nebula-xtask"))
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("run the real xtask CLI")
}

fn successful_plan(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("planner stdout is JSON")
}

fn assert_failure(output: &Output, reasons: &[&str]) {
    assert!(
        !output.status.success(),
        "invalid plan unexpectedly accepted"
    );
    assert!(
        output.stdout.is_empty(),
        "failure must not emit a partial plan"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for reason in reasons {
        assert!(stderr.contains(reason), "expected `{reason}` in {stderr}");
    }
}
