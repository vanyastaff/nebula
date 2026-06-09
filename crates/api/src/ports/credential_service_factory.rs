//! Composition-root factory for the production [`CredentialService`] facade.
//!
//! Built here in `nebula-api` — which already depends on
//! `nebula-credential-runtime`, `nebula-storage` (with the
//! `credential-in-memory` adapter), `nebula-engine`, and `tokio-util` — so
//! `apps/server` constructs a real service through a single typed call and
//! stays free of any credential dependency, mirroring
//! [`super::credential_schema_registry::try_default_registry_port`]. No
//! `deny.toml` edge is added.
//!
//! The service composes the secure layered store
//! (`Audit(Cache(Encryption(InMemoryStore)))`), the engine resolver, and the
//! lease lifecycle. Storage is **encrypted-at-rest but in-memory** — durable
//! across `.await`, not across process restart. A durable backend is a future
//! swap behind the now-erased `CredentialService` collaborators.

use std::sync::Arc;

use nebula_credential::{
    ApiKeyCredential, BasicAuthCredential, ErasedPendingStore, OAuth2Credential,
};
use nebula_credential_runtime::{
    CredentialService, CredentialServiceBuilder, CredentialServiceError, DispatchError,
    DispatchOps, NoopObserver, register_interactive_ops, register_refreshable_ops,
    register_revocable_ops, register_runtime_ops, register_testable_ops,
};
use nebula_engine::credential::LeaseLifecycleConfig;
use nebula_storage::credential::{
    AuditEvent, AuditSink, CacheConfig, EnvKeyProvider, InMemoryPendingStore, InMemoryStore,
    KeyProvider,
};

/// Audit sink that records every credential operation to the tracing log
/// (metadata only — [`AuditEvent`] carries no secret material by design).
/// Honest local-first sink: the audit trail goes to the structured log
/// stream, not silently dropped. A durable sink (DB) is a future swap;
/// this keeps the §14 audit trail visible without a backend.
struct TracingAuditSink;

impl AuditSink for TracingAuditSink {
    fn record(&self, event: &AuditEvent) -> Result<(), nebula_credential::StoreError> {
        tracing::info!(
            target: "nebula.credential.audit",
            cred_id = %event.credential_id,
            op = ?event.operation,
            result = ?event.result,
            "credential audit event"
        );
        Ok(())
    }
}

/// Construction failure for [`try_default_credential_service`].
///
/// Each variant names the composition step that failed. Source-chained
/// (`#[source]`/`#[from]`) where the underlying type is reachable from
/// `nebula-api`; stringified for the key-provider step, whose
/// `nebula_storage::credential::ProviderError` would otherwise force an
/// awkward direct dependency on a deeper storage error type at this seam.
#[derive(Debug, thiserror::Error)]
pub enum CredentialServiceFactoryError {
    /// A first-party credential KEY failed to register in the shared
    /// registry (a composition bug — first-party KEYs are statically
    /// unique, so this is unreachable in practice).
    #[error("credential registry registration failed")]
    Registry(#[from] nebula_credential::RegisterError),
    /// The encryption key provider could not be initialized. In production
    /// this means `NEBULA_CRED_MASTER_KEY` is unset, malformed, or the
    /// refused dev placeholder. The message is the provider error's
    /// sanitized `Display` (never key material).
    #[error("credential key provider init failed: {0}")]
    KeyProvider(String),
    /// A capability dispatch op failed to register (e.g. a duplicate KEY
    /// across two registrars).
    #[error("credential dispatch-ops registration failed")]
    Dispatch(#[from] DispatchError),
    /// The service builder rejected the composed parts — most often a
    /// registry capability advertised without a matching registered op.
    #[error("credential service build failed")]
    Build(#[from] CredentialServiceError),
}

/// Build the production [`CredentialService`] with the first-party
/// credential types registered (`api_key`, `basic_auth`, `oauth2`).
///
/// The service shares its registered type set with the schema port via
/// `credential_schema_registry::default_registry`, so the
/// registry-advertised capabilities and the dispatch ops table cannot
/// drift. OAuth2 advertises all four ops-modeled capabilities
/// (interactive + refreshable + revocable + testable), so each matching
/// `register_*_ops` is wired; API-key and basic-auth advertise none and
/// take base ops only. A mismatch here would make
/// [`CredentialServiceBuilder::build`] return
/// [`CredentialServiceError::CapabilityWithoutOps`].
///
/// # Key provider (fail-closed in production)
///
/// Production reads a base64 AES-256 key from `NEBULA_CRED_MASTER_KEY`
/// (via [`EnvKeyProvider::from_env`]); an unset or malformed key fails
/// startup rather than silently using a weak key. For local development,
/// `NEBULA_CRED_DEV_KEY=1` selects a fixed in-process key so the service
/// boots without a configured master key — credentials are then **not**
/// securely encrypted, and a loud `warn!` says so. Never set it in
/// production.
///
/// # Storage durability
///
/// The raw store is in-memory: credential state is encrypted at rest but
/// does **not** survive a process restart. A durable backend is a future
/// swap behind the erased [`CredentialService`] collaborators.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if registry registration, key
/// provider init, dispatch-ops registration, or the final service build
/// fails. See each variant for the specific composition step.
pub fn try_default_credential_service()
-> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    with_key_provider(resolve_key_provider()?)
}

/// A fixed all-`0x42` 32-byte AES-256 key, base64-encoded — the dev-only key
/// selected by `NEBULA_CRED_DEV_KEY=1`. The public `from_base64` constructor
/// is used because the raw-byte `StaticKeyProvider` is test-gated and
/// `nebula_crypto::EncryptionKey` is not a dependency of this crate. This is a
/// deliberately weak dev placeholder, **not** a production secret.
const DEV_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

/// Resolve the encryption key provider: fail-closed `NEBULA_CRED_MASTER_KEY`
/// in production ([`EnvKeyProvider::from_env`]), or the fixed in-process dev
/// key behind an explicit `NEBULA_CRED_DEV_KEY=1` opt-in (with a loud `warn!`).
fn resolve_key_provider() -> Result<Arc<dyn KeyProvider>, CredentialServiceFactoryError> {
    if std::env::var("NEBULA_CRED_DEV_KEY").as_deref() == Ok("1") {
        tracing::warn!(
            "credential: NEBULA_CRED_DEV_KEY=1 — using a fixed in-process dev key; \
             credentials are NOT securely encrypted. Never set this in production."
        );
        Ok(Arc::new(EnvKeyProvider::from_base64(DEV_KEY_B64).map_err(
            |e| CredentialServiceFactoryError::KeyProvider(e.to_string()),
        )?))
    } else {
        Ok(Arc::new(EnvKeyProvider::from_env().map_err(|e| {
            CredentialServiceFactoryError::KeyProvider(e.to_string())
        })?))
    }
}

/// Build a [`CredentialService`] over the in-memory store with a
/// caller-supplied encryption key provider.
///
/// Separated from [`try_default_credential_service`] so a caller with its own
/// [`KeyProvider`] (a durable backend, or a test) composes the same service
/// without going through the `NEBULA_CRED_*` env resolution. Registers the
/// first-party type set (shared with the schema port via
/// `credential_schema_registry::default_registry`) and the matching
/// dispatch ops.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if registry registration,
/// dispatch-ops registration, or the final service build fails.
pub fn with_key_provider(
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    let registry = super::credential_schema_registry::default_registry()?;

    // Dispatch ops, fixed to the erased pending store so the engine resolver
    // and `DispatchOps` need no further monomorphization. Base ops for every
    // type; the four capability registrars only for the types that advertise
    // them (OAuth2). This set MUST match the registry's advertised caps or
    // `build()` returns `CapabilityWithoutOps`.
    let mut ops = DispatchOps::<ErasedPendingStore>::new();
    register_runtime_ops::<ApiKeyCredential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<BasicAuthCredential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_interactive_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_refreshable_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_revocable_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_testable_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;

    tracing::warn!(
        "credential: audit sink is log-only (target=nebula.credential.audit); \
         the audit trail is NOT durably persisted to a backend yet."
    );
    let audit_sink: Arc<dyn AuditSink> = Arc::new(TracingAuditSink);

    let pending = ErasedPendingStore::new(Arc::new(InMemoryPendingStore::new()));
    let raw_store = InMemoryStore::new();
    let observer = Arc::new(NoopObserver::new());
    let cache_config = CacheConfig::default();
    let lease_config = LeaseLifecycleConfig::default();
    // Process-wide shutdown token for the lease reaper. The token is owned by
    // the service; the lease task stops when the service (and so this token)
    // is dropped at process exit — `apps/server` carries no `tokio-util` dep,
    // so the factory mints the token internally.
    let shutdown = tokio_util::sync::CancellationToken::new();

    let service = CredentialServiceBuilder::new(
        raw_store,
        key_provider,
        audit_sink,
        cache_config,
        pending,
        Arc::new(registry),
        Arc::new(ops),
        observer,
        lease_config,
        shutdown,
    )
    .build()?;

    tracing::info!(
        "credential: CredentialService composed (encrypted-at-rest in-memory store; \
         non-durable across restart)"
    );
    Ok(Arc::new(service))
}
