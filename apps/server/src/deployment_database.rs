//! The admitted database shared by the server's persistence adapters.

use nebula_api::{
    ApiConfig,
    config::{ExecutionBackendKind, ExecutionStoreConfig, IdempotencyBackend},
};

use crate::compose::TransportInitError;

/// Pool ownership belongs to deployment composition, not any one aggregate.
/// Clones retain the same pool and its shutdown state.
#[derive(Clone)]
pub(crate) enum DeploymentDatabase {
    /// SQLite tenancy/credentials beside internal in-memory execution adapters.
    Memory(sqlx::SqlitePool),
    /// Durable single-process deployment.
    Sqlite(nebula_storage::sqlite::DeploymentPool),
    /// Shared deployment database.
    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl DeploymentDatabase {
    /// Open and admit the one database used by server-owned persistence.
    /// Operator commands reuse this stage without constructing HTTP or runtime
    /// dependencies. An explicit PostgreSQL DSN takes precedence over the environment.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn open(
        config: &ExecutionStoreConfig,
        postgres_dsn_override: Option<&str>,
    ) -> Result<Self, TransportInitError> {
        match config.backend {
            ExecutionBackendKind::Memory => nebula_storage::sqlite::open_memory_deployment()
                .await
                .map(Self::Memory)
                .map_err(|error| database_failure("memory", "open", &error)),
            ExecutionBackendKind::Sqlite => {
                use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};

                let options = SqliteConnectOptions::new()
                    .filename(&config.db_path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal);
                let deployment = nebula_storage::sqlite::DeploymentPool::connect(options)
                    .await
                    .map_err(|error| database_failure("sqlite", "open", &error))?;
                if let Err(error) = nebula_storage::sqlite::init_schema(deployment.pool()).await {
                    deployment.pool().close().await;
                    return Err(database_failure("sqlite", "schema admission", &error));
                }
                tracing::info!(backend = "sqlite", "deployment database admitted");
                Ok(Self::Sqlite(deployment))
            },
            ExecutionBackendKind::Postgres => Self::open_postgres(postgres_dsn_override).await,
        }
    }

    #[cfg(feature = "postgres")]
    async fn open_postgres(dsn_override: Option<&str>) -> Result<Self, TransportInitError> {
        let environment_dsn;
        let dsn = if let Some(explicit) = dsn_override {
            explicit
        } else {
            environment_dsn = std::env::var("DATABASE_URL").map_err(|_| {
                TransportInitError::ExecutionBackendUnavailable {
                    requested: "postgres",
                    requirement: "DATABASE_URL must be set when API_EXECUTION_BACKEND=postgres",
                }
            })?;
            &environment_dsn
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect(dsn)
            .await
            .map_err(|error| {
                let category = match error {
                    sqlx::Error::Io(error) => format!("io:{:?}", error.kind()),
                    sqlx::Error::Tls(_) => "tls".into(),
                    sqlx::Error::Configuration(_) => "configuration".into(),
                    sqlx::Error::Database(_) => "database".into(),
                    sqlx::Error::PoolTimedOut => "timeout".into(),
                    _ => "connection".into(),
                };
                TransportInitError::ExecutionDatabase(format!(
                    "postgres: deployment database connection failed ({category})"
                ))
            })?;
        if let Err(error) = nebula_storage::postgres::init_schema(&pool).await {
            pool.close().await;
            return Err(database_failure("postgres", "schema admission", &error));
        }
        tracing::info!(backend = "postgres", "deployment database admitted");
        Ok(Self::Postgres(pool))
    }

    #[cfg(not(feature = "postgres"))]
    async fn open_postgres(_dsn_override: Option<&str>) -> Result<Self, TransportInitError> {
        Err(TransportInitError::ExecutionBackendUnavailable {
            requested: "postgres",
            requirement: "build with the `postgres` cargo feature to link PostgreSQL storage",
        })
    }

    pub(crate) const fn backend(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Sqlite(_) => "sqlite",
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => "postgres",
        }
    }
}

fn database_failure(
    backend: &'static str,
    stage: &'static str,
    error: &nebula_storage_port::StorageError,
) -> TransportInitError {
    let category = crate::storage_diagnostics::storage_error_category(error);
    tracing::error!(
        backend,
        stage,
        error.category = category,
        "deployment database setup failed"
    );
    TransportInitError::ExecutionDatabase(format!("{backend}: {stage} failed ({category})"))
}

impl std::fmt::Debug for DeploymentDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Locators can contain credentials or private filesystem paths.
        formatter.write_str(self.backend())
    }
}

/// Reject incompatible HTTP replay storage before creating the deployment database.
#[tracing::instrument(skip_all)]
pub(crate) fn validate_idempotency_backend(config: &ApiConfig) -> Result<(), TransportInitError> {
    if config.execution.backend != ExecutionBackendKind::Postgres
        && config.idempotency.backend == IdempotencyBackend::Postgres
    {
        return Err(TransportInitError::IdempotencyBackendUnavailable {
            requested: "postgres",
            requirement: "API_EXECUTION_BACKEND=postgres so HTTP replay shares the deployment database",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::DeploymentDatabase;

    #[tokio::test]
    async fn database_opener_rejects_foreign_schema_without_disclosing_locator() {
        use nebula_api::config::{ExecutionBackendKind, ExecutionStoreConfig};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private-locator-canary.db");
        let foreign = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::query("CREATE TABLE foreign_owner (id INTEGER PRIMARY KEY)")
            .execute(&foreign)
            .await
            .unwrap();
        foreign.close().await;

        let config = ExecutionStoreConfig {
            backend: ExecutionBackendKind::Sqlite,
            db_path: path.to_str().unwrap().to_owned(),
        };
        let error = DeploymentDatabase::open(&config, None).await.unwrap_err();
        assert!(matches!(
            error,
            crate::compose::TransportInitError::ExecutionDatabase(_)
        ));
        let diagnostic = format!("{error} {error:?}");
        assert!(!diagnostic.contains("private-locator-canary"));
        assert!(!diagnostic.contains("foreign_owner"));
        assert!(diagnostic.contains("schema admission"));
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn database_opener_does_not_disclose_rejected_postgres_dsn() {
        use nebula_api::config::{ExecutionBackendKind, ExecutionStoreConfig};

        let config = ExecutionStoreConfig {
            backend: ExecutionBackendKind::Postgres,
            ..ExecutionStoreConfig::default()
        };
        let error = DeploymentDatabase::open(
            &config,
            Some("postgres://operator:private-password-canary@localhost:invalid-port/nebula"),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            crate::compose::TransportInitError::ExecutionDatabase(_)
        ));
        assert!(!format!("{error} {error:?}").contains("private-password-canary"));
    }

    #[cfg(not(feature = "postgres"))]
    #[tokio::test]
    async fn database_opener_rejects_unlinked_postgres_backend() {
        use nebula_api::config::{ExecutionBackendKind, ExecutionStoreConfig};

        let config = ExecutionStoreConfig {
            backend: ExecutionBackendKind::Postgres,
            ..ExecutionStoreConfig::default()
        };
        assert!(matches!(
            DeploymentDatabase::open(&config, Some("never-opened")).await,
            Err(
                crate::compose::TransportInitError::ExecutionBackendUnavailable {
                    requested: "postgres",
                    ..
                }
            )
        ));
    }

    #[tokio::test]
    async fn sqlite_auth_lifecycle_reopens_and_observes_deployment_pool_shutdown() {
        use nebula_api::{
            config::OAuthProvidersConfig,
            domain::auth::backend::{
                AuthError, CreatePatParams, PasswordOutcome, SecretString, SignupRequest, mfa,
            },
            ports::email::EchoSink,
        };
        use nebula_storage::credential::EnvKeyProvider;
        use nebula_storage::sqlite::DeploymentPool;
        use sqlx::sqlite::SqliteConnectOptions;
        use std::sync::Arc;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.db");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true);
        let deployment = DeploymentPool::connect(options.clone()).await.unwrap();
        let pool = deployment.pool().clone();
        nebula_storage::sqlite::init_schema(&pool).await.unwrap();
        let sink = Arc::new(EchoSink::default());
        let key = Arc::new(
            EnvKeyProvider::from_base64("QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=").unwrap(),
        );
        let auth = crate::compose::build_auth_backend(
            &DeploymentDatabase::Sqlite(deployment),
            OAuthProvidersConfig::default(),
            sink.clone(),
            None,
            key.clone(),
            Vec::new(),
        )
        .await
        .unwrap();
        let profile = auth
            .register_user(SignupRequest {
                email: "owner@deployment.example".into(),
                password: SecretString::new("Strong-Passw0rd-2026".into()),
                display_name: "Local owner".into(),
            })
            .await
            .unwrap();
        assert!(!profile.email_verified);
        let verification = sink.drain().pop().unwrap().body;
        auth.verify_email(&verification).await.unwrap();
        assert!(matches!(
            auth.verify_email(&verification).await,
            Err(AuthError::InvalidToken)
        ));
        assert!(matches!(
            auth.authenticate_password(&profile.email, "Strong-Passw0rd-2026", None)
                .await
                .unwrap(),
            PasswordOutcome::Authenticated(_)
        ));
        let session = auth.create_session(&profile.user_id).await.unwrap();
        let enrollment = auth.start_mfa_enrollment(&profile.user_id).await.unwrap();
        auth.confirm_mfa_enrollment(
            &profile.user_id,
            &mfa::current_code(&enrollment.secret_base32).unwrap(),
        )
        .await
        .unwrap();
        let minted = auth
            .create_pat(
                &profile.user_id,
                CreatePatParams {
                    name: "automation".into(),
                    scopes: vec!["full_access".into()],
                    ttl_seconds: None,
                },
            )
            .await
            .unwrap();
        pool.close().await;
        assert!(matches!(
            auth.get_user_profile(&profile.user_id).await,
            Err(AuthError::Internal(_))
        ));
        drop(auth);

        let deployment = DeploymentPool::connect(options).await.unwrap();
        let pool = deployment.pool().clone();
        nebula_storage::sqlite::init_schema(&pool).await.unwrap();
        let auth = crate::compose::build_auth_backend(
            &DeploymentDatabase::Sqlite(deployment),
            OAuthProvidersConfig::default(),
            sink.clone(),
            None,
            key,
            Vec::new(),
        )
        .await
        .unwrap();
        assert!(
            auth.get_user_profile(&profile.user_id)
                .await
                .unwrap()
                .mfa_enabled
        );
        let PasswordOutcome::MfaRequired { challenge_token } = auth
            .authenticate_password(&profile.email, "Strong-Passw0rd-2026", None)
            .await
            .unwrap()
        else {
            panic!("MFA persisted")
        };
        let code = mfa::current_code(&enrollment.secret_base32).unwrap();
        assert_eq!(
            auth.verify_mfa(&challenge_token, &code)
                .await
                .unwrap()
                .user_id,
            profile.user_id
        );
        assert!(matches!(
            auth.verify_mfa(&challenge_token, &code).await,
            Err(AuthError::InvalidToken)
        ));
        assert!(auth.lookup_pat(&minted.plaintext).await.unwrap().is_some());
        auth.revoke_pat(&profile.user_id, &minted.record.id)
            .await
            .unwrap();
        assert!(auth.lookup_pat(&minted.plaintext).await.unwrap().is_none());
        use nebula_storage::auth::SessionRepo;
        let sessions = nebula_storage::auth::sqlite::SqliteSessionRepo::new(pool.clone());
        assert!(sessions.get(session.id.as_bytes()).await.unwrap().is_some());
        auth.revoke_session(&session.id).await.unwrap();
        assert!(sessions.get(session.id.as_bytes()).await.unwrap().is_none());
        auth.request_password_reset(&profile.email).await.unwrap();
        let reset = sink.drain().pop().unwrap().body;
        auth.complete_password_reset(&reset, "Changed-Passw0rd-2026")
            .await
            .unwrap();
        assert!(
            auth.authenticate_password(&profile.email, "Strong-Passw0rd-2026", None)
                .await
                .is_err()
        );
        assert!(matches!(
            auth.authenticate_password(&profile.email, "Changed-Passw0rd-2026", None)
                .await
                .unwrap(),
            PasswordOutcome::MfaRequired { .. }
        ));
        let live_session = auth.create_session(&profile.user_id).await.unwrap();
        let live_pat = auth
            .create_pat(
                &profile.user_id,
                CreatePatParams {
                    name: "archive probe".into(),
                    scopes: vec!["full_access".into()],
                    ttl_seconds: None,
                },
            )
            .await
            .unwrap();
        sqlx::query("UPDATE users SET deleted_at = 1 WHERE email = ?")
            .bind(&profile.email)
            .execute(&pool)
            .await
            .unwrap();
        let session_principal = auth
            .get_principal_by_session(&live_session.id)
            .await
            .unwrap();
        let pat_principal = auth.lookup_pat(&live_pat.plaintext).await.unwrap();
        assert!(
            session_principal.is_none() && pat_principal.is_none(),
            "archived user must lose both session and PAT authority"
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn sqlite_auth_is_not_exposed_with_rejected_identity_material() {
        use nebula_api::{config::OAuthProvidersConfig, ports::email::EchoSink};
        use nebula_storage::credential::EnvKeyProvider;
        use std::sync::Arc;
        let deployment = nebula_storage::sqlite::DeploymentPool::connect(
            sqlx::sqlite::SqliteConnectOptions::new().in_memory(true),
        )
        .await
        .unwrap();
        let pool = deployment.pool().clone();
        nebula_storage::sqlite::init_schema(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (id, email, display_name, created_at, mfa_enabled, mfa_secret_envelope)
            VALUES (?, 'owner@example.test', 'Owner', 0, 1, ?)")
            .bind([1_u8; 16].as_slice()).bind(b"INVALID_FACTOR_CANARY-6a77".as_slice()).execute(&pool).await.unwrap();
        let result = crate::compose::build_auth_backend(
            &DeploymentDatabase::Sqlite(deployment),
            OAuthProvidersConfig::default(),
            Arc::new(EchoSink::default()),
            None,
            Arc::new(
                EnvKeyProvider::from_base64("QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=")
                    .unwrap(),
            ),
            Vec::new(),
        )
        .await;
        match result {
            Ok(_) => panic!("invalid identity material must stop startup"),
            Err(error) => assert!(!error.to_string().contains("INVALID_FACTOR_CANARY")),
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn deployment_database_debug_never_echoes_its_locator() {
        let path = "sqlite://var/lib/tenant-private/nebula.db";
        let pool = sqlx::SqlitePool::connect_lazy(path).expect("lazy SQLite pool");
        assert_eq!(format!("{:?}", DeploymentDatabase::Memory(pool)), "memory");
        let directory = tempfile::tempdir().unwrap();
        let deployment = nebula_storage::sqlite::DeploymentPool::connect(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(directory.path().join("tenant-private.db"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
        assert_eq!(format!("{deployment:?}"), "SqliteDeploymentPool");
        assert_eq!(
            format!("{:?}", DeploymentDatabase::Sqlite(deployment.clone())),
            "sqlite"
        );
        deployment.pool().close().await;
        #[cfg(feature = "postgres")]
        {
            let dsn = "postgres://operator:super-secret@example.invalid/tenant-private";
            let pool = sqlx::PgPool::connect_lazy(dsn).expect("lazy PostgreSQL pool");
            assert_eq!(
                format!("{:?}", DeploymentDatabase::Postgres(pool)),
                "postgres"
            );
        }
    }

    /// Exercise the production composition and domain operations, including
    /// their shared shutdown authority, on a private admitted schema.
    #[cfg(feature = "postgres")]
    #[tokio::test]
    #[ignore = "requires live PostgreSQL; set DATABASE_URL and run explicitly"]
    async fn postgres_auth_and_replay_share_deployment_storage_and_shutdown() {
        use std::{str::FromStr, sync::Arc, time::Duration};

        use axum::http::{HeaderMap, StatusCode};
        use futures::FutureExt;
        use nebula_api::{
            ApiConfig,
            config::IdempotencyBackend,
            domain::auth::backend::{AuthError, SecretString, SignupRequest},
            middleware::idempotency::{CachedResponse, IdempotencyStoreError},
            ports::email::EchoSink,
        };
        use nebula_storage::credential::EnvKeyProvider;
        use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

        let dsn = std::env::var("DATABASE_URL").expect("SETUP: DATABASE_URL required");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&dsn)
            .await
            .expect("SETUP: connect PostgreSQL");
        let schema = format!("deployment_pool_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("SETUP: private schema");

        let result = std::panic::AssertUnwindSafe(async {
            let options = PgConnectOptions::from_str(&dsn)
                .expect("SETUP: PostgreSQL options")
                .options([("search_path", schema.as_str())]);
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(2))
                .connect_with(options)
                .await
                .expect("SETUP: deployment pool");
            nebula_storage::postgres::init_schema(&pool)
                .await
                .expect("SETUP: admitted deployment schema");
            let database = DeploymentDatabase::Postgres(pool.clone());
            let mut config = ApiConfig::for_test();
            config.idempotency.backend = IdempotencyBackend::Postgres;
            let key_provider = Arc::new(
                EnvKeyProvider::from_base64("QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=")
                    .expect("test key"),
            );
            let auth = crate::compose::build_auth_backend(
                &database,
                std::mem::take(&mut config.auth.oauth),
                Arc::new(EchoSink::default()),
                None,
                key_provider,
                Vec::new(),
            )
            .await
            .expect("compose identity on deployment pool");
            let replay = crate::compose::build_idempotency_store(&config, &database)
                .expect("compose HTTP replay on deployment pool");

            let user = auth
                .register_user(SignupRequest {
                    email: "owner@deployment.example".into(),
                    password: SecretString::new("Strong-Passw0rd-2026".into()),
                    display_name: "Deployment owner".into(),
                })
                .await
                .expect("register through composed auth backend");
            let stored_email: String = sqlx::query_scalar("SELECT email FROM users")
                .fetch_one(&pool)
                .await
                .expect("identity was written in deployment schema");
            assert_eq!(stored_email, user.email);
            assert_eq!(
                auth.get_user_profile(&user.user_id)
                    .await
                    .expect("read profile")
                    .email,
                user.email
            );

            let response = Arc::new(CachedResponse {
                status: StatusCode::CREATED,
                headers: HeaderMap::new(),
                body: b"deployment-receipt".to_vec(),
                request_fingerprint: [7; 32],
            });
            replay
                .put("request-key".into(), response.clone())
                .await
                .expect("persist replay");
            let stored_body: Vec<u8> = sqlx::query_scalar("SELECT body FROM api_idempotency_dedup")
                .fetch_one(&pool)
                .await
                .expect("replay was written in deployment schema");
            assert_eq!(stored_body, response.body);
            assert_eq!(
                replay
                    .get("request-key")
                    .await
                    .expect("read replay")
                    .expect("cached")
                    .body,
                response.body
            );

            // Sharing a database URL is insufficient: both adapters must also
            // share this pool's lifetime rather than leave independent pools open.
            pool.close().await;
            assert!(matches!(
                auth.get_user_profile(&user.user_id).await,
                Err(AuthError::Internal(_))
            ));
            assert!(matches!(
                replay.get("request-key").await,
                Err(IdempotencyStoreError::Backend(_))
            ));
        })
        .catch_unwind()
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin)
            .await;
        admin.close().await;
        cleanup.expect("CLEANUP: remove private schema");
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
