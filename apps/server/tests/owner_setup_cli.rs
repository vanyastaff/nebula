//! Offline operator entrypoint: no listener, mail, cloud or fixture identity.

use std::{path::Path, process::Stdio, time::Duration};

async fn invoke(directory: &Path, arguments: &[&str], password: &[u8]) -> std::process::Output {
    invoke_with_storage(directory, arguments, password, "sqlite", None).await
}

async fn invoke_with_storage(
    directory: &Path,
    arguments: &[&str],
    password: &[u8],
    backend: &str,
    dsn: Option<&str>,
) -> std::process::Output {
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-server"));
    command
        .args(arguments)
        .env_clear()
        .env("API_EXECUTION_BACKEND", backend)
        .env("API_EXECUTION_DB_PATH", "deployment.db")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "invalid-unused-telemetry")
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Keep the OS loader context for Winsock; no Nebula configuration is inherited.
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
        command.env("SYSTEMROOT", system_root);
    }
    if let Some(dsn) = dsn {
        command.env("DATABASE_URL", dsn);
    }
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(password).await;
    drop(stdin);
    tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("setup must terminate without serving")
        .unwrap()
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires live PostgreSQL; run explicitly with DATABASE_URL"]
async fn postgres_operator_cli_uses_the_selected_deployment_database() {
    use futures::FutureExt;
    let dsn = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&dsn)
        .await
        .unwrap();
    let schema = format!("owner_cli_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let mut url = url::Url::parse(&dsn).unwrap();
    let parameters: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let options = parameters
        .iter()
        .filter(|(key, _)| key == "options")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    url.query_pairs_mut()
        .clear()
        .extend_pairs(parameters.iter().filter(|(key, _)| key != "options"))
        .append_pair("options", &format!("{options} -csearch_path={schema}"));
    let result = std::panic::AssertUnwindSafe(async {
        let directory = tempfile::tempdir().unwrap();
        let output = invoke_with_storage(directory.path(), BEGIN, b"Postgres-password-canary-2026", "postgres", Some(url.as_str())).await;
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "accepted");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("password-canary"));
        assert_eq!(directory.path().read_dir().unwrap().count(), 0, "PG selection must not create SQLite files");
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect(url.as_str()).await.unwrap();
        let state: (String, bool, String) = sqlx::query_as(
            "SELECT u.email, u.email_verified_at IS NULL, m.role FROM users u CROSS JOIN org_memberships m",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(state, ("owner@local.example".into(), true, "OrgOwner".into()));
        pool.close().await;
        for operation in ["status", "resume"] {
            let output = invoke_with_storage(directory.path(), &["setup", operation], b"", "postgres", Some(url.as_str())).await;
            assert!(output.status.success(), "{output:?}");
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "accepted");
        }
    }).catch_unwind().await;
    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await;
    admin.close().await;
    cleanup.unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

const BEGIN: &[&str] = &[
    "setup",
    "begin",
    "--email",
    "Owner@Local.example",
    "--display-name",
    "Local owner",
    "--organization-name",
    "My workflows",
];

#[tokio::test]
async fn offline_owner_setup_is_durable_and_does_not_verify_email_or_issue_session() {
    let directory = tempfile::tempdir().unwrap();
    let output = invoke(directory.path(), BEGIN, b"Owner-password-canary-2026\n").await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "accepted");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Owner-password-canary"));

    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(directory.path().join("deployment.db")),
    )
    .await
    .unwrap();
    let user: (String, Option<i64>, String) =
        sqlx::query_as("SELECT email, email_verified_at, password_hash FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(user.0, "owner@local.example");
    assert_eq!(user.1, None);
    assert!(
        nebula_api::domain::auth::backend::password::verify_password(
            &user.2,
            "Owner-password-canary-2026",
        )
        .unwrap()
    );
    for table in ["sessions", "verification_tokens"] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }
    let role: String = sqlx::query_scalar("SELECT role FROM org_memberships")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(role, "OrgOwner");
    pool.close().await;
    let status = invoke(directory.path(), &["setup", "status"], b"").await;
    assert!(status.status.success(), "{status:?}");
    assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "accepted");
    let repeated = invoke(directory.path(), BEGIN, b"Different-password-canary\n").await;
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("resume"));
    let resume = invoke(directory.path(), &["setup", "resume"], b"").await;
    assert!(resume.status.success(), "{resume:?}");
    assert_eq!(String::from_utf8_lossy(&resume.stdout).trim(), "accepted");
    use nebula_api::domain::auth::backend::{AuthBackend, DurableAuthBackend, PasswordOutcome};
    use nebula_storage::auth::{AuthPersistence, identity_secret::IdentitySecretCodec};
    use std::sync::Arc;
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(
        sqlx::sqlite::SqliteConnectOptions::new().filename(directory.path().join("deployment.db")),
    )
    .await
    .unwrap();
    let key = Arc::new(
        nebula_storage::credential::EnvKeyProvider::from_base64(
            "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=",
        )
        .unwrap(),
    );
    let mail = Arc::new(nebula_api::ports::email::EchoSink::default());
    let auth = DurableAuthBackend::new(
        AuthPersistence::sqlite(
            &deployment,
            Arc::new(IdentitySecretCodec::new(key).unwrap()),
        ),
        mail.clone(),
        None,
    );
    assert!(matches!(
        auth.authenticate_password("owner@local.example", "Owner-password-canary-2026", None)
            .await
            .unwrap(),
        PasswordOutcome::Authenticated(_)
    ));
    assert!(
        auth.authenticate_password("owner@local.example", "Different-password-canary", None)
            .await
            .is_err()
    );
    assert!(
        mail.drain().is_empty(),
        "ordinary local login sends no email"
    );
    deployment.pool().close().await;
}

#[tokio::test]
async fn rejected_setup_input_does_not_create_storage_or_echo_the_secret() {
    for password in [
        b"short".to_vec(),
        vec![b'x'; 4097],
        b"canary\nsecond-line".to_vec(),
        vec![0xff],
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = invoke(directory.path(), BEGIN, &password).await;
        assert!(!output.status.success());
        assert_eq!(directory.path().read_dir().unwrap().count(), 0);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("canary"));
    }
}

#[tokio::test]
async fn failed_tenant_acceptance_resumes_the_committed_account_in_a_new_process() {
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("deployment.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(deployment.pool())
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_setup_receipt BEFORE INSERT ON tenant_provisioning_receipts BEGIN SELECT RAISE(ABORT, 'private-receipt-canary'); END")
        .execute(deployment.pool()).await.unwrap();
    deployment.pool().close().await;
    let failed = invoke(directory.path(), BEGIN, b"Owner-password-canary-2026\n").await;
    assert!(!failed.status.success());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("canary"));
    let pending = invoke(directory.path(), &["setup", "status"], b"").await;
    assert!(pending.status.success(), "{pending:?}");
    assert_eq!(String::from_utf8_lossy(&pending.stdout).trim(), "pending");
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(options.clone())
        .await
        .unwrap();
    let pool = deployment.pool();
    let user_id: Vec<u8> = sqlx::query_scalar("SELECT id FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    for table in [
        "orgs",
        "workspaces",
        "org_memberships",
        "tenant_provisioning_receipts",
    ] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "tenant acceptance must roll back {table}");
    }
    sqlx::query("DROP TRIGGER reject_setup_receipt")
        .execute(pool)
        .await
        .unwrap();
    pool.close().await;
    let resumed = invoke(directory.path(), &["setup", "resume"], b"").await;
    assert!(resumed.status.success(), "{resumed:?}");
    assert_eq!(String::from_utf8_lossy(&resumed.stdout).trim(), "accepted");
    let deployment = nebula_storage::sqlite::DeploymentPool::connect(options)
        .await
        .unwrap();
    let retained: Vec<u8> = sqlx::query_scalar("SELECT id FROM users")
        .fetch_one(deployment.pool())
        .await
        .unwrap();
    assert_eq!(retained, user_id, "resume must keep the committed account");
    let grants: i64 =
        sqlx::query_scalar("SELECT count(*) FROM org_memberships WHERE role = 'OrgOwner'")
            .fetch_one(deployment.pool())
            .await
            .unwrap();
    assert_eq!(grants, 1);
    deployment.pool().close().await;
}

#[tokio::test]
async fn accepted_setup_does_not_restore_purged_tenant_or_account_on_resume() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        invoke(directory.path(), BEGIN, b"Owner-password-canary-2026\n")
            .await
            .status
            .success()
    );
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("deployment.db"))
            .foreign_keys(true),
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM orgs")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let resumed = invoke(directory.path(), &["setup", "resume"], b"").await;
    assert!(resumed.status.success(), "{resumed:?}");
    assert_eq!(String::from_utf8_lossy(&resumed.stdout).trim(), "accepted");
    let repeated = invoke(directory.path(), BEGIN, b"Replacement-password-canary\n").await;
    assert!(!repeated.status.success());
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(directory.path().join("deployment.db")),
    )
    .await
    .unwrap();
    for table in ["orgs", "workspaces", "org_memberships", "users"] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "resume must not recreate {table}");
    }
    pool.close().await;
}
