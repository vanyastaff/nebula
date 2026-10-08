//! Exercise the real worker entry point without a configured deployment.

use std::{process::Output, time::Duration};

async fn invoke(arguments: &[&str]) -> Output {
    let output = invoke_with_environment(arguments, &[]).await;
    assert!(!String::from_utf8_lossy(&output.stderr).contains("NEBULA_WORKER_ARTIFACT_SET_DIGEST"));
    output
}

async fn invoke_with_environment(arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let directory = tempfile::tempdir().expect("isolated working directory");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-worker"));
    command
        .args(arguments)
        .env_clear()
        .envs(environment.iter().copied())
        .env(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://[invalid-entry-canary",
        )
        .current_dir(directory.path())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .expect("CLI admission must exit without starting the worker")
        .expect("launch worker binary");
    assert_eq!(
        directory
            .path()
            .read_dir()
            .expect("read working directory")
            .count(),
        0
    );
    output
}

async fn rejected_startup(overrides: &[(&str, &str)], diagnostic: &str) {
    let mut environment = vec![
        (
            "NEBULA_WORKER_ARTIFACT_SET_DIGEST",
            "7171717171717171717171717171717171717171717171717171717171717171",
        ),
        ("NEBULA_WORKER_DB_PATH", "deployment.db"),
    ];
    environment.extend_from_slice(overrides);
    let output = invoke_with_environment(&[], &environment).await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(diagnostic), "{stderr}");
    assert!(!stderr.contains("submitted-secret-canary"));
}

#[tokio::test]
async fn missing_credential_key_does_not_create_database() {
    rejected_startup(&[], "NEBULA_CRED_MASTER_KEY").await;
}

#[tokio::test]
async fn malformed_credential_key_does_not_create_database() {
    rejected_startup(
        &[("NEBULA_CRED_MASTER_KEY", "submitted-secret-canary")],
        "key material decode failed",
    )
    .await;
}

#[tokio::test]
async fn malformed_legacy_key_does_not_create_database() {
    rejected_startup(
        &[
            ("NEBULA_CRED_DEV_KEY", "1"),
            ("NEBULA_CRED_LEGACY_MASTER_KEYS", "submitted-secret-canary"),
        ],
        "legacy master-key configuration",
    )
    .await;
}

#[tokio::test]
async fn help_does_not_require_deployment_configuration() {
    for flag in ["--help", "-h"] {
        let output = invoke(&[flag]).await;
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
        assert!(output.stderr.is_empty());
    }
}

#[tokio::test]
async fn version_identifies_the_build_without_initialization() {
    let output = invoke(&["--version"]).await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("nebula-worker ", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[tokio::test]
async fn invalid_arguments_fail_before_deployment_initialization() {
    for arguments in [
        &["--not-a-worker-option"][..],
        &["unexpected-positional"][..],
    ] {
        let output = invoke(arguments).await;
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("--help"));
        assert!(output.stdout.is_empty());
    }
}
