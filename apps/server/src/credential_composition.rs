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
    NoopObserver, SigningKeyCredential, StateSource, register_runtime_ops,
    runtime::{CredentialResolver, LeaseLifecycle, LeaseLifecycleConfig},
};
use nebula_engine::credential::default_in_memory_coordinator;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditSink, EncryptionLayer, EnvKeyProvider, InMemoryPendingStore,
    KeyProvider, SqliteCredentialPersistence,
};
use nebula_storage_port::{CredentialPersistence, CredentialPersistenceError};
use thiserror::Error;

use crate::credential_adapters::{RegistryCredentialSchema, ReqwestRefreshTransport};

const DEFAULT_CREDENTIAL_DB: &str = "sqlite://nebula-credentials.db?mode=rwc";
const DEVELOPMENT_KEY_BASE64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

/// Fully composed first-party credential runtime parts.
pub(crate) struct CredentialRuntime {
    pub(crate) service: Arc<CredentialService>,
    pub(crate) catalog: Arc<dyn CredentialSchemaPort>,
}

/// Failure to compose the first-party credential runtime.
#[derive(Debug, Error)]
pub(crate) enum CredentialCompositionError {
    #[error("credential registry registration failed")]
    Registry(#[from] nebula_credential::RegisterError),
    #[error("credential dispatch registration failed")]
    Dispatch(#[from] DispatchError),
    #[error("credential service composition failed")]
    Service(#[from] CredentialServiceError),
    #[error("credential key provider initialization failed: {0}")]
    KeyProvider(String),
    #[error("credential store initialization failed: {0}")]
    Store(String),
    #[error("credential refresh transport initialization failed: {0}")]
    RefreshTransport(String),
}

/// Resolve the process-wide credential/identity key provider.
pub(crate) fn resolve_first_party_key_provider()
-> Result<Arc<dyn KeyProvider>, CredentialCompositionError> {
    if std::env::var("NEBULA_CRED_DEV_KEY").as_deref() == Ok("1") {
        tracing::warn!(
            "security: NEBULA_CRED_DEV_KEY=1 — using a fixed development key; \
             credential and Plane-A identity secrets are not securely encrypted"
        );
        EnvKeyProvider::from_base64(DEVELOPMENT_KEY_BASE64)
            .map(|provider| Arc::new(provider) as Arc<dyn KeyProvider>)
            .map_err(|error| CredentialCompositionError::KeyProvider(error.to_string()))
    } else {
        EnvKeyProvider::from_env()
            .map(|provider| Arc::new(provider) as Arc<dyn KeyProvider>)
            .map_err(|error| CredentialCompositionError::KeyProvider(error.to_string()))
    }
}

/// Compose the durable first-party runtime and its shared catalog projection.
pub(crate) async fn compose_first_party_runtime(
    key_provider: Arc<dyn KeyProvider>,
) -> Result<CredentialRuntime, CredentialCompositionError> {
    let database_url =
        std::env::var("NEBULA_CRED_DB").unwrap_or_else(|_| DEFAULT_CREDENTIAL_DB.to_owned());
    let store = SqliteCredentialPersistence::connect(&database_url)
        .await
        .map_err(|error| CredentialCompositionError::Store(error.to_string()))?;
    tracing::info!(db = %database_url, "credential durable SQLite store opened");

    compose_runtime(store, key_provider)
}

#[cfg(test)]
pub(crate) async fn compose_memory_service(
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<CredentialService>, CredentialCompositionError> {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .map_err(|error| CredentialCompositionError::Store(error.to_string()))?;
    Ok(compose_runtime(store, key_provider)?.service)
}

fn compose_runtime<S: CredentialPersistence + 'static>(
    raw_store: S,
    key_provider: Arc<dyn KeyProvider>,
) -> Result<CredentialRuntime, CredentialCompositionError> {
    let registry = Arc::new(first_party_registry()?);
    let catalog: Arc<dyn CredentialSchemaPort> =
        Arc::new(RegistryCredentialSchema::new(Arc::clone(&registry)));
    let ops = Arc::new(first_party_ops()?);
    validate_capability_dispatch(&registry, &ops)?;

    tracing::warn!(
        "credential audit sink is trace-only; durable audit persistence remains K2 debt"
    );
    let encrypted: Arc<dyn CredentialPersistence> =
        Arc::new(EncryptionLayer::new(raw_store, key_provider));
    let store: Arc<dyn CredentialPersistence> = Arc::new(AuditLayer::new(
        encrypted,
        Arc::new(TracingAuditSink) as Arc<dyn AuditSink>,
    ));
    let refresh_coordinator = Arc::new(
        default_in_memory_coordinator()
            .map_err(|error| CredentialServiceError::Internal(error.to_string()))?,
    );
    let observer: Arc<dyn CredentialObserver> = Arc::new(NoopObserver::new());
    let refresh_transport = ReqwestRefreshTransport::new()
        .map_err(|error| CredentialCompositionError::RefreshTransport(error.to_string()))?;
    let resolver = CredentialResolver::with_dependencies(
        Arc::clone(&store),
        refresh_coordinator,
        Arc::new(refresh_transport),
    )
    .with_event_bus(observer.event_bus());
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
        StateSource::LocalEncrypted,
    ));

    Ok(CredentialRuntime { service, catalog })
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
