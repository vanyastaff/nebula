//! Exercise process admission before deployment configuration or side effects.

use std::{process::Output, time::Duration};

async fn invoke(arguments: &[&str]) -> Output {
    let output = invoke_with_environment(
        arguments,
        &[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://[invalid-entry-canary",
        )],
    )
    .await;
    assert!(!String::from_utf8_lossy(&output.stderr).contains("invalid-entry-canary"));
    output
}

async fn invoke_with_environment(arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let directory = tempfile::tempdir().expect("isolated working directory");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-server"));
    command
        .args(arguments)
        .env_clear()
        .envs(environment.iter().copied())
        .current_dir(directory.path())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .expect("CLI admission must exit without starting the server")
        .expect("launch server binary");
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
        ("NEBULA_ENV", "development"),
        ("API_EXECUTION_BACKEND", "sqlite"),
        ("API_EXECUTION_DB_PATH", "deployment.db"),
        ("NEBULA_CRED_DEV_KEY", "1"),
        (
            "NEBULA_WORKER_ARTIFACT_SET_DIGEST",
            "7171717171717171717171717171717171717171717171717171717171717171",
        ),
        ("SERVER_BIND_ADDRESS", "127.0.0.1:0"),
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "disabled"),
    ];
    environment.extend_from_slice(overrides);
    let output = invoke_with_environment(&[], &environment).await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(diagnostic), "{stderr}");
    assert!(!stderr.contains("submitted-secret-canary"));
}

#[tokio::test]
async fn memory_is_not_a_deployment_backend() {
    rejected_startup(
        &[("API_EXECUTION_BACKEND", "memory")],
        "memory is a test adapter",
    )
    .await;
}

#[tokio::test]
async fn separate_workers_reject_sqlite_before_database_creation() {
    rejected_startup(
        &[("NEBULA_EXECUTION", "separate-workers")],
        "separate workers require PostgreSQL",
    )
    .await;
}

#[tokio::test]
async fn removed_postgres_auth_selector_fails_before_opening_sqlite() {
    rejected_startup(
        &[("API_AUTH_BACKEND", "postgres")],
        "API_AUTH_BACKEND has been removed",
    )
    .await;
}

#[tokio::test]
async fn removed_sqlite_auth_selector_fails_before_opening_storage() {
    rejected_startup(
        &[
            ("API_AUTH_BACKEND", "sqlite"),
            ("API_EXECUTION_BACKEND", "memory"),
        ],
        "API_AUTH_BACKEND has been removed",
    )
    .await;
}

#[tokio::test]
async fn removed_auth_selector_cannot_silently_override_the_deployment() {
    for value in ["", "memory", "sqlite", "submitted-secret-canary"] {
        rejected_startup(
            &[("API_AUTH_BACKEND", value)],
            "API_AUTH_BACKEND has been removed",
        )
        .await;
    }
}

#[tokio::test]
async fn setup_rejects_removed_auth_configuration_before_creating_storage() {
    let output = invoke_with_environment(
        &["setup", "status"],
        &[
            ("API_EXECUTION_BACKEND", "sqlite"),
            ("API_EXECUTION_DB_PATH", "deployment.db"),
            ("API_AUTH_BACKEND", "submitted-secret-canary"),
        ],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("API_AUTH_BACKEND has been removed"));
    assert!(!diagnostic.contains("submitted-secret-canary"));
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn non_unicode_auth_selector_cannot_be_treated_as_absent() {
    #[cfg(windows)]
    let value = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0xd800])
    };
    #[cfg(unix)]
    let value = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![0xff])
    };
    let directory = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-server"));
    command
        .args(["setup", "status"])
        .env_clear()
        .env("API_EXECUTION_BACKEND", "sqlite")
        .env("API_EXECUTION_DB_PATH", "deployment.db")
        .env("API_AUTH_BACKEND", value)
        .current_dir(directory.path())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("API_AUTH_BACKEND has been removed"));
    assert_eq!(directory.path().read_dir().unwrap().count(), 0);
}

#[tokio::test]
async fn postgres_replay_requires_postgres_deployment_before_opening_sqlite() {
    rejected_startup(
        &[("API_IDEMPOTENCY_BACKEND", "postgres")],
        "API_IDEMPOTENCY_BACKEND=postgres requires API_EXECUTION_BACKEND=postgres",
    )
    .await;
}

#[tokio::test]
async fn missing_credential_key_does_not_create_database() {
    rejected_startup(&[("NEBULA_CRED_DEV_KEY", "0")], "NEBULA_CRED_MASTER_KEY").await;
}

#[tokio::test]
async fn malformed_credential_key_does_not_create_database() {
    rejected_startup(
        &[
            ("NEBULA_CRED_DEV_KEY", "0"),
            ("NEBULA_CRED_MASTER_KEY", "submitted-secret-canary"),
        ],
        "key material decode failed",
    )
    .await;
}

#[tokio::test]
async fn malformed_legacy_key_does_not_create_database() {
    rejected_startup(
        &[("NEBULA_CRED_LEGACY_MASTER_KEYS", "submitted-secret-canary")],
        "legacy master-key configuration",
    )
    .await;
}

#[tokio::test]
async fn malformed_worker_identity_does_not_create_database() {
    rejected_startup(
        &[(
            "NEBULA_WORKER_ARTIFACT_SET_DIGEST",
            "submitted-secret-canary",
        )],
        "NEBULA_WORKER_ARTIFACT_SET_DIGEST",
    )
    .await;
}

#[tokio::test]
async fn incomplete_tenant_bootstrap_does_not_create_database() {
    rejected_startup(
        &[("NEBULA_BOOTSTRAP_ORG_NAME", "incomplete")],
        "tenant bootstrap configuration",
    )
    .await;
}

#[tokio::test]
async fn occupied_listener_does_not_create_database_or_start_owners() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    rejected_startup(&[("SERVER_BIND_ADDRESS", &address)], "server failed").await;
}

#[tokio::test]
async fn malformed_bind_override_does_not_create_database() {
    rejected_startup(
        &[("SERVER_BIND_ADDRESS", "not-an-address")],
        "SERVER_BIND_ADDRESS",
    )
    .await;
}

#[tokio::test]
async fn help_does_not_require_deployment_configuration() {
    for flag in ["--help", "-h"] {
        let output = invoke(&[flag]).await;
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage:"));
        assert!(stdout.contains("--transport"));
        assert!(output.stderr.is_empty());
    }
}

#[tokio::test]
async fn version_identifies_the_build_without_initialization() {
    let output = invoke(&["--version"]).await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("nebula-server ", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[tokio::test]
async fn invalid_arguments_fail_before_deployment_initialization() {
    for arguments in [
        &["--unknown-option", "api"][..],
        &["--transport", "invalid"][..],
    ] {
        let output = invoke(arguments).await;
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("--help"));
        assert!(output.stdout.is_empty());
    }
}
