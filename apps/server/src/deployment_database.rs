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
    Sqlite(sqlx::SqlitePool),
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
    async fn deployment_database_debug_never_echoes_its_locator() {
        let path = "sqlite://var/lib/tenant-private/nebula.db";
        let pool = sqlx::SqlitePool::connect_lazy(path).expect("lazy SQLite pool");
        assert_eq!(
            format!("{:?}", DeploymentDatabase::Sqlite(pool.clone())),
            "sqlite"
        );
        assert_eq!(format!("{:?}", DeploymentDatabase::Memory(pool)), "memory");
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
