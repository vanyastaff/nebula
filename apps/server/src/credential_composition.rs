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
    EventMetricObserver, OAuth2Credential, SigningKeyCredential, StateSource,
    register_interactive_ops, register_refreshable_ops, register_runtime_ops,
    runtime::{
        CredentialLifecycleRuntime, CredentialRefreshSchedulerConfig,
        CredentialRefreshSchedulerConfigError, CredentialResolver, LeaseLifecycleConfig,
        ReclaimSweepHandle, RefreshCoordConfig, RefreshCoordMetrics, RefreshCoordinator,
        SentinelEscalationPolicy,
    },
};
use nebula_crypto::EncryptionKey;
use nebula_metrics::MetricsRegistry;
#[cfg(feature = "postgres")]
use nebula_storage::credential::PgCredentialPersistence;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditSink, CredentialKeyring, CredentialKeyringError,
    CredentialStoreStartupError, EncryptionLayer, EnvKeyProvider, KeyProvider,
    SqliteCredentialPersistence,
};
use nebula_storage_port::{
    CredentialPersistence, CredentialPersistenceError, CredentialRefreshSchedule,
    store::{RefreshClaimAdjudicator, RefreshClaimReclaimer, RefreshClaimStore, ReplicaId},
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
    lifecycle: CredentialLifecycleRuntime,
    pub(crate) catalog: Arc<dyn CredentialSchemaPort>,
    /// Privileged reconciliation seam, sharing this runtime's claim storage so
    /// the adjudicator clears the same poison row `try_claim` reads.
    pub(crate) adjudicator: Arc<dyn RefreshClaimAdjudicator>,
    /// The runtime's audit sink, so `AuditOperation::Reconcile` reaches the same
    /// sink as every other credential operation rather than a second one.
    pub(crate) audit_sink: Arc<dyn AuditSink>,
}

impl CredentialRuntime {
    pub(crate) fn service(&self) -> Arc<CredentialService> {
        self.lifecycle.service()
    }

    pub(crate) async fn shutdown(&mut self) {
        self.lifecycle.shutdown().await;
    }

    #[cfg(test)]
    fn reclaim_sweep_is_finished(&self) -> bool {
        self.lifecycle.reclaim_sweep_is_finished()
    }
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
    /// `NEBULA_CRED_DB_MAX_CONNECTIONS` is set but not a positive integer.
    #[error("NEBULA_CRED_DB_MAX_CONNECTIONS must be a positive integer")]
    InvalidStorePoolSize,
    #[error("credential refresh transport initialization failed: {0}")]
    RefreshTransport(String),
    #[error("credential refresh coordinator initialization failed: {0}")]
    RefreshCoordinator(String),
    #[error("credential refresh scheduler configuration is invalid")]
    RefreshScheduler(#[from] CredentialRefreshSchedulerConfigError),
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
    let refresh_ports = refresh_runtime_ports(store.refresh_schedule(), store.refresh_claim_repo());
    let pending = sqlite_pending_store(&store, Arc::clone(&key_provider), Vec::new());
    let runtime = compose_runtime(
        store,
        refresh_ports,
        pending,
        key_provider,
        Vec::new(),
        Arc::new(MetricsRegistry::new()),
    )?;
    let service = runtime.service();
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
            let refresh_ports =
                refresh_runtime_ports(store.refresh_schedule(), store.refresh_claim_repo());
            let pending =
                sqlite_pending_store(&store, Arc::clone(&key_provider), legacy_keys.clone());
            // Database URLs can carry credentials or tenant-specific
            // filesystem paths. Record only the closed backend class.
            tracing::info!(
                backend = backend.as_str(),
                "credential durable store opened"
            );
            compose_runtime(
                store,
                refresh_ports,
                pending,
                key_provider,
                legacy_keys,
                metrics_registry,
            )
        },
        CredentialDatabaseBackend::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let store =
                    PgCredentialPersistence::connect_sized(database_url, credential_pool_size()?)
                        .await
                        .map_err(CredentialCompositionError::Store)?;
                let refresh_ports =
                    refresh_runtime_ports(store.refresh_schedule(), store.refresh_claim_repo());
                let pending =
                    postgres_pending_store(&store, Arc::clone(&key_provider), legacy_keys.clone());
                tracing::info!(
                    backend = backend.as_str(),
                    "credential durable store opened"
                );
                compose_runtime(
                    store,
                    refresh_ports,
                    pending,
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
    refresh_ports: CredentialRefreshRuntimePorts,
    pending: ErasedPendingStore,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    metrics_registry: Arc<MetricsRegistry>,
) -> Result<CredentialRuntime, CredentialCompositionError>
where
    P: CredentialPersistence + 'static,
{
    let oauth_transport = Arc::new(
        ReqwestOAuthTransport::new()
            .map_err(|error| CredentialCompositionError::RefreshTransport(error.to_string()))?,
    );
    compose_runtime_with_transport(
        raw_store,
        refresh_ports,
        pending,
        key_provider,
        legacy_keys,
        metrics_registry,
        oauth_transport,
    )
}

// The concrete transport retains first-party egress policy. Tests supply its
// existing TLS/DNS fixture constructor; production always enters above.
fn compose_runtime_with_transport<P>(
    raw_store: P,
    refresh_ports: CredentialRefreshRuntimePorts,
    pending: ErasedPendingStore,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    metrics_registry: Arc<MetricsRegistry>,
    oauth_transport: Arc<ReqwestOAuthTransport>,
) -> Result<CredentialRuntime, CredentialCompositionError>
where
    P: CredentialPersistence + 'static,
{
    compose_runtime_with_policy_and_transport(
        raw_store,
        refresh_ports,
        pending,
        CredentialEncryptionConfig {
            key_provider,
            legacy_keys,
        },
        metrics_registry,
        oauth_transport,
        CredentialLifecyclePolicy::default(),
    )
}

struct CredentialEncryptionConfig {
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
}

#[derive(Default)]
struct CredentialLifecyclePolicy {
    refresh: RefreshCoordConfig,
    scheduler: CredentialRefreshSchedulerConfig,
}

#[cfg(test)]
fn compose_runtime_with_test_policy<P>(
    raw_store: P,
    refresh_ports: CredentialRefreshRuntimePorts,
    pending: ErasedPendingStore,
    key_provider: Arc<dyn KeyProvider>,
    metrics_registry: Arc<MetricsRegistry>,
    oauth_transport: Arc<ReqwestOAuthTransport>,
    policy: CredentialLifecyclePolicy,
) -> Result<CredentialRuntime, CredentialCompositionError>
where
    P: CredentialPersistence + 'static,
{
    compose_runtime_with_policy_and_transport(
        raw_store,
        refresh_ports,
        pending,
        CredentialEncryptionConfig {
            key_provider,
            legacy_keys: Vec::new(),
        },
        metrics_registry,
        oauth_transport,
        policy,
    )
}

fn compose_runtime_with_policy_and_transport<P>(
    raw_store: P,
    refresh_ports: CredentialRefreshRuntimePorts,
    pending: ErasedPendingStore,
    encryption: CredentialEncryptionConfig,
    metrics_registry: Arc<MetricsRegistry>,
    oauth_transport: Arc<ReqwestOAuthTransport>,
    policy: CredentialLifecyclePolicy,
) -> Result<CredentialRuntime, CredentialCompositionError>
where
    P: CredentialPersistence + 'static,
{
    let CredentialEncryptionConfig {
        key_provider,
        legacy_keys,
    } = encryption;
    let CredentialLifecyclePolicy {
        refresh: refresh_config,
        scheduler: scheduler_config,
    } = policy;
    let CredentialRefreshRuntimePorts {
        schedule: refresh_schedule,
        claims: claim_repo,
        reclaimer,
        adjudicator,
    } = refresh_ports;
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
    let escalation_policy = SentinelEscalationPolicy::new(
        refresh_config.sentinel_threshold,
        refresh_config.sentinel_window,
    )
    .map_err(|error| CredentialCompositionError::RefreshCoordinator(error.to_string()))?;
    let reclaim_sweep = ReclaimSweepHandle::spawn(
        Arc::clone(&refresh_coordinator),
        reclaimer,
        escalation_policy,
        Some(Arc::clone(&credential_events)),
    );
    let resolver = CredentialResolver::with_dependencies(
        Arc::clone(&store),
        refresh_coordinator,
        oauth_transport.clone(),
    )
    .with_event_bus(credential_events);
    let lifecycle = CredentialLifecycleRuntime::compose_with_refresh_schedule(
        refresh_schedule,
        scheduler_config,
        reclaim_sweep,
        LeaseLifecycleConfig::default(),
        observer.lease_bus(),
        observer.metrics(),
        |lease| {
            Arc::new(CredentialService::from_secure_parts(
                store,
                resolver,
                lease,
                pending,
                registry,
                ops,
                observer,
                oauth_transport,
                StateSource::LocalEncrypted,
            ))
        },
    )?;

    Ok(CredentialRuntime {
        lifecycle,
        catalog,
        adjudicator,
        audit_sink,
    })
}

fn sqlite_pending_store(
    store: &SqliteCredentialPersistence,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
) -> ErasedPendingStore {
    ErasedPendingStore::new(Arc::new(
        store.pending_state_store(key_provider, legacy_keys),
    ))
}

#[cfg(feature = "postgres")]
fn postgres_pending_store(
    store: &PgCredentialPersistence,
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
) -> ErasedPendingStore {
    ErasedPendingStore::new(Arc::new(
        store.pending_state_store(key_provider, legacy_keys),
    ))
}

/// Group due-scan, request, reclaim, and adjudication capabilities for one
/// durable credential backend.
///
/// `refresh_claim_repo()` clones a pool handle, so two calls are two handles
/// onto one store rather than two stores. All three trait objects below are
/// unsizing coercions of one `Arc`, which guarantees reclaim and adjudication
/// operate on the very row `try_claim` reads. Every backend supplies its own repo here: sqlite
/// and postgres from their admitted pool, and the in-memory composition case —
/// `compose_memory_service` and the composition test fixtures — from
/// `SqliteCredentialPersistence::connect_memory`, a SQLite **in-memory
/// database** whose repo is `SqliteRefreshClaimRepo`. `InMemoryRefreshClaimRepo`
/// exists, but this path does not build it.
struct CredentialRefreshRuntimePorts {
    schedule: Arc<dyn CredentialRefreshSchedule>,
    claims: Arc<dyn RefreshClaimStore>,
    reclaimer: Arc<dyn RefreshClaimReclaimer>,
    adjudicator: Arc<dyn RefreshClaimAdjudicator>,
}

fn refresh_runtime_ports<S, R>(schedule: S, repo: R) -> CredentialRefreshRuntimePorts
where
    S: CredentialRefreshSchedule,
    R: RefreshClaimStore + RefreshClaimReclaimer + RefreshClaimAdjudicator + 'static,
{
    let repo = Arc::new(repo);
    CredentialRefreshRuntimePorts {
        schedule: Arc::new(schedule),
        claims: Arc::clone(&repo) as Arc<dyn RefreshClaimStore>,
        reclaimer: Arc::clone(&repo) as Arc<dyn RefreshClaimReclaimer>,
        adjudicator: repo as Arc<dyn RefreshClaimAdjudicator>,
    }
}

fn server_replica_id() -> ReplicaId {
    ReplicaId::new(format!("nebula-server:{}", Uuid::new_v4()))
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

/// Connections the PostgreSQL credential store may pool, from
/// `NEBULA_CRED_DB_MAX_CONNECTIONS`; the store default when unset.
///
/// Every credential admission reads through this pool, so it bounds how many
/// run against PostgreSQL at once in this process.
#[cfg(feature = "postgres")]
fn credential_pool_size() -> Result<std::num::NonZeroU32, CredentialCompositionError> {
    match std::env::var("NEBULA_CRED_DB_MAX_CONNECTIONS") {
        Err(_) => Ok(nebula_storage::credential::DEFAULT_CREDENTIAL_POOL_SIZE),
        Ok(raw) => raw
            .trim()
            .parse()
            .map_err(|_| CredentialCompositionError::InvalidStorePoolSize),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use nebula_credential::{
        AuthStyle, OAuth2Pending, PendingState, PendingStateStore, PendingStoreError, SecretString,
        credentials::OAuth2Config,
    };

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

    #[test]
    fn first_party_composition_admits_universal_oauth2_capabilities() {
        let registry = first_party_registry().expect("first-party registry composes");
        let ops = first_party_ops().expect("first-party dispatch composes");

        assert_eq!(
            registry.capabilities_of("oauth2"),
            Some(Capabilities::INTERACTIVE | Capabilities::REFRESHABLE)
        );
        assert_eq!(
            ops.capabilities_of("oauth2"),
            Capabilities::INTERACTIVE | Capabilities::REFRESHABLE
        );
        validate_capability_dispatch(&registry, &ops)
            .expect("every advertised OAuth2 capability has runtime dispatch");
    }

    #[tokio::test]
    async fn composed_runtime_retains_reclaim_sweep_until_shutdown() {
        let store = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("ready in-memory credential store");
        let refresh_ports =
            refresh_runtime_ports(store.refresh_schedule(), store.refresh_claim_repo());
        let key_provider: Arc<dyn KeyProvider> =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_BASE64).expect("valid fixed test key"));
        let pending = sqlite_pending_store(&store, Arc::clone(&key_provider), Vec::new());
        let mut runtime = compose_runtime(
            store,
            refresh_ports,
            pending,
            key_provider,
            Vec::new(),
            Arc::new(MetricsRegistry::new()),
        )
        .expect("credential runtime composes");

        assert!(
            !runtime.reclaim_sweep_is_finished(),
            "composition must retain a live periodic poison-accounting owner"
        );
        runtime.shutdown().await;
        for _ in 0..8 {
            if runtime.reclaim_sweep_is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            runtime.reclaim_sweep_is_finished(),
            "shutdown must abort the retained reclaim task"
        );
    }

    fn oauth_pending(state: &str) -> OAuth2Pending {
        OAuth2Pending {
            config: OAuth2Config::authorization_code("https://app.example.test/oauth/callback")
                .auth_url("https://provider.example.test/authorize")
                .token_url("https://provider.example.test/token")
                .build(),
            client_id: "client-id".to_owned(),
            client_secret: SecretString::new("client-secret"),
            auth_style: AuthStyle::Header,
            pkce_verifier: SecretString::new("pkce-verifier"),
            state: state.to_owned(),
            redirect_uri: "https://app.example.test/oauth/callback".to_owned(),
        }
    }

    #[tokio::test]
    async fn sqlite_composition_preserves_oauth_pending_state_across_restart() {
        const OWNER: &str = "owner-restart";
        const SESSION: &str = "session-restart";
        let directory = tempfile::tempdir().expect("temporary credential database directory");
        let database_path = directory.path().join("credentials.db");
        let database_url = database_path
            .to_str()
            .expect("temporary path is valid UTF-8");

        let first_store = SqliteCredentialPersistence::connect(database_url)
            .await
            .expect("first admitted credential store");
        let first_key = test_provider(19);
        let first_pending = sqlite_pending_store(&first_store, Arc::clone(&first_key), Vec::new());
        let first_refresh_ports = refresh_runtime_ports(
            first_store.refresh_schedule(),
            first_store.refresh_claim_repo(),
        );
        let mut first_runtime = compose_runtime(
            first_store,
            first_refresh_ports,
            first_pending.clone(),
            first_key,
            Vec::new(),
            Arc::new(MetricsRegistry::new()),
        )
        .expect("first credential runtime composes");
        let token = first_pending
            .put(
                OAuth2Pending::KIND,
                OWNER,
                SESSION,
                oauth_pending("restart-state"),
            )
            .await
            .expect("OAuth pending state is stored");

        first_runtime.shutdown().await;
        drop(first_runtime);
        drop(first_pending);

        let second_store = SqliteCredentialPersistence::connect(database_url)
            .await
            .expect("credential store reopens after restart");
        let second_key = test_provider(19);
        let second_pending =
            sqlite_pending_store(&second_store, Arc::clone(&second_key), Vec::new());
        let second_refresh_ports = refresh_runtime_ports(
            second_store.refresh_schedule(),
            second_store.refresh_claim_repo(),
        );
        let mut second_runtime = compose_runtime(
            second_store,
            second_refresh_ports,
            second_pending.clone(),
            second_key,
            Vec::new(),
            Arc::new(MetricsRegistry::new()),
        )
        .expect("second credential runtime composes");

        let wrong_binding = second_pending
            .consume::<OAuth2Pending>(OAuth2Pending::KIND, &token, OWNER, "wrong-session")
            .await
            .expect_err("wrong session binding must fail closed");
        assert!(matches!(
            wrong_binding,
            PendingStoreError::ValidationFailed { .. }
        ));

        let consumed = second_pending
            .consume::<OAuth2Pending>(OAuth2Pending::KIND, &token, OWNER, SESSION)
            .await
            .expect("matching OAuth pending state survives restart");
        assert_eq!(consumed.state, "restart-state");

        second_runtime.shutdown().await;
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

#[cfg(test)]
#[path = "credential_acquisition_restart_tests.rs"]
mod acquisition_restart_tests;

#[cfg(test)]
#[path = "credential_refresh_restart_tests.rs"]
mod refresh_restart_tests;
