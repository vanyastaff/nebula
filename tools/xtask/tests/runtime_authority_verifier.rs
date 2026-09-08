use std::process::Command;

#[test]
fn missing_trusted_provenance_is_a_verification_failure_without_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nebula-xtask"))
        .args([
            "north-star-gates",
            "verify-runtime-authority",
            "--artifact-root",
        ])
        .arg(directory.path())
        .arg("--expected-provenance")
        .arg(directory.path().join("absent.json"))
        .args([
            "--source-revision",
            &"a".repeat(40),
            "--repository",
            "fixture/repository",
            "--run-id",
            "1234",
            "--run-attempt",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("runtime authority artifact")
    );
}
