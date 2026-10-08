//! Read/project-only credential composition for execution workers.
//!
//! This module opens the credential store on the worker's deployment database
//! (credentials live beside tenancy and executions), applies the key policy,
//! secures the concrete persistence adapter, and constructs the
//! credential-owned projection runtime. It deliberately has no management
//! command gateway, refresh coordinator, lease lifecycle, or reclaim sweep.

use std::sync::Arc;

use nebula_credential::{
    ApiKeyCredential, BasicAuthCredential, CredentialProjectionRuntime,
    CredentialProjectionRuntimeBuildError, CredentialRegistry, CredentialSlotResolver,
    DispatchError, DispatchOps, ErasedPendingStore, OAuth2Credential, SigningKeyCredential,
    StateSource, register_interactive_ops, register_refreshable_ops, register_runtime_ops,
};
#[cfg(feature = "postgres")]
use nebula_storage::credential::PgCredentialPersistence;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditSink, CredentialKeyring, CredentialKeyringError,
    CredentialStoreStartupError, EncryptionLayer, EnvKeyProvider, KeyProvider, ProviderError,
    SqliteCredentialPersistence,
};
use nebula_storage_port::{CredentialPersistence, CredentialPersistenceError};

const DEVELOPMENT_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

/// The pool onto the worker's deployment database, which holds its
/// credentials beside executions. The credential store shares this pool; no
/// second pool opens on the same database.
#[derive(Clone)]
pub enum DeploymentDatabase {
    /// The single-process SQLite database (`NEBULA_WORKER_DB_PATH`).
    Sqlite(sqlx::SqlitePool),
    /// The shared PostgreSQL database (`NEBULA_WORKER_DATABASE_URL`).
    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl std::fmt::Debug for DeploymentDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Pools carry their connect options; name only the backend.
        formatter.write_str(self.backend())
    }
}

impl DeploymentDatabase {
    const fn backend(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => "postgres",
        }
    }
}

/// Failure to compose the worker's credential projection runtime.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialProjectionCompositionError {
    /// The first-party credential registry rejected a static registration.
    #[error("credential projection registry registration failed")]
    Registry(#[from] nebula_credential::RegisterError),
    /// The first-party projection operation table is inconsistent.
    #[error("credential projection operation registration failed")]
    Dispatch(#[from] DispatchError),
    /// The process credential key could not be loaded.
    #[error("credential key provider initialization failed")]
    KeyProvider(#[source] ProviderError),
    /// The bounded decrypt-only keyring configuration was rejected.
    #[error("credential legacy master-key configuration is invalid")]
    Keyring(#[source] CredentialKeyringError),
    /// The deployment database could not open or migrate the credential store.
    #[error("credential store initialization failed")]
    Store(#[source] CredentialStoreStartupError),
    /// The credential-owned projection runtime rejected incomplete parts.
    #[error("credential projection runtime construction failed")]
    Projection(#[from] CredentialProjectionRuntimeBuildError),
}

/// Validated key configuration for the worker's first-party projection.
///
/// Prepare this before opening the deployment database. Key loading uses
/// `NEBULA_CRED_MASTER_KEY`, except when `NEBULA_CRED_DEV_KEY=1` explicitly
/// opts into the shared fixed development key. The same bounded decrypt-only
/// legacy keyring as the server is installed for projection.
pub struct CredentialProjectionConfig {
    keyring: CredentialKeyring,
}

impl CredentialProjectionConfig {
    /// Validate and retain the process key configuration without database I/O.
    ///
    /// # Errors
    ///
    /// Returns a typed error when current or decrypt-only keys are invalid.
    pub fn from_env() -> Result<Self, CredentialProjectionCompositionError> {
        Ok(Self {
            keyring: resolve_first_party_keyring()?,
        })
    }

    /// Compose the projection on the admitted deployment pool, consuming the
    /// validated key configuration without rereading the environment.
    ///
    /// # Errors
    ///
    /// Returns a typed store, registration or projection-construction failure.
    pub async fn compose(
        self,
        database: &DeploymentDatabase,
    ) -> Result<Arc<dyn CredentialSlotResolver>, CredentialProjectionCompositionError> {
        let projection = match database {
            DeploymentDatabase::Sqlite(pool) => {
                let store = SqliteCredentialPersistence::connect_pool(pool.clone())
                    .await
                    .map_err(CredentialProjectionCompositionError::Store)?;
                build_first_party_projection_with_keyring(store, self.keyring)?
            },
            #[cfg(feature = "postgres")]
            DeploymentDatabase::Postgres(pool) => {
                let store = PgCredentialPersistence::connect_pool(pool.clone())
                    .await
                    .map_err(CredentialProjectionCompositionError::Store)?;
                build_first_party_projection_with_keyring(store, self.keyring)?
            },
        };
        tracing::info!(
            backend = database.backend(),
            "credential projection store opened on the deployment database"
        );
        Ok(projection)
    }
}

/// Build the first-party projection runtime around one raw credential store.
///
/// This testable composition seam installs the same encryption, trace-audit,
/// registry, and operation layers used by [`CredentialProjectionConfig::compose`].
/// It returns only the object-safe read/project capability.
///
/// # Errors
///
/// Returns a typed registration or projection-construction failure.
pub fn build_first_party_projection<P>(
    raw_store: P,
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<dyn CredentialSlotResolver>, CredentialProjectionCompositionError>
where
    P: CredentialPersistence + 'static,
{
    let keyring = CredentialKeyring::from_config(key_provider, None, None)
        .map_err(CredentialProjectionCompositionError::Keyring)?;
    build_first_party_projection_with_keyring(raw_store, keyring)
}

fn build_first_party_projection_with_keyring<P>(
    raw_store: P,
    keyring: CredentialKeyring,
) -> Result<Arc<dyn CredentialSlotResolver>, CredentialProjectionCompositionError>
where
    P: CredentialPersistence + 'static,
{
    let registry = Arc::new(first_party_registry()?);
    let ops = Arc::new(first_party_ops()?);
    tracing::warn!(
        "credential audit sink is trace-only; durable audit persistence is scheduled for K3"
    );
    let encrypted: Arc<dyn CredentialPersistence> = Arc::new(EncryptionLayer::with_legacy_keys(
        raw_store,
        keyring.current(),
        keyring.credential_legacy(),
    ));
    let audit_sink: Arc<dyn AuditSink> = Arc::new(TracingAuditSink);
    let store: Arc<dyn CredentialPersistence> = Arc::new(AuditLayer::new(encrypted, audit_sink));
    let projection = CredentialProjectionRuntime::from_secure_parts(
        store,
        registry,
        ops,
        StateSource::LocalEncrypted,
    )?;
    Ok(Arc::new(projection))
}

fn resolve_first_party_keyring() -> Result<CredentialKeyring, CredentialProjectionCompositionError>
{
    let span = tracing::info_span!("credential_keyring_resolution", process = "worker");
    let _guard = span.enter();
    let current: Arc<dyn KeyProvider> = if std::env::var("NEBULA_CRED_DEV_KEY").as_deref()
        == Ok("1")
    {
        tracing::warn!(
            "security: NEBULA_CRED_DEV_KEY=1 - using a fixed development key; credential secrets are not securely encrypted"
        );
        Arc::new(
            EnvKeyProvider::from_base64(DEVELOPMENT_KEY_BASE64)
                .map_err(CredentialProjectionCompositionError::KeyProvider)?,
        )
    } else {
        Arc::new(
            EnvKeyProvider::from_env()
                .map_err(CredentialProjectionCompositionError::KeyProvider)?,
        )
    };
    CredentialKeyring::from_env(current).map_err(|error| {
        tracing::error!(
            reason = error.category(),
            "credential keyring configuration rejected"
        );
        CredentialProjectionCompositionError::Keyring(error)
    })
}

fn first_party_registry() -> Result<CredentialRegistry, nebula_credential::RegisterError> {
    let mut registry = CredentialRegistry::new();
    registry.register(ApiKeyCredential, "nebula-credential")?;
    registry.register(BasicAuthCredential, "nebula-credential")?;
    registry.register(OAuth2Credential, "nebula-credential")?;
    registry.register(SigningKeyCredential, "nebula-credential")?;
    Ok(registry)
}

fn first_party_ops() -> Result<DispatchOps<ErasedPendingStore>, DispatchError> {
    let mut ops = DispatchOps::new();
    register_runtime_ops::<ApiKeyCredential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<BasicAuthCredential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_interactive_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_refreshable_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<SigningKeyCredential, ErasedPendingStore>(&mut ops)?;
    Ok(ops)
}

struct TracingAuditSink;

impl AuditSink for TracingAuditSink {
    fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
        tracing::info!(
            target: "nebula.credential.audit",
            credential_id = %event.credential_id,
            operation = ?event.operation,
            result = ?event.result,
            "credential audit event"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::{CredentialId, credential_key};
    use nebula_credential::{
        Capabilities, CredentialSlotResolveError, SecretString, TenantScope, scheme::SecretToken,
        serde_secret,
    };
    use nebula_env::testing::EnvGuard;
    use nebula_storage::credential::{
        EncryptionLayer, EnvKeyProvider, KeyProvider, SqliteCredentialPersistence,
    };
    use nebula_storage_port::{
        CredentialCreate, CredentialMaterialTransition, CredentialOwner, CredentialPersistence,
        CredentialReplacement, CredentialSelector, Scope, SecretBytes, StoredCredential,
    };
    use tokio_util::sync::CancellationToken;

    use super::{
        CredentialProjectionConfig, DeploymentDatabase, build_first_party_projection,
        build_first_party_projection_with_keyring,
    };
    use nebula_storage::credential::CredentialKeyring;

    /// A credential store on a fresh in-memory deployment database, and that
    /// database's pool.
    async fn memory_store() -> (SqliteCredentialPersistence, sqlx::SqlitePool) {
        let pool = nebula_storage::sqlite::open_memory_deployment()
            .await
            .expect("in-memory deployment database");
        let store = SqliteCredentialPersistence::connect_pool(pool.clone())
            .await
            .expect("ready in-memory credential store");
        (store, pool)
    }

    /// Provision `scope`'s tenant on the deployment `pool`: a credential
    /// belongs to a live workspace in the deployment database (migration 0070).
    async fn provision(pool: &sqlx::SqlitePool, scope: &Scope) {
        use nebula_storage_port::dto::{
            PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningRequest,
        };
        use nebula_storage_port::store::TenantProvisioningStore as _;

        let request = TenantProvisioningRequest::new(
            TenantOrgCreate::new(
                scope.org_id.clone(),
                scope.org_id.clone(),
                "Fixture".into(),
                "fixture".into(),
                "free".into(),
                None,
                serde_json::json!({}),
            )
            .expect("org values"),
            TenantDefaultWorkspaceCreate::new(
                scope.workspace_id.clone(),
                "default".into(),
                "Default".into(),
                None,
                "fixture".into(),
                serde_json::json!({}),
            )
            .expect("workspace values"),
            PrincipalKind::User,
            "fixture-owner".into(),
            None,
        )
        .expect("provisioning request");
        nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone())
            .provision_tenant(request)
            .await
            .expect("provision the fixture tenant");
    }

    const TEST_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
    const OLD_KEY_BASE64: &str = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE=";

    fn key_provider(encoded: &str) -> Arc<dyn KeyProvider> {
        Arc::new(EnvKeyProvider::from_base64(encoded).expect("valid fixed test key"))
    }

    fn api_key_data(secret: &str) -> SecretBytes {
        let token = SecretToken::new(SecretString::new(secret));
        let data = serde_secret::expose_for_serialization(|| serde_json::to_vec(&token))
            .expect("test credential serializes");
        SecretBytes::new(data)
    }

    fn api_key_create(secret: &str) -> CredentialCreate {
        CredentialCreate::new(
            "api_key".to_owned(),
            api_key_data(secret),
            "secret_token".to_owned(),
            1,
            None,
            None,
            false,
            serde_json::Map::new(),
        )
    }

    #[test]
    fn first_party_projection_registers_oauth2_with_its_advertised_operations() {
        let registry = super::first_party_registry().expect("first-party registry is valid");
        let ops = super::first_party_ops().expect("first-party operation table is valid");
        let key = credential_key!("oauth2");

        let advertised = registry
            .capabilities_of(key.as_str())
            .expect("OAuth2 is present in the worker registry");
        let dispatched = ops.capabilities_of(key.as_str());
        assert!(advertised.contains(Capabilities::INTERACTIVE));
        assert!(advertised.contains(Capabilities::REFRESHABLE));
        assert_eq!(advertised.difference(dispatched), Capabilities::empty());
    }

    #[tokio::test]
    async fn rolling_keyring_projects_legacy_server_rows_and_current_only_fails_closed() {
        let (raw_store, pool) = memory_store().await;
        let raw_store = Arc::new(raw_store);
        let scope = TenantScope::new("org-rotation", "workspace-rotation");
        let credential_id = CredentialId::new();
        provision(&pool, &Scope::new("workspace-rotation", "org-rotation")).await;
        let owner = CredentialOwner::from_scope(&Scope::new("workspace-rotation", "org-rotation"));
        let selector = CredentialSelector::new(owner, credential_id);

        // Model a server replica that still writes with the old current key.
        let old_writer = EncryptionLayer::new(Arc::clone(&raw_store), key_provider(OLD_KEY_BASE64));
        old_writer
            .create(&selector, api_key_create("rolling-secret"))
            .await
            .expect("old-key server write succeeds");
        let version_before_projection = match raw_store
            .get(&selector)
            .await
            .expect("legacy row exists before projection")
        {
            StoredCredential::Live(row) => row.version(),
            StoredCredential::Tombstoned(_) => panic!("rotation fixture must remain live"),
        };

        // A bridge worker must project the old row through its explicit
        // decrypt-only key while continuing to use the new key as current.
        let bridge_keyring = CredentialKeyring::from_config(
            key_provider(TEST_KEY_BASE64),
            Some(OLD_KEY_BASE64),
            None,
        )
        .expect("rolling keyring is valid");
        let bridge =
            build_first_party_projection_with_keyring(Arc::clone(&raw_store), bridge_keyring)
                .expect("bridge projection composes");
        let guard = bridge
            .resolve_slot(
                &scope,
                credential_id,
                credential_key!("api_key"),
                Capabilities::empty(),
                CancellationToken::new(),
            )
            .await
            .expect("bridge worker projects an old-key row");
        let token = guard
            .into_typed::<SecretToken>()
            .expect("API key projects as a secret token");
        assert_eq!(token.token().expose_secret(), "rolling-secret");
        let version_after_projection = match raw_store
            .get(&selector)
            .await
            .expect("legacy row remains after projection")
        {
            StoredCredential::Live(row) => row.version(),
            StoredCredential::Tombstoned(_) => panic!("rotation fixture must remain live"),
        };
        assert_eq!(
            version_after_projection, version_before_projection,
            "projection must not rewrite or advance a legacy-key row"
        );

        let current_only =
            build_first_party_projection(Arc::clone(&raw_store), key_provider(TEST_KEY_BASE64))
                .expect("current-only projection composes");
        let error = current_only
            .resolve_slot(
                &scope,
                credential_id,
                credential_key!("api_key"),
                Capabilities::empty(),
                CancellationToken::new(),
            )
            .await
            .expect_err("removing a still-required legacy key must fail closed");
        assert_eq!(error, CredentialSlotResolveError::InvalidState);

        // A supported server mutation reads with the rolling keyring and
        // writes the replacement with the new current key. It must advance
        // the row exactly once; projection reads remain side-effect free.
        let rotating_store = EncryptionLayer::with_legacy_keys(
            Arc::clone(&raw_store),
            key_provider(TEST_KEY_BASE64),
            CredentialKeyring::from_config(
                key_provider(TEST_KEY_BASE64),
                Some(OLD_KEY_BASE64),
                None,
            )
            .expect("rolling keyring is valid")
            .credential_legacy(),
        );
        let before = match rotating_store
            .get(&selector)
            .await
            .expect("server reads the old-key row")
        {
            StoredCredential::Live(row) => row,
            StoredCredential::Tombstoned(_) => panic!("rotation fixture must remain live"),
        };
        let commit = rotating_store
            .replace(
                &selector,
                CredentialReplacement::new(
                    before.version(),
                    None,
                    false,
                    serde_json::Map::new(),
                    CredentialMaterialTransition::advance(
                        nebula_storage_port::MaterialUpdate::Replace(
                            nebula_storage_port::CredentialMaterial::new(
                                api_key_data("rolling-secret"),
                                "secret_token".to_owned(),
                                1,
                                None,
                            ),
                        ),
                    ),
                ),
            )
            .await
            .expect("new-key server mutation rewrites the row");
        assert_eq!(commit.version().get(), before.version().get() + 1);

        let current_only =
            build_first_party_projection(Arc::clone(&raw_store), key_provider(TEST_KEY_BASE64))
                .expect("current-only projection composes after rotation");
        let guard = current_only
            .resolve_slot(
                &scope,
                credential_id,
                credential_key!("api_key"),
                Capabilities::empty(),
                CancellationToken::new(),
            )
            .await
            .expect("current-only worker projects the rewritten row");
        let token = guard
            .into_typed::<SecretToken>()
            .expect("API key projects as a secret token");
        assert_eq!(token.token().expose_secret(), "rolling-secret");
        let after_projection = raw_store
            .get(&selector)
            .await
            .expect("raw rewritten row remains readable");
        let StoredCredential::Live(after_projection) = after_projection else {
            panic!("rotation fixture must remain live");
        };
        assert_eq!(after_projection.version(), commit.version());
    }

    #[tokio::test]
    async fn production_composition_carries_env_legacy_key_to_projection() {
        let temp = tempfile::tempdir().expect("temporary credential directory");
        let database_path = temp.path().join("worker.db");
        // The worker's deployment pool, as `build_stores` opens it.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&database_path)
                    .create_if_missing(true),
            )
            .await
            .expect("worker deployment pool");
        let raw_store = Arc::new(
            SqliteCredentialPersistence::connect_pool(pool.clone())
                .await
                .expect("ready file-backed credential store"),
        );
        let scope = TenantScope::new("org-env-rotation", "workspace-env-rotation");
        provision(
            &pool,
            &Scope::new("workspace-env-rotation", "org-env-rotation"),
        )
        .await;
        let credential_id = CredentialId::new();
        let owner =
            CredentialOwner::from_scope(&Scope::new("workspace-env-rotation", "org-env-rotation"));
        let selector = CredentialSelector::new(owner, credential_id);
        let old_writer = EncryptionLayer::new(Arc::clone(&raw_store), key_provider(OLD_KEY_BASE64));
        old_writer
            .create(&selector, api_key_create("env-rolling-secret"))
            .await
            .expect("old-key server write succeeds");
        drop(old_writer);
        drop(raw_store);

        let mut env = EnvGuard::acquire();
        env.set("NEBULA_CRED_MASTER_KEY", TEST_KEY_BASE64);
        env.set("NEBULA_CRED_LEGACY_MASTER_KEYS", OLD_KEY_BASE64);
        env.remove("NEBULA_CRED_LEGACY_EMPTY_ID_MASTER_KEY");
        env.remove("NEBULA_CRED_DEV_KEY");

        let config = CredentialProjectionConfig::from_env()
            .expect("production configuration accepts the legacy key");
        env.remove("NEBULA_CRED_MASTER_KEY");
        env.remove("NEBULA_CRED_LEGACY_MASTER_KEYS");
        let projection = config
            .compose(&DeploymentDatabase::Sqlite(pool))
            .await
            .expect("production composition accepts the configured legacy key");
        let guard = projection
            .resolve_slot(
                &scope,
                credential_id,
                credential_key!("api_key"),
                Capabilities::empty(),
                CancellationToken::new(),
            )
            .await
            .expect("production projection decrypts the legacy server row");
        let token = guard
            .into_typed::<SecretToken>()
            .expect("API key projects as a secret token");
        assert_eq!(token.token().expose_secret(), "env-rolling-secret");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn projection_composition_spawns_no_lifecycle_owner_tasks() {
        let raw_store = Arc::new(memory_store().await.0);
        let key_provider: Arc<dyn KeyProvider> =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_BASE64).expect("valid fixed test key"));
        // Keep both snapshots in one uninterrupted current-thread poll so SQLite
        // housekeeping cannot finish while the synchronous constructor is measured.
        let alive_before = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();

        let projection = build_first_party_projection(raw_store, key_provider)
            .expect("credential projection runtime composes");

        assert_eq!(
            tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks(),
            alive_before,
            "read/project-only composition must not spawn refresh, lease, or reclaim owners"
        );
        drop(projection);
    }

    #[tokio::test]
    async fn projection_drop_releases_store_and_key_without_detached_owners() {
        let raw_store = Arc::new(memory_store().await.0);
        let raw_store_lifecycle = Arc::downgrade(&raw_store);
        let key_provider =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_BASE64).expect("valid fixed test key"));
        let key_provider_lifecycle = Arc::downgrade(&key_provider);
        let key: Arc<dyn KeyProvider> = key_provider.clone();
        let projection = build_first_party_projection(raw_store.clone(), key)
            .expect("credential projection runtime composes");

        drop(raw_store);
        drop(key_provider);
        assert!(raw_store_lifecycle.upgrade().is_some());
        assert!(key_provider_lifecycle.upgrade().is_some());

        drop(projection);
        assert!(
            raw_store_lifecycle.upgrade().is_none(),
            "projection drop must not leave a refresh or reclaim owner holding the store"
        );
        assert!(
            key_provider_lifecycle.upgrade().is_none(),
            "projection drop must not leave a lifecycle owner holding the key provider"
        );
    }

    #[tokio::test]
    async fn deployment_database_debug_never_echoes_its_locator() {
        let pool = sqlx::SqlitePool::connect_lazy("sqlite://var/lib/tenant-private/worker.db")
            .expect("lazy SQLite pool");
        assert_eq!(format!("{:?}", DeploymentDatabase::Sqlite(pool)), "sqlite");
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
}
