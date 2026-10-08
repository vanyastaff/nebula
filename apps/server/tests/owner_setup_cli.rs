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
        assert_serving_identity_survives_restart(
            directory.path(), "postgres", Some(url.as_str()), "Postgres-password-canary-2026",
        ).await;
        assert_execution_survives_restart(
            directory.path(), "postgres", Some(url.as_str()), "Postgres-password-canary-2026",
            ReopenMoment::Completed,
        ).await;
        assert_execution_survives_restart(
            directory.path(), "postgres", Some(url.as_str()), "Postgres-password-canary-2026",
            ReopenMoment::TimerParked,
        ).await;
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

struct ServingProcess {
    child: tokio::process::Child,
    output: std::sync::Arc<std::sync::Mutex<ProcessOutput>>,
    readers: Vec<tokio::task::JoinHandle<()>>,
    base_url: String,
}

#[derive(Default)]
struct ProcessOutput {
    tail: std::collections::VecDeque<String>,
    password_disclosed: bool,
}

impl ServingProcess {
    async fn start(directory: &Path, backend: &str, dsn: Option<&str>) -> Self {
        use tokio::io::AsyncBufReadExt;
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nebula-server"));
        command
            .env_clear()
            .env("NEBULA_ENV", "local")
            .env("API_EXECUTION_DB_PATH", "deployment.db")
            .env("NEBULA_CRED_DEV_KEY", "1")
            .env("NEBULA_WORKER_ARTIFACT_SET_DIGEST", "71".repeat(32))
            .env("SERVER_BIND_ADDRESS", "127.0.0.1:0")
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", "disabled")
            .env("NO_COLOR", "1")
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
            command.env("SYSTEMROOT", system_root);
        }
        if let Some(dsn) = dsn {
            command.env("DATABASE_URL", dsn);
        }
        if !backend.is_empty() {
            command.env("API_EXECUTION_BACKEND", backend);
        }
        let mut child = command.spawn().unwrap();
        let (sender, mut readiness) = tokio::sync::mpsc::channel(1);
        let output = std::sync::Arc::new(std::sync::Mutex::new(ProcessOutput::default()));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let streams: Vec<Box<dyn tokio::io::AsyncRead + Unpin + Send>> =
            vec![Box::new(stdout), Box::new(stderr)];
        let readers = streams
            .into_iter()
            .map(|stream| {
                let sender = sender.clone();
                let output = output.clone();
                tokio::spawn(async move {
                    let mut lines = tokio::io::BufReader::new(stream).lines();
                    while let Some(line) = lines.next_line().await.unwrap() {
                        if line.contains("starting transport") {
                            // Discover the actual OS-assigned port; never reserve then release one.
                            let port = line
                                .split_once("127.0.0.1:")
                                .unwrap()
                                .1
                                .chars()
                                .take_while(char::is_ascii_digit)
                                .collect::<String>();
                            let _ = sender.try_send(format!("http://127.0.0.1:{port}"));
                        }
                        // Drain both pipes continuously, retaining only a bounded diagnostic tail.
                        let mut output = output.lock().unwrap();
                        output.password_disclosed |= line.contains("password-canary");
                        if output.tail.len() == 32 {
                            output.tail.pop_front();
                        }
                        output.tail.push_back(line.chars().take(1024).collect());
                    }
                })
            })
            .collect();
        drop(sender);
        let mut server = Self {
            child,
            output,
            readers,
            base_url: String::new(),
        };
        if let Ok(Some(address)) =
            tokio::time::timeout(Duration::from_secs(30), readiness.recv()).await
        {
            server.base_url = address;
        } else {
            let diagnostics = server.output.lock().unwrap().tail.clone();
            server.stop().await;
            panic!("ordinary server failed to listen: {diagnostics:?}");
        }
        server
    }

    async fn stop(mut self) {
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
        for reader in self.readers {
            reader.await.unwrap();
        }
        assert!(
            !self.output.lock().unwrap().password_disclosed,
            "password reached server diagnostics"
        );
    }
}

async fn assert_serving_identity_survives_restart(
    directory: &Path,
    backend: &str,
    dsn: Option<&str>,
    password: &str,
) {
    use futures::FutureExt;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let mut identity = None;
    let mut session = None;
    let mut memberships = None;
    for _ in 0..2 {
        let server = ServingProcess::start(directory, backend, dsn).await;
        let result = std::panic::AssertUnwindSafe(async {
            if let Some(cookie) = &session {
                let response = client.get(format!("{}/api/v1/me/orgs", server.base_url))
                    .header(reqwest::header::COOKIE, cookie).send().await.unwrap();
                assert_eq!(response.status(), 200, "session must survive a process restart");
                assert_eq!(Some(response.json::<serde_json::Value>().await.unwrap()), memberships);
            }
            let response = client.post(format!("{}/api/v1/auth/login", server.base_url))
                .json(&serde_json::json!({"email":"owner@local.example", "password":password}))
                .send().await.unwrap();
            assert_eq!(response.status(), 200, "ordinary serving must find the enrolled owner without an auth selector");
            let cookie = response.headers().get_all(reqwest::header::SET_COOKIE).iter()
                .map(|header| header.to_str().unwrap().split(';').next().unwrap())
                .find(|cookie| cookie.starts_with("__Host-nebula-session="))
                .expect("HTTP login returns the session cookie").to_owned();
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["user"]["email"], "owner@local.example");
            assert_eq!(body["user"]["email_verified"], false);
            if let Some(previous) = &identity {
                assert_eq!(&body["user"]["user_id"], previous);
            }
            identity = Some(body["user"]["user_id"].clone());
            let response = client.get(format!("{}/api/v1/me/orgs", server.base_url))
                .header(reqwest::header::COOKIE, &cookie).send().await.unwrap();
            assert_eq!(response.status(), 200);
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["orgs"].as_array().unwrap().len(), 1);
            assert_eq!(body["orgs"][0]["role"], "owner");
            memberships = Some(body);
            session = Some(cookie);
            let rejected = client.post(format!("{}/api/v1/auth/login", server.base_url))
                .json(&serde_json::json!({"email":"owner@local.example", "password":"incorrect-password-canary"}))
                .send().await.unwrap();
            assert_eq!(rejected.status(), 401);
        }).catch_unwind().await;
        server.stop().await;
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

#[tokio::test]
async fn mandatory_worker_failure_stops_the_ordinary_server() {
    let directory = tempfile::tempdir().unwrap();
    let mut server = ServingProcess::start(directory.path(), "sqlite", None).await;
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(directory.path().join("deployment.db")),
    )
    .await
    .unwrap();
    // Fault injection into this private deployment: resource fanout loses its
    // durable relation. No test-only switch alters the production supervisor.
    sqlx::query("DROP TABLE resource_deliveries")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let result = tokio::time::timeout(Duration::from_secs(15), server.child.wait()).await;
    if result.is_err() {
        server.child.kill().await.unwrap();
    }
    for reader in server.readers {
        reader.await.unwrap();
    }
    let status = result
        .expect("mandatory worker failure must stop HTTP and the process")
        .unwrap();
    assert!(
        !status.success(),
        "mandatory owner failure must not report clean exit"
    );
}

#[tokio::test]
async fn default_local_server_executes_workflow_and_preserves_output_on_restart() {
    let directory = tempfile::tempdir().unwrap();
    let output = invoke(directory.path(), BEGIN, b"Owner-password-canary-2026").await;
    assert!(output.status.success(), "{output:?}");
    assert_execution_survives_restart(
        directory.path(),
        "",
        None,
        "Owner-password-canary-2026",
        ReopenMoment::Completed,
    )
    .await;
}

#[tokio::test]
async fn default_local_server_recovers_timer_after_process_crash() {
    let directory = tempfile::tempdir().unwrap();
    let output = invoke(directory.path(), BEGIN, b"Owner-password-canary-2026").await;
    assert!(output.status.success(), "{output:?}");
    assert_execution_survives_restart(
        directory.path(),
        "",
        None,
        "Owner-password-canary-2026",
        ReopenMoment::TimerParked,
    )
    .await;
}

#[derive(Clone, Copy)]
enum ReopenMoment {
    Completed,
    TimerParked,
}

async fn assert_execution_survives_restart(
    directory: &Path,
    backend: &str,
    dsn: Option<&str>,
    password: &str,
    reopen: ReopenMoment,
) {
    use futures::FutureExt;
    use serde_json::json;

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let mut execution_url = None;
    let mut session = None;
    for process in 0..2 {
        let server = ServingProcess::start(directory, backend, dsn).await;
        let result = std::panic::AssertUnwindSafe(async {
            let prefix = format!("{}/api/v1/orgs/personal/workspaces/default", server.base_url);
            if execution_url.is_none() {
                let response = client.post(format!("{}/api/v1/auth/login", server.base_url))
                    .json(&json!({"email":"owner@local.example", "password":password}))
                    .send().await.unwrap();
                assert_eq!(response.status(), 200);
                let cookie = response.headers().get_all(reqwest::header::SET_COOKIE).iter()
                    .map(|value| value.to_str().unwrap().split(';').next().unwrap())
                    .collect::<Vec<_>>().join("; ");
                let login: serde_json::Value = response.json().await.unwrap();
                let csrf = login["csrf_token"].as_str().unwrap();
                let (action, parameters) = match reopen {
                    ReopenMoment::Completed => ("core.set_fields", json!({
                        "data":{"type":"literal","value":{"answer":42}}
                    })),
                    ReopenMoment::TimerParked => ("core.delay", json!({
                        "data":{"type":"literal","value":{"answer":42}},
                        "mode":{"type":"literal","value":"for"},
                        "amount":{"type":"literal","value":2000},
                        "unit":{"type":"literal","value":"milliseconds"}
                    })),
                };
                let response = client.post(format!("{prefix}/workflows"))
                    .header(reqwest::header::COOKIE, &cookie).header("x-csrf-token", csrf)
                    .json(&json!({"name":format!("Local execution {}", uuid::Uuid::new_v4()), "definition": {
                        "nodes":[{"id":"step", "name":"Step", "plugin_key":"core", "action_key":action,
                            "parameters":parameters, "enabled":true}],
                        "connections":[]
                    }})).send().await.unwrap();
                let status = response.status();
                let body: serde_json::Value = response.json().await.unwrap();
                assert_eq!(status, 201, "{body}");
                let workflow = body["id"].as_str().unwrap();
                let response = client.post(format!("{prefix}/workflows/{workflow}/activate"))
                    .header(reqwest::header::COOKIE, &cookie).header("x-csrf-token", csrf)
                    .send().await.unwrap();
                let status = response.status();
                let body: serde_json::Value = response.json().await.unwrap();
                assert_eq!(status, 200, "{body}");
                let response = client.post(format!("{prefix}/workflows/{workflow}/executions"))
                    .header(reqwest::header::COOKIE, &cookie).header("x-csrf-token", csrf)
                    .json(&json!({"input":{}})).send().await.unwrap();
                let status = response.status();
                let body: serde_json::Value = response.json().await.unwrap();
                assert_eq!(status, 202, "{body}");
                execution_url = Some(format!("/executions/{}", body["id"].as_str().unwrap()));
                session = Some(cookie);
            }
            // The production timer scanner runs every 30 seconds. Reopening
            // before the timer is due must still recover on its next scan.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
            loop {
                let response = client.get(format!("{prefix}{}", execution_url.as_ref().unwrap()))
                    .header(reqwest::header::COOKIE, session.as_ref().unwrap()).send().await.unwrap();
                assert_eq!(response.status(), 200);
                let body: serde_json::Value = response.json().await.unwrap();
                if process == 0 && matches!(reopen, ReopenMoment::TimerParked) {
                    assert_ne!(body["status"], "completed", "must crash while the timer is parked");
                    if body["nodes"]["step"]["status"] == "waiting" {
                        break;
                    }
                }
                if body["status"] == "completed" {
                    assert_eq!(body["nodes"]["step"]["status"], "completed");
                    assert_eq!(body["nodes"]["step"]["output"], json!({"type":"inline","value":{"answer":42}}));
                    break;
                }
                assert!(tokio::time::Instant::now() < deadline, "ordinary server never completed accepted work: {body}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).catch_unwind().await;
        server.stop().await;
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

#[tokio::test]
async fn ordinary_sqlite_serving_uses_the_enrolled_owner_without_an_auth_selector() {
    let directory = tempfile::tempdir().unwrap();
    let output = invoke(directory.path(), BEGIN, b"Owner-password-canary-2026").await;
    assert!(output.status.success(), "{output:?}");
    assert_serving_identity_survives_restart(
        directory.path(),
        "sqlite",
        None,
        "Owner-password-canary-2026",
    )
    .await;
}

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
