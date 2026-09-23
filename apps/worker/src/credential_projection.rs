//! Read/project-only credential composition for execution workers.
//!
//! This module selects the credential database and key policy, secures the
//! concrete persistence adapter, and constructs the credential-owned
//! projection runtime. It deliberately has no management command gateway,
//! refresh coordinator, lease lifecycle, or reclaim sweep.

use std::sync::Arc;

use nebula_credential::{
    ApiKeyCredential, BasicAuthCredential, CredentialProjectionRuntime,
    CredentialProjectionRuntimeBuildError, CredentialRegistry, CredentialSlotResolver,
    DispatchError, DispatchOps, ErasedPendingStore, SigningKeyCredential, StateSource,
    register_runtime_ops,
};
#[cfg(feature = "postgres")]
use nebula_storage::credential::PgCredentialPersistence;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditSink, CredentialKeyring, CredentialKeyringError,
    CredentialStoreStartupError, EncryptionLayer, EnvKeyProvider, KeyProvider, ProviderError,
    SqliteCredentialPersistence,
};
use nebula_storage_port::{CredentialPersistence, CredentialPersistenceError};

const DEFAULT_CREDENTIAL_DB: &str = "sqlite://nebula-credentials.db?mode=rwc";
const DEVELOPMENT_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialDatabaseBackend {
    Sqlite,
    Postgres,
}

impl CredentialDatabaseBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
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
    /// The selected durable credential store could not be opened or migrated.
    #[error("credential store initialization failed")]
    Store(#[source] CredentialStoreStartupError),
    /// The credential database URL selected an unsupported backend.
    #[error(
        "NEBULA_CRED_DB has an unsupported scheme; use sqlite://, postgres://, or postgresql://"
    )]
    UnsupportedStoreScheme,
    /// PostgreSQL was explicitly requested from a binary without its driver.
    #[cfg(not(feature = "postgres"))]
    #[error(
        "NEBULA_CRED_DB requests PostgreSQL, but nebula-worker was built without the `postgres` feature"
    )]
    PostgresStoreUnavailable,
    /// The credential-owned projection runtime rejected incomplete parts.
    #[error("credential projection runtime construction failed")]
    Projection(#[from] CredentialProjectionRuntimeBuildError),
}

/// Compose the worker's first-party credential projection from environment policy.
///
/// `NEBULA_CRED_DB` defaults to the same SQLite URL as the server. Key loading
/// uses `NEBULA_CRED_MASTER_KEY`, except when `NEBULA_CRED_DEV_KEY=1`
/// explicitly opts into the shared fixed development key. The same bounded
/// decrypt-only legacy keyring as the server is installed for projection.
///
/// # Errors
///
/// Returns a typed error when key loading, backend selection, store startup,
/// first-party registration, or projection construction fails.
pub async fn compose_first_party_projection()
-> Result<Arc<dyn CredentialSlotResolver>, CredentialProjectionCompositionError> {
    let keyring = resolve_first_party_keyring()?;
    let database_url =
        std::env::var("NEBULA_CRED_DB").unwrap_or_else(|_| DEFAULT_CREDENTIAL_DB.to_owned());
    compose_first_party_projection_for_database(&database_url, keyring).await
}

/// Build the first-party projection runtime around one raw credential store.
///
/// This testable composition seam installs the same encryption, trace-audit,
/// registry, and operation layers used by [`compose_first_party_projection`].
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

async fn compose_first_party_projection_for_database(
    database_url: &str,
    keyring: CredentialKeyring,
) -> Result<Arc<dyn CredentialSlotResolver>, CredentialProjectionCompositionError> {
    let backend = classify_credential_database(database_url)?;
    match backend {
        CredentialDatabaseBackend::Sqlite => {
            let store = SqliteCredentialPersistence::connect(database_url)
                .await
                .map_err(CredentialProjectionCompositionError::Store)?;
            tracing::info!(
                backend = backend.as_str(),
                "credential projection store opened"
            );
            build_first_party_projection_with_keyring(store, keyring)
        },
        CredentialDatabaseBackend::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let store = PgCredentialPersistence::connect(database_url)
                    .await
                    .map_err(CredentialProjectionCompositionError::Store)?;
                tracing::info!(
                    backend = backend.as_str(),
                    "credential projection store opened"
                );
                build_first_party_projection_with_keyring(store, keyring)
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = keyring;
                Err(CredentialProjectionCompositionError::PostgresStoreUnavailable)
            }
        },
    }
}

fn classify_credential_database(
    database_url: &str,
) -> Result<CredentialDatabaseBackend, CredentialProjectionCompositionError> {
    let Some((scheme, _)) = database_url.split_once("://") else {
        if database_url.split_once(':').is_some_and(|(prefix, _)| {
            prefix.eq_ignore_ascii_case("postgres") || prefix.eq_ignore_ascii_case("postgresql")
        }) {
            return Err(CredentialProjectionCompositionError::UnsupportedStoreScheme);
        }
        return Ok(CredentialDatabaseBackend::Sqlite);
    };
    if scheme.eq_ignore_ascii_case("sqlite") {
        Ok(CredentialDatabaseBackend::Sqlite)
    } else if scheme.eq_ignore_ascii_case("postgres") || scheme.eq_ignore_ascii_case("postgresql") {
        Ok(CredentialDatabaseBackend::Postgres)
    } else {
        Err(CredentialProjectionCompositionError::UnsupportedStoreScheme)
    }
}

fn first_party_registry() -> Result<CredentialRegistry, nebula_credential::RegisterError> {
    let mut registry = CredentialRegistry::new();
    registry.register(ApiKeyCredential, "nebula-credential")?;
    registry.register(BasicAuthCredential, "nebula-credential")?;
    registry.register(SigningKeyCredential, "nebula-credential")?;
    Ok(registry)
}

fn first_party_ops() -> Result<DispatchOps<ErasedPendingStore>, DispatchError> {
    let mut ops = DispatchOps::new();
    register_runtime_ops::<ApiKeyCredential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<BasicAuthCredential, ErasedPendingStore>(&mut ops)?;
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
    use std::assert_matches;
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

    #[cfg(not(feature = "postgres"))]
    use super::compose_first_party_projection_for_database;
    use super::{
        CredentialDatabaseBackend, CredentialProjectionCompositionError,
        build_first_party_projection, build_first_party_projection_with_keyring,
        classify_credential_database, compose_first_party_projection,
    };
    use nebula_storage::credential::CredentialKeyring;

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

    #[tokio::test]
    async fn rolling_keyring_projects_legacy_server_rows_and_current_only_fails_closed() {
        let raw_store = Arc::new(
            SqliteCredentialPersistence::connect_memory()
                .await
                .expect("ready in-memory credential store"),
        );
        let scope = TenantScope::new("org-rotation", "workspace-rotation");
        let credential_id = CredentialId::new();
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
                    api_key_data("rolling-secret"),
                    "secret_token".to_owned(),
                    1,
                    None,
                    None,
                    false,
                    serde_json::Map::new(),
                    CredentialMaterialTransition::advance(),
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
        let database_path = temp.path().join("worker-credentials.db");
        let database_url = format!("sqlite://{}?mode=rwc", database_path.display());
        let raw_store = Arc::new(
            SqliteCredentialPersistence::connect(&database_url)
                .await
                .expect("ready file-backed credential store"),
        );
        let scope = TenantScope::new("org-env-rotation", "workspace-env-rotation");
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
        env.set("NEBULA_CRED_DB", &database_url);
        env.set("NEBULA_CRED_MASTER_KEY", TEST_KEY_BASE64);
        env.set("NEBULA_CRED_LEGACY_MASTER_KEYS", OLD_KEY_BASE64);
        env.remove("NEBULA_CRED_LEGACY_EMPTY_ID_MASTER_KEY");
        env.remove("NEBULA_CRED_DEV_KEY");

        let projection = compose_first_party_projection()
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
        let raw_store = Arc::new(
            SqliteCredentialPersistence::connect_memory()
                .await
                .expect("ready in-memory credential store"),
        );
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
        let raw_store = Arc::new(
            SqliteCredentialPersistence::connect_memory()
                .await
                .expect("ready in-memory credential store"),
        );
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

    #[test]
    fn database_backend_classification_matches_server_policy() {
        assert_eq!(
            classify_credential_database("sqlite://credentials.db")
                .expect("SQLite URL must classify"),
            CredentialDatabaseBackend::Sqlite
        );
        assert_eq!(
            classify_credential_database("sqlite::memory:")
                .expect("SQLite memory locator must classify"),
            CredentialDatabaseBackend::Sqlite
        );
        assert_eq!(
            classify_credential_database("var/lib/nebula/credentials.db")
                .expect("SQLite path must classify"),
            CredentialDatabaseBackend::Sqlite
        );
        assert_eq!(
            classify_credential_database("postgres://db/nebula")
                .expect("PostgreSQL URL must classify"),
            CredentialDatabaseBackend::Postgres
        );
        assert_eq!(
            classify_credential_database("postgresql://db/nebula")
                .expect("PostgreSQL alias URL must classify"),
            CredentialDatabaseBackend::Postgres
        );
    }

    #[test]
    fn unsupported_database_scheme_diagnostic_never_echoes_url() {
        let database_url = "mysql://operator:super-secret@example.invalid/tenant-private";
        let error = classify_credential_database(database_url)
            .expect_err("unsupported credential backend must fail closed");
        let diagnostic = format!("{error:?}: {error}");

        assert_matches!(
            error,
            CredentialProjectionCompositionError::UnsupportedStoreScheme
        );
        assert!(!diagnostic.contains(database_url));
        assert!(!diagnostic.contains("super-secret"));
        assert!(!diagnostic.contains("tenant-private"));
    }

    #[cfg(not(feature = "postgres"))]
    #[tokio::test]
    async fn postgres_request_without_feature_fails_closed_and_redacts_url() {
        let database_url =
            "postgres://operator:super-secret@example.invalid/tenant-private?sslmode=require";
        let key_provider: Arc<dyn KeyProvider> =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_BASE64).expect("valid fixed test key"));
        let keyring = CredentialKeyring::from_config(key_provider, None, None)
            .expect("empty legacy keyring composes");
        let error = match compose_first_party_projection_for_database(database_url, keyring).await {
            Err(error) => error,
            Ok(_) => panic!("PostgreSQL must not fall back without the feature"),
        };
        let diagnostic = format!("{error:?}: {error}");

        assert_matches!(
            error,
            CredentialProjectionCompositionError::PostgresStoreUnavailable
        );
        assert!(!diagnostic.contains(database_url));
        assert!(!diagnostic.contains("super-secret"));
        assert!(!diagnostic.contains("tenant-private"));
    }
}
