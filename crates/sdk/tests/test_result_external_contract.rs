//! External-consumer contract for the credential test-result migration.
//!
//! The fixture deliberately depends on `nebula-sdk` alone. It proves that the
//! curated integration persona path exposes the payload-free result shape and
//! that the removed provider-controlled `reason` payload stays unnameable.

use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};

const FIXTURE_FILES: &[&str] = &[
    "Cargo.toml",
    "src/bin/positive.rs",
    "src/bin/removed_reason.rs",
];

#[test]
fn sdk_only_consumer_uses_payload_free_test_result() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/fixtures/test_result_consumer");
    let fixture_manifest = fs::read_to_string(fixture_dir.join("Cargo.toml"))
        .expect("read committed external-consumer manifest");

    let nebula_dependencies = fixture_manifest
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("nebula-") && line.contains('='))
        .collect::<Vec<_>>();
    assert_eq!(
        nebula_dependencies,
        ["nebula-sdk = { version = \"=0.1.0\", default-features = false }"],
        "fixture must have exactly one nebula-* dependency and it must be nebula-sdk"
    );
    assert!(
        !fixture_manifest.contains("nebula-credential"),
        "fixture must not bypass the SDK with a direct credential dependency"
    );

    let temp = tempfile::tempdir().expect("create isolated external-consumer directory");
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
        .expect("write patched manifest in temporary fixture");

    let workspace_root = manifest_dir.join("../..");
    fs::copy(
        workspace_root.join("Cargo.lock"),
        temp.path().join("Cargo.lock"),
    )
    .expect("copy workspace lockfile into temporary fixture");

    let positive = cargo_check(temp.path(), "positive");
    assert!(
        positive.status.success(),
        "new payload-free SDK contract must compile:\n{}",
        render_output(&positive)
    );

    let removed_reason = cargo_check(temp.path(), "removed_reason");
    assert!(
        !removed_reason.status.success(),
        "removed free-form reason field must not compile"
    );

    let stderr = String::from_utf8_lossy(&removed_reason.stderr);
    assert!(
        stderr.contains("variant `TestResult::Failed` has no field named `reason`"),
        "negative fixture must fail for the removed field, not for an unrelated reason:\n{}",
        render_output(&removed_reason)
    );
    assert!(
        stderr.contains("available fields are: `code`"),
        "compiler must identify the replacement payload field:\n{}",
        render_output(&removed_reason)
    );

    let lower = stderr.to_ascii_lowercase();
    for unrelated in [
        "unresolved import",
        "use of unresolved module or unlinked crate",
        "can't find crate",
        "could not find `integration` in `nebula_sdk`",
        "could not find `credential` in `integration`",
        "no matching package named",
        "failed to get `nebula-sdk`",
    ] {
        assert!(
            !lower.contains(unrelated),
            "negative fixture failed for unrelated dependency/import error `{unrelated}`:\n{}",
            render_output(&removed_reason)
        );
    }
}

fn copy_fixture(source_root: &Path, destination_root: &Path) {
    for relative in FIXTURE_FILES {
        let source = source_root.join(relative);
        let destination = destination_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).expect("create temporary fixture directory");
        }
        fs::copy(source, destination).expect("copy committed fixture into temporary directory");
    }
}

fn cargo_check(fixture_root: &Path, binary: &str) -> Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    Command::new(cargo)
        .current_dir(fixture_root)
        .args(["check", "--offline", "--quiet", "--bin", binary])
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_TARGET_DIR", fixture_root.join("target"))
        .output()
        .expect("run cargo check for external SDK consumer")
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
