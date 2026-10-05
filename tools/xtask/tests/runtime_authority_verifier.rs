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
            "--expected-provenance-sha256",
            &"a".repeat(64),
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

#[test]
fn missing_or_invalid_manifest_digest_emits_no_effective_states() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("expected.json");
    std::fs::write(&manifest, b"invalid JSON").unwrap();
    for digest in [None, Some(""), Some("not-a-digest")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nebula-xtask"));
        command
            .args([
                "north-star-gates",
                "verify-runtime-authority",
                "--artifact-root",
            ])
            .arg(directory.path())
            .arg("--expected-provenance")
            .arg(&manifest)
            .args([
                "--source-revision",
                &"a".repeat(40),
                "--repository",
                "fixture/repository",
                "--run-id",
                "1234",
                "--run-attempt",
                "1",
            ]);
        if let Some(digest) = digest {
            command.args(["--expected-provenance-sha256", digest]);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        if digest.is_none() {
            assert!(stderr.contains("--expected-provenance-sha256"));
        } else {
            assert!(stderr.contains("trusted digest"));
        }
    }
}

#[test]
fn replaced_manifest_emits_no_effective_states() {
    use std::fmt::Write as _;

    use sha2::{Digest, Sha256};

    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("expected.json");
    let original = b"original trusted manifest";
    let mut trusted_digest = String::new();
    for byte in Sha256::digest(original) {
        write!(&mut trusted_digest, "{byte:02x}").unwrap();
    }
    std::fs::write(&manifest, b"rewritten manifest").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nebula-xtask"))
        .args([
            "north-star-gates",
            "verify-runtime-authority",
            "--artifact-root",
        ])
        .arg(directory.path())
        .arg("--expected-provenance")
        .arg(&manifest)
        .args([
            "--expected-provenance-sha256",
            &trusted_digest,
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
            .contains("trusted digest")
    );
}
