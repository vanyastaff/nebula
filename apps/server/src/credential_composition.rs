//! First-party credential runtime composition.
//!
//! Concrete adapters, key policy, registry selection, encryption, audit, and
//! process lifecycle belong to the deployment application. `nebula-api`
//! receives only its object-safe command gateway and catalog read model.

use std::sync::Arc;

use nebula_api::ports::credential_schema::CredentialSchemaPort;
use nebula_credential::{
    ApiKeyCredential, BasicAuthCredential, Capabilities, CredentialObserver, CredentialRegistry,
    CredentialService, CredentialServiceError, DispatchError, DispatchOps, ErasedPendingStore,
    EventMetricObserver, SigningKeyCredential, StateSource, register_runtime_ops,
    runtime::{
        CredentialResolver, LeaseLifecycle, LeaseLifecycleConfig, ReclaimSweepHandle,
        RefreshCoordConfig, RefreshCoordMetrics, RefreshCoordinator, SentinelThresholdConfig,
        SentinelTrigger,
    },
};
use nebula_crypto::EncryptionKey;
use nebula_metrics::MetricsRegistry;
#[cfg(feature = "postgres")]
use nebula_storage::credential::PgCredentialPersistence;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditSink, CredentialKeyring, CredentialKeyringError,
    CredentialStoreStartupError, EncryptionLayer, EnvKeyProvider, InMemoryPendingStore,
    KeyProvider, SqliteCredentialPersistence,
};
use nebula_storage_port::{
    CredentialPersistence, CredentialPersistenceError,
    store::{RefreshClaimAdjudicator, RefreshClaimStore, ReplicaId},
};
use thiserror::Error;
use uuid::Uuid;

use crate::credential_adapters::{RegistryCredentialSchema, ReqwestOAuthTransport};

const DEFAULT_CREDENTIAL_DB: &str = "sqlite://nebula-credentials.db?mode=rwc";
const DEVELOPMENT_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
const CREDENTIAL_EVENT_BUFFER: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

/// Fully composed first-party credential runtime parts.
pub(crate) struct CredentialRuntime {
    pub(crate) service: Arc<CredentialService>,
    pub(crate) catalog: Arc<dyn CredentialSchemaPort>,
    /// Privileged reconciliation seam, sharing this runtime's claim storage so
    /// the adjudicator clears the same poison row `try_claim` reads.
    pub(crate) adjudicator: Arc<dyn RefreshClaimAdjudicator>,
    /// The runtime's audit sink, so `AuditOperation::Reconcile` reaches the same
    /// sink as every other credential operation rather than a second one.
    pub(crate) audit_sink: Arc<dyn AuditSink>,
    /// Retains the sole periodic poison-accounting owner for the full server
    /// lifecycle. Drop aborts the task during shutdown.
    _reclaim_sweep: ReclaimSweepHandle,
}

/// Failure to compose the first-party credential runtime.
#[derive(Debug, Error)]
pub(crate) enum CredentialCompositionError {
    #[error("credential registry registration failed")]
    Registry(#[from] nebula_credential::RegisterError),
    #[error("credential catalog schema export failed")]
    Catalog(#[from] nebula_schema::JsonSchemaExportError),
    #[error("credential dispatch registration failed")]
    Dispatch(#[from] DispatchError),
    #[error("credential service composition failed")]
    Service(#[from] CredentialServiceError),
    #[error("credential key provider initialization failed: {0}")]
    KeyProvider(String),
    #[error("credential legacy master-key configuration is invalid")]
    InvalidLegacyKeyring(#[source] CredentialKeyringError),
    #[error("credential store initialization failed")]
    Store(#[source] CredentialStoreStartupError),
    #[error(
        "NEBULA_CRED_DB has an unsupported scheme; use sqlite://, postgres://, or postgresql://"
    )]
    UnsupportedStoreScheme,
    #[error(
        "NEBULA_CRED_DB requests PostgreSQL, but nebula-server was built without the `postgres` feature"
    )]
    #[cfg(not(feature = "postgres"))]
    PostgresStoreUnavailable,
    #[error("credential refresh transport initialization failed: {0}")]
    RefreshTransport(String),
    #[error("credential refresh coordinator initialization failed: {0}")]
    RefreshCoordinator(String),
}

/// Resolve the process-wide credential/identity encryption keyring.
pub(crate) fn resolve_first_party_keyring() -> Result<CredentialKeyring, CredentialCompositionError>
{
    let span = tracing::info_span!("credential_keyring_resolution");
    let _guard = span.enter();
    let current: Arc<dyn KeyProvider> =
        if std::env::var("NEBULA_CRED_DEV_KEY").as_deref() == Ok("1") {
            tracing::warn!(
                "security: NEBULA_CRED_DEV_KEY=1 — using a fixed development key; \
             credential and Plane-A identity secrets are not securely encrypted"
            );
            Arc::new(
                EnvKeyProvider::from_base64(DEVELOPMENT_KEY_BASE64)
                    .map_err(|error| CredentialCompositionError::KeyProvider(error.to_string()))?,
            )
        } else {
            Arc::new(
                EnvKeyProvider::from_env()
                    .map_err(|error| CredentialCompositionError::KeyProvider(error.to_string()))?,
            )
        };
    CredentialKeyring::from_env(current).map_err(|error| {
        tracing::error!(
            reason = error.category(),
            "credential keyring configuration rejected"
        );
        CredentialCompositionError::InvalidLegacyKeyring(error)
    })
}

/// Compose the durable first-party runtime and its shared catalog projection.
pub(crate) async fn compose_first_party_runtime(
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    metrics_registry: Arc<MetricsRegistry>,
) -> Result<CredentialRuntime, CredentialCompositionError> {
    let database_url =
        std::env::var("NEBULA_CRED_DB").unwrap_or_else(|_| DEFAULT_CREDENTIAL_DB.to_owned());
    compose_first_party_runtime_for_database(
        &database_url,
        key_provider,
        legacy_keys,
        metrics_registry,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn compose_memory_service(
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<CredentialService>, CredentialCompositionError> {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .map_err(CredentialCompositionError::Store)?;
    let (claim_repo, adjudicator) = shared_claim_repo(store.refresh_claim_repo());
    let runtime = compose_runtime(
        store,
        claim_repo,
        adjudicator,
        key_provider,
        Vec::new(),
        Arc::new(MetricsRegistry::new()),
    )?;
    let service = Arc::clone(&runtime.service);
    // Isolated tests do not exercise periodic maintenance. Production keeps
    // the complete runtime guard until `app::serve` exits.
    drop(runtime);
    Ok(service)
}

async fn compose_first_party_runtime_for_database(
    database_url: &str,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    metrics_registry: Arc<MetricsRegistry>,
) -> Result<CredentialRuntime, CredentialCompositionError> {
    let backend = classify_credential_database(database_url)?;

    match backend {
        CredentialDatabaseBackend::Sqlite => {
            let store = SqliteCredentialPersistence::connect(database_url)
                .await
                .map_err(CredentialCompositionError::Store)?;
            let (claim_repo, adjudicator) = shared_claim_repo(store.refresh_claim_repo());
            // Database URLs can carry credentials or tenant-specific
            // filesystem paths. Record only the closed backend class.
            tracing::info!(
                backend = backend.as_str(),
                "credential durable store opened"
            );
            compose_runtime(
                store,
                claim_repo,
                adjudicator,
                key_provider,
                legacy_keys,
                metrics_registry,
            )
        },
        CredentialDatabaseBackend::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let store = PgCredentialPersistence::connect(database_url)
                    .await
                    .map_err(CredentialCompositionError::Store)?;
                let (claim_repo, adjudicator) = shared_claim_repo(store.refresh_claim_repo());
                tracing::info!(
                    backend = backend.as_str(),
                    "credential durable store opened"
                );
                compose_runtime(
                    store,
                    claim_repo,
                    adjudicator,
                    key_provider,
                    legacy_keys,
                    metrics_registry,
                )
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = (key_provider, legacy_keys, metrics_registry);
                Err(CredentialCompositionError::PostgresStoreUnavailable)
            }
        },
    }
}

fn classify_credential_database(
    database_url: &str,
) -> Result<CredentialDatabaseBackend, CredentialCompositionError> {
    let Some((scheme, _)) = database_url.split_once("://") else {
        if database_url.split_once(':').is_some_and(|(prefix, _)| {
            prefix.eq_ignore_ascii_case("postgres") || prefix.eq_ignore_ascii_case("postgresql")
        }) {
            // A malformed PostgreSQL locator must not fall through to SQLite
            // path handling. Otherwise an operator typo such as
            // `postgres:...` can silently open a local file instead of the
            // intended durable backend.
            return Err(CredentialCompositionError::UnsupportedStoreScheme);
        }
        // Preserve SqliteCredentialPersistence's documented path-friendly
        // surface: relative, absolute, and Windows paths plus
        // `sqlite::memory:` are all SQLite. Only an explicit URL authority
        // scheme is allowed to select another backend.
        return Ok(CredentialDatabaseBackend::Sqlite);
    };
    if scheme.eq_ignore_ascii_case("sqlite") {
        Ok(CredentialDatabaseBackend::Sqlite)
    } else if scheme.eq_ignore_ascii_case("postgres") || scheme.eq_ignore_ascii_case("postgresql") {
        Ok(CredentialDatabaseBackend::Postgres)
    } else {
        Err(CredentialCompositionError::UnsupportedStoreScheme)
    }
}

fn compose_runtime<P>(
    raw_store: P,
    claim_repo: Arc<dyn RefreshClaimStore>,
    adjudicator: Arc<dyn RefreshClaimAdjudicator>,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    metrics_registry: Arc<MetricsRegistry>,
) -> Result<CredentialRuntime, CredentialCompositionError>
where
    P: CredentialPersistence + 'static,
{
    let registry = Arc::new(first_party_registry()?);
    let catalog: Arc<dyn CredentialSchemaPort> =
        Arc::new(RegistryCredentialSchema::new(Arc::clone(&registry))?);
    let ops = Arc::new(first_party_ops()?);
    validate_capability_dispatch(&registry, &ops)?;

    tracing::warn!(
        "credential audit sink is trace-only; durable audit persistence is scheduled for K3"
    );
    let encrypted: Arc<dyn CredentialPersistence> = Arc::new(EncryptionLayer::with_legacy_keys(
        raw_store,
        key_provider,
        legacy_keys,
    ));
    let audit_sink: Arc<dyn AuditSink> = Arc::new(TracingAuditSink);
    let store: Arc<dyn CredentialPersistence> =
        Arc::new(AuditLayer::new(encrypted, Arc::clone(&audit_sink)));
    let refresh_config = RefreshCoordConfig::default();
    let refresh_metrics = RefreshCoordMetrics::with_registry(&metrics_registry)
        .map_err(|error| CredentialCompositionError::RefreshCoordinator(error.to_string()))?;
    let refresh_coordinator = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&claim_repo),
            server_replica_id(),
            refresh_config.clone(),
        )
        .map_err(|error| CredentialCompositionError::RefreshCoordinator(error.to_string()))?
        .with_metrics(refresh_metrics)
        .with_audit_sink(Arc::clone(&audit_sink)),
    );
    let observer: Arc<dyn CredentialObserver> =
        Arc::new(EventMetricObserver::new(CREDENTIAL_EVENT_BUFFER));
    let credential_events = observer.event_bus();
    let sentinel = Arc::new(SentinelTrigger::new(
        claim_repo,
        SentinelThresholdConfig {
            threshold: refresh_config.sentinel_threshold,
            window: refresh_config.sentinel_window,
        },
    ));
    let reclaim_sweep = ReclaimSweepHandle::spawn(
        Arc::clone(&refresh_coordinator),
        sentinel,
        Some(Arc::clone(&credential_events)),
    );
    let oauth_transport = Arc::new(
        ReqwestOAuthTransport::new()
            .map_err(|error| CredentialCompositionError::RefreshTransport(error.to_string()))?,
    );
    let resolver = CredentialResolver::with_dependencies(
        Arc::clone(&store),
        refresh_coordinator,
        oauth_transport.clone(),
    )
    .with_event_bus(credential_events);
    let lease = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        observer.lease_bus(),
        observer.metrics(),
        tokio_util::sync::CancellationToken::new(),
    );
    let pending = ErasedPendingStore::new(Arc::new(InMemoryPendingStore::new()));
    let service = Arc::new(CredentialService::from_secure_parts(
        store,
        resolver,
        lease,
        pending,
        registry,
        ops,
        observer,
        oauth_transport,
        StateSource::LocalEncrypted,
    ));

    Ok(CredentialRuntime {
        service,
        catalog,
        adjudicator,
        audit_sink,
        _reclaim_sweep: reclaim_sweep,
    })
}

/// Build the claim store and its adjudication seam over **one** adapter
/// instance.
///
/// `refresh_claim_repo()` clones a pool handle, so two calls are two handles
/// onto one store rather than two stores. Both trait objects below are unsizing
/// coercions of one `Arc`, which is what guarantees the adjudicator clears the
/// very row `try_claim` reads. Every backend supplies its own repo here: sqlite
/// and postgres from their admitted pool, and the in-memory composition case —
/// `compose_memory_service` and the composition test fixtures — from
/// `SqliteCredentialPersistence::connect_memory`, a SQLite **in-memory
/// database** whose repo is `SqliteRefreshClaimRepo`. `InMemoryRefreshClaimRepo`
/// exists, but this path does not build it.
fn shared_claim_repo<R>(repo: R) -> (Arc<dyn RefreshClaimStore>, Arc<dyn RefreshClaimAdjudicator>)
where
    R: RefreshClaimStore + RefreshClaimAdjudicator + 'static,
{
    let repo = Arc::new(repo);
    (
        Arc::clone(&repo) as Arc<dyn RefreshClaimStore>,
        repo as Arc<dyn RefreshClaimAdjudicator>,
    )
}

fn server_replica_id() -> ReplicaId {
    ReplicaId::new(format!("nebula-server:{}", Uuid::new_v4()))
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

fn validate_capability_dispatch(
    registry: &CredentialRegistry,
    ops: &DispatchOps<ErasedPendingStore>,
) -> Result<(), CredentialServiceError> {
    let modeled = Capabilities::REFRESHABLE
        | Capabilities::TESTABLE
        | Capabilities::REVOCABLE
        | Capabilities::INTERACTIVE;
    for key in registry.iter_keys() {
        let advertised = registry
            .capabilities_of(key)
            .unwrap_or_default()
            .intersection(modeled);
        let missing = advertised.difference(ops.capabilities_of(key));
        if !missing.is_empty() {
            return Err(CredentialServiceError::CapabilityWithoutOps {
                capability: first_missing_capability(missing).to_owned(),
                key: key.to_owned(),
            });
        }
    }
    Ok(())
}

fn first_missing_capability(missing: Capabilities) -> &'static str {
    if missing.contains(Capabilities::REFRESHABLE) {
        "refresh"
    } else if missing.contains(Capabilities::TESTABLE) {
        "test"
    } else if missing.contains(Capabilities::REVOCABLE) {
        "revoke"
    } else if missing.contains(Capabilities::INTERACTIVE) {
        "interactive"
    } else {
        "unknown"
    }
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
    use super::*;
    use base64::Engine as _;

    const TEST_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

    fn encoded_key(byte: u8) -> String {
        base64::engine::general_purpose::STANDARD.encode([byte; 32])
    }

    fn test_provider(byte: u8) -> Arc<dyn KeyProvider> {
        Arc::new(EnvKeyProvider::from_base64(&encoded_key(byte)).expect("valid fixed test key"))
    }

    #[test]
    fn keyring_accepts_bounded_distinct_decrypt_only_keys() {
        let configured = format!("{},{}", encoded_key(2), encoded_key(3));
        let keyring =
            CredentialKeyring::from_config(test_provider(1), Some(configured.as_str()), None)
                .expect("distinct legacy keys compose");

        assert_eq!(keyring.identity_legacy().len(), 2);
        let current_id = keyring
            .current()
            .current()
            .expect("current key")
            .key_id()
            .to_owned();
        assert!(
            keyring
                .identity_legacy()
                .iter()
                .all(|(key_id, _)| key_id != &current_id)
        );
    }

    #[test]
    fn keyring_rejects_duplicate_current_and_malformed_entries_without_echoing_secrets() {
        let duplicate = encoded_key(2);
        let cases = [
            format!("{duplicate},{duplicate}"),
            encoded_key(1),
            format!("{},,{}", encoded_key(2), encoded_key(3)),
            "not-base64-secret".to_owned(),
        ];

        for configured in cases {
            let error = match CredentialKeyring::from_config(
                test_provider(1),
                Some(configured.as_str()),
                None,
            ) {
                Err(error) => error,
                Ok(_) => panic!("unsafe keyring configuration must fail closed"),
            };
            let diagnostic = format!("{error:?}: {error}");
            assert!(!diagnostic.contains(&configured));
        }
    }

    #[test]
    fn keyring_rejects_more_than_eight_legacy_keys() {
        let configured = (2..=10).map(encoded_key).collect::<Vec<_>>().join(",");

        assert!(matches!(
            CredentialKeyring::from_config(test_provider(1), Some(configured.as_str()), None,),
            Err(CredentialKeyringError::TooManyLegacyKeys)
        ));
    }

    #[test]
    fn empty_id_alias_is_credential_only() {
        let empty_id_key = encoded_key(1);
        let keyring =
            CredentialKeyring::from_config(test_provider(1), None, Some(empty_id_key.as_str()))
                .expect("explicit empty-id alias composes");

        assert!(keyring.identity_legacy().is_empty());
        assert_eq!(keyring.credential_legacy().len(), 1);
        assert!(keyring.credential_legacy()[0].0.is_empty());
    }

    #[test]
    fn process_replica_ids_are_unique_and_diagnostic() {
        let first = server_replica_id();
        let second = server_replica_id();

        assert_ne!(first, second);
        assert!(first.as_str().starts_with("nebula-server:"));
        assert!(second.as_str().starts_with("nebula-server:"));
    }

    #[tokio::test]
    async fn composed_runtime_retains_reclaim_sweep_until_shutdown() {
        let store = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("ready in-memory credential store");
        let (claim_repo, adjudicator) = shared_claim_repo(store.refresh_claim_repo());
        let key_provider: Arc<dyn KeyProvider> =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_BASE64).expect("valid fixed test key"));
        let runtime = compose_runtime(
            store,
            claim_repo,
            adjudicator,
            key_provider,
            Vec::new(),
            Arc::new(MetricsRegistry::new()),
        )
        .expect("credential runtime composes");

        assert!(
            !runtime._reclaim_sweep.is_finished(),
            "composition must retain a live periodic poison-accounting owner"
        );
        runtime._reclaim_sweep.abort();
        for _ in 0..8 {
            if runtime._reclaim_sweep.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            runtime._reclaim_sweep.is_finished(),
            "shutdown must abort the retained reclaim task"
        );
    }

    #[test]
    fn database_backend_classification_is_explicit() {
        assert!(matches!(
            classify_credential_database("sqlite://credentials.db"),
            Ok(CredentialDatabaseBackend::Sqlite)
        ));
        assert!(matches!(
            classify_credential_database("sqlite::memory:"),
            Ok(CredentialDatabaseBackend::Sqlite)
        ));
        assert!(matches!(
            classify_credential_database("var/lib/nebula/credentials.db"),
            Ok(CredentialDatabaseBackend::Sqlite)
        ));
        assert!(matches!(
            classify_credential_database("/var/lib/nebula/credentials.db"),
            Ok(CredentialDatabaseBackend::Sqlite)
        ));
        assert!(matches!(
            classify_credential_database(r"C:\nebula\credentials.db"),
            Ok(CredentialDatabaseBackend::Sqlite)
        ));
        assert!(matches!(
            classify_credential_database("postgres://db/nebula"),
            Ok(CredentialDatabaseBackend::Postgres)
        ));
        assert!(matches!(
            classify_credential_database("postgresql://db/nebula"),
            Ok(CredentialDatabaseBackend::Postgres)
        ));
        for malformed in [
            "postgres:operator-secret@example.invalid/nebula",
            "POSTGRESQL:operator-secret@example.invalid/nebula",
        ] {
            assert!(matches!(
                classify_credential_database(malformed),
                Err(CredentialCompositionError::UnsupportedStoreScheme)
            ));
        }
    }

    #[test]
    fn unsupported_database_scheme_diagnostic_never_echoes_url() {
        let database_url = "mysql://operator:super-secret@example.invalid/tenant-private";
        let error = classify_credential_database(database_url)
            .expect_err("unsupported credential backend must fail closed");
        let diagnostic = format!("{error:?}: {error}");

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
        let result = compose_first_party_runtime_for_database(
            database_url,
            key_provider,
            Vec::new(),
            Arc::new(MetricsRegistry::new()),
        )
        .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("PostgreSQL must not fall back without the feature"),
        };
        let diagnostic = format!("{error:?}: {error}");

        assert!(matches!(
            error,
            CredentialCompositionError::PostgresStoreUnavailable
        ));
        assert!(!diagnostic.contains(database_url));
        assert!(!diagnostic.contains("super-secret"));
        assert!(!diagnostic.contains("tenant-private"));
    }
}
