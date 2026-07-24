//! Compile-pass contract for SDK-owned procedural derives.

use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};

const FIXTURE_FILES: &[&str] = &["Cargo.toml", "src/main.rs"];

#[test]
fn sdk_only_consumer_can_expand_supported_derive_families() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/fixtures/derive_consumer");
    let fixture_manifest =
        fs::read_to_string(fixture_dir.join("Cargo.toml")).expect("read derive fixture manifest");

    let dependency_section = fixture_manifest
        .split_once("[dependencies]")
        .expect("fixture has dependencies section")
        .1
        .split_once("[workspace]")
        .expect("fixture has isolated workspace marker")
        .0;
    let dependencies = dependency_section
        .lines()
        .map(str::trim)
        .filter(|line| line.contains('='))
        .collect::<Vec<_>>();
    assert_eq!(
        dependencies,
        ["nebula = { package = \"nebula-sdk\", version = \"=0.1.0\", default-features = false }"],
        "fixture must depend on a renamed nebula-sdk and nothing else"
    );

    let temp = tempfile::tempdir().expect("create isolated derive-consumer directory");
    copy_fixture(&fixture_dir, temp.path());

    let sdk_path = manifest_dir
        .canonicalize()
        .expect("canonicalize local nebula-sdk path");
    let mut patched_manifest = fixture_manifest;
    patched_manifest.push_str(&format!(
        "\n[patch.crates-io]\nnebula-sdk = {{ path = \"{}\" }}\n",
        toml_basic_string(&sdk_path)
    ));
    fs::write(temp.path().join("Cargo.toml"), patched_manifest)
        .expect("write patched derive-consumer manifest");

    let workspace_root = manifest_dir.join("../..");
    fs::copy(
        workspace_root.join("Cargo.lock"),
        temp.path().join("Cargo.lock"),
    )
    .expect("copy workspace lockfile into derive-consumer fixture");

    let output = cargo_check(temp.path());
    assert!(
        output.status.success(),
        "SDK-only derive consumer must compile:\n{}",
        render_output(&output)
    );
}

#[test]
fn renamed_leaf_dependencies_remain_supported_with_sdk_fallbacks() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/fixtures/renamed_derive_consumer");
    let fixture_manifest = fs::read_to_string(fixture_dir.join("Cargo.toml"))
        .expect("read renamed-derive fixture manifest");

    for renamed_dependency in [
        "action-leaf",
        "credential-leaf",
        "plugin-leaf",
        "resource-leaf",
        "schema-leaf",
        "validator-leaf",
    ] {
        assert!(
            fixture_manifest.contains(&format!("{renamed_dependency} = {{ package = ")),
            "fixture must rename `{renamed_dependency}`"
        );
    }

    let temp = tempfile::tempdir().expect("create isolated renamed-consumer directory");
    copy_fixture(&fixture_dir, temp.path());

    let mut patched_manifest = fixture_manifest;
    patched_manifest.push_str("\n[patch.crates-io]\n");
    for (package, relative_path) in [
        ("nebula-sdk", "."),
        ("nebula-action", "../action"),
        ("nebula-credential", "../credential"),
        ("nebula-plugin", "../plugin"),
        ("nebula-resource", "../resource"),
        ("nebula-schema", "../schema"),
        ("nebula-validator", "../validator"),
    ] {
        let path = manifest_dir
            .join(relative_path)
            .canonicalize()
            .expect("canonicalize patched workspace package");
        patched_manifest.push_str(&format!(
            "{package} = {{ path = \"{}\" }}\n",
            toml_basic_string(&path)
        ));
    }
    fs::write(temp.path().join("Cargo.toml"), patched_manifest)
        .expect("write patched renamed-consumer manifest");

    let workspace_root = manifest_dir.join("../..");
    fs::copy(
        workspace_root.join("Cargo.lock"),
        temp.path().join("Cargo.lock"),
    )
    .expect("copy workspace lockfile into renamed-consumer fixture");

    let output = cargo_check(temp.path());
    assert!(
        output.status.success(),
        "renamed leaf derive consumer must compile:\n{}",
        render_output(&output)
    );
}

fn copy_fixture(source_root: &Path, destination_root: &Path) {
    for relative in FIXTURE_FILES {
        let source = source_root.join(relative);
        let destination = destination_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).expect("create temporary fixture directory");
        }
        fs::copy(source, destination).expect("copy committed fixture");
    }
}

fn cargo_check(fixture_root: &Path) -> Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    Command::new(cargo)
        .current_dir(fixture_root)
        .args(["check", "--offline", "--quiet"])
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_TARGET_DIR", fixture_root.join("target"))
        .output()
        .expect("run cargo check for external SDK derive consumer")
}

fn toml_basic_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn render_output(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
