//! Exercise the real matrix entry point in a child process so environment
//! overrides cannot race other tests or accidentally connect to a developer DB.

use std::process::{Command, Output};

mod observations;

fn postgres_case() -> Command {
    let mut command = Command::new(std::env::current_exe().expect("test executable is available"));
    command
        .args([
            "--exact",
            "create_get_roundtrip::case_3_postgres",
            "--nocapture",
        ])
        .env_remove("DATABASE_URL")
        .env_remove("NEBULA_REQUIRE_POSTGRES");
    command
}

fn assert_case_ran(output: &Output) {
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "the subprocess must execute exactly the requested matrix case: {output:?}"
    );
}

#[test]
fn optional_postgres_without_url_skips() {
    let output = postgres_case().output().expect("matrix subprocess starts");
    assert_case_ran(&output);
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("skipping Postgres case"),
        "{output:?}"
    );
}

#[test]
fn required_postgres_without_url_fails() {
    let output = postgres_case()
        .env("NEBULA_REQUIRE_POSTGRES", "1")
        .output()
        .expect("matrix subprocess starts");
    assert_required_failure(&output);
}

fn assert_required_failure(output: &Output) {
    assert_case_ran(output);
    assert!(!output.status.success(), "{output:?}");
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    let expected = if cfg!(feature = "postgres") {
        "NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL"
    } else {
        "NEBULA_REQUIRE_POSTGRES is set but the postgres feature is disabled"
    };
    assert!(diagnostic.contains(expected), "{output:?}");
}

#[cfg(unix)]
#[test]
fn required_postgres_with_nonunicode_url_fails() {
    use std::os::unix::ffi::OsStrExt;
    let output = postgres_case()
        .env("NEBULA_REQUIRE_POSTGRES", "1")
        .env("DATABASE_URL", std::ffi::OsStr::from_bytes(b"\xff"))
        .output()
        .expect("matrix subprocess starts");
    assert_required_failure(&output);
}
