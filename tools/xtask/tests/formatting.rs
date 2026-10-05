#[path = "support/fixture_process.rs"]
mod fixture_process;
use fixture_process::fixture_command;

use std::{fs, path::Path, process::Output};

use tempfile::TempDir;

#[test]
fn checks_nondefault_members_and_optional_local_dependencies_outside_workspace() {
    let fixture = fixture_workspace();
    let output = check(fixture.path());
    assert_success(&output);
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    for package in ["fixture-first", "fixture-second", "fixture-local"] {
        assert!(
            diagnostic.contains(&format!("fmt-check: {package}")),
            "missing metadata package {package}: {diagnostic}"
        );
    }

    // Each source must fail the real formatter, including the nondefault member
    // and optional path dependency which cannot come from workspace_members.
    for relative in ["first", "second", "local dependency"] {
        let source = fixture.path().join(relative).join("src/lib.rs");
        fs::write(&source, "pub fn example( ){ }\n").expect("write badly formatted source");
        let old = fixture_command("cargo")
            .current_dir(fixture.path())
            .args(["fmt", "--all", "--", "--check"])
            .output()
            .expect("start original formatter");
        assert!(!old.status.success(), "old full gate must cover {relative}");
        let output = check(fixture.path());
        assert!(
            !output.status.success(),
            "new full gate must reject bad formatting in {relative}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("failed with status"),
            "must propagate the real formatter failure: {output:?}"
        );
        assert_eq!(
            fs::read_to_string(&source).expect("read unchanged source"),
            "pub fn example( ){ }\n",
            "formatting check must not repair sources"
        );
        fs::write(source, "pub fn example() {}\n").expect("restore formatted source");
    }
}

#[test]
fn retains_workspace_rustfmt_configuration() {
    let fixture = fixture_workspace();
    fs::write(fixture.path().join("rustfmt.toml"), "hard_tabs = true\n")
        .expect("write workspace formatter configuration");
    fs::write(
        fixture.path().join("second/src/lib.rs"),
        "pub fn example() {\n\tlet _number = 1;\n}\n",
    )
    .expect("write source formatted with workspace configuration");
    assert_success(&check(fixture.path()));
}

fn fixture_workspace() -> TempDir {
    let fixture = tempfile::tempdir().expect("create fixture workspace");
    fs::write(
        fixture.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"first\", \"second\"]\n\
         default-members = [\"first\"]\nexclude = [\"local dependency\"]\nresolver = \"3\"\n",
    )
    .expect("write workspace manifest");
    for (relative, name) in [
        ("first", "fixture-first"),
        ("second", "fixture-second"),
        ("local dependency", "fixture-local"),
    ] {
        let package = fixture.path().join(relative);
        fs::create_dir_all(package.join("src")).expect("create source directory");
        let dependency = if relative == "first" {
            "\n[dependencies]\nfixture-local = { path = \"../local dependency\", optional = true }\n"
        } else {
            ""
        };
        fs::write(
            package.join("Cargo.toml"),
            format!(
                "[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n{dependency}"
            ),
        )
        .expect("write package manifest");
        fs::write(package.join("src/lib.rs"), "pub fn example() {}\n")
            .expect("write formatted source");
    }
    assert_success(
        &fixture_command("cargo")
            .current_dir(fixture.path())
            .args(["generate-lockfile", "--offline"])
            .output()
            .expect("generate fixture lockfile"),
    );
    fixture
}

fn check(root: &Path) -> Output {
    fixture_command(env!("CARGO_BIN_EXE_nebula-xtask"))
        .current_dir(root)
        .arg("fmt-check")
        .output()
        .expect("start full formatting check")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
