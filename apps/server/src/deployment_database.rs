//! The admitted database shared by the server's persistence adapters.

use nebula_api::{
    ApiConfig,
    config::{AuthBackendKind, ExecutionBackendKind, IdempotencyBackend},
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
    pub(crate) const fn backend(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Sqlite(_) => "sqlite",
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => "postgres",
        }
    }
}

impl std::fmt::Debug for DeploymentDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Locators can contain credentials or private filesystem paths.
        formatter.write_str(self.backend())
    }
}

/// Reject split persistence authorities before creating deployment storage.
#[tracing::instrument(skip_all)]
pub(crate) fn validate_backend_selection(config: &ApiConfig) -> Result<(), TransportInitError> {
    if config.auth.backend == AuthBackendKind::Sqlite
        && config.execution.backend != ExecutionBackendKind::Sqlite
    {
        return Err(TransportInitError::AuthBackendUnavailable {
            requested: "sqlite",
            requirement: "API_EXECUTION_BACKEND=sqlite so identity shares the deployment database",
        });
    }
    if config.execution.backend != ExecutionBackendKind::Postgres {
        if config.auth.backend == AuthBackendKind::Postgres {
            return Err(TransportInitError::AuthBackendUnavailable {
                requested: "postgres",
                requirement: "API_EXECUTION_BACKEND=postgres so identity shares the deployment database",
            });
        }
        if config.idempotency.backend == IdempotencyBackend::Postgres {
            return Err(TransportInitError::IdempotencyBackendUnavailable {
                requested: "postgres",
                requirement: "API_EXECUTION_BACKEND=postgres so HTTP replay shares the deployment database",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::DeploymentDatabase;

    #[tokio::test]
    async fn sqlite_auth_lifecycle_reopens_and_observes_deployment_pool_shutdown() {
        use nebula_api::{
            config::{AuthBackendKind, OAuthProvidersConfig},
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
            AuthBackendKind::Sqlite,
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
            AuthBackendKind::Sqlite,
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
        use nebula_api::{
            config::{AuthBackendKind, OAuthProvidersConfig},
            ports::email::EchoSink,
        };
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
            AuthBackendKind::Sqlite,
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
            config::{AuthBackendKind, IdempotencyBackend},
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
            config.auth.backend = AuthBackendKind::Postgres;
            config.idempotency.backend = IdempotencyBackend::Postgres;
            let key_provider = Arc::new(
                EnvKeyProvider::from_base64("QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=")
                    .expect("test key"),
            );
            let auth = crate::compose::build_auth_backend(
                &database,
                config.auth.backend.clone(),
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
