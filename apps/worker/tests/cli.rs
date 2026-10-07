//! Exercise the real worker entry point without a configured deployment.

use std::{process::Output, time::Duration};

async fn invoke(arguments: &[&str]) -> Output {
    let directory = tempfile::tempdir().expect("isolated working directory");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-worker"));
    command
        .args(arguments)
        .env_clear()
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
    assert!(!String::from_utf8_lossy(&output.stderr).contains("NEBULA_WORKER_ARTIFACT_SET_DIGEST"));
    output
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
