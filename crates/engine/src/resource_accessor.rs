//! Engine-side [`ResourceAccessor`] implementation.
//!
//! [`EngineResourceAccessor`] bridges the engine's resource manager to the
//! [`ResourceAccessor`] capability trait consumed by actions. Acquire runs
//! the full manager lease pipeline (slot-identity-pinned, scope-aware) and
//! returns a boxed [`nebula_resource::ResourceGuard`] for downcast by action
//! code — not a raw `ManagedResource` handle.

use std::{any::Any, collections::HashMap, fmt, future::Future, pin::Pin, sync::Arc};

use nebula_core::{CoreError, ResourceKey, accessor::ResourceAccessor, scope::Scope};
use nebula_resource::{
    AcquireOptions, ErrorKind, Manager, ResourceContext, SLOT_IDENTITY_UNBOUND,
    dedup::slot_identity,
};
use tokio_util::sync::CancellationToken;

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Engine-side implementation of [`ResourceAccessor`].
///
/// Wraps an [`Arc<nebula_resource::Manager>`] and dispatches `acquire_any` /
/// `try_acquire_any` through [`Manager::acquire_erased`] using the execution
/// scope and optional per-key slot identities recorded at activation.
pub struct EngineResourceAccessor {
    manager: Arc<Manager>,
    scope: Scope,
    cancel: CancellationToken,
    slot_identities: Arc<HashMap<ResourceKey, u64>>,
}

impl EngineResourceAccessor {
    /// Creates a new accessor backed by the given resource manager.
    #[must_use]
    pub fn new(manager: Arc<Manager>, scope: Scope, cancel: CancellationToken) -> Self {
        Self {
            manager,
            scope,
            cancel,
            slot_identities: Arc::new(HashMap::new()),
        }
    }

    /// Overrides the default slot-identity map (key → resolved credential hash).
    #[must_use]
    pub fn with_slot_identities(mut self, slot_identities: HashMap<ResourceKey, u64>) -> Self {
        self.slot_identities = Arc::new(slot_identities);
        self
    }

    fn slot_identity_for(&self, key: &ResourceKey) -> u64 {
        self.slot_identities
            .get(key)
            .copied()
            .unwrap_or(SLOT_IDENTITY_UNBOUND)
    }

    fn resource_ctx(&self) -> ResourceContext {
        ResourceContext::minimal(self.scope.clone(), self.cancel.clone())
    }

    fn map_err(key: &ResourceKey, err: nebula_resource::Error) -> CoreError {
        let detail = format!("{}: {err}", key.as_str());
        match err.kind() {
            ErrorKind::NotFound => CoreError::CredentialNotFound { key: detail },
            ErrorKind::Ambiguous => CoreError::scope_violation(key.as_str(), detail),
            _ => CoreError::CredentialNotConfigured(detail),
        }
    }
}

impl fmt::Debug for EngineResourceAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineResourceAccessor")
            .field("manager", &"<Manager>")
            .field("scope", &self.scope)
            .finish()
    }
}

impl ResourceAccessor for EngineResourceAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.manager
            .has_registered_for_scope(key, &self.scope, self.slot_identity_for(key))
    }

    fn acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
        let manager = Arc::clone(&self.manager);
        let key = key.clone();
        let ctx = self.resource_ctx();
        let slot_identity = self.slot_identity_for(&key);
        let options = AcquireOptions::default();
        Box::pin(async move {
            Manager::acquire_erased(manager, &key, &ctx, &options, slot_identity)
                .await
                .map_err(|e| Self::map_err(&key, e))
        })
    }

    fn try_acquire_any(
        &self,
        key: &ResourceKey,
    ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
        let fut = self.acquire_any(key);
        Box::pin(async move {
            match fut.await {
                Ok(value) => Ok(Some(value)),
                Err(CoreError::CredentialNotFound { .. }) => Ok(None),
                Err(err) => Err(err),
            }
        })
    }
}

/// Build slot identities for activation from resolved `(slot, credential)` pairs.
#[must_use]
pub fn slot_identities_for_key(
    key: ResourceKey,
    pairs: &[(&str, &str)],
) -> HashMap<ResourceKey, u64> {
    let id = slot_identity(pairs.iter().copied());
    let mut map = HashMap::new();
    map.insert(key, id);
    map
}

#[cfg(test)]
mod tests {
    use std::{
        fmt,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use nebula_resource::{
        Manager, ResidentConfig, ResourceContext, ScopeLevel,
        dedup::SLOT_IDENTITY_UNBOUND,
        error::Error,
        resource::{Resource, ResourceConfig, ResourceMetadata},
        runtime::{TopologyRuntime, resident::ResidentRuntime},
        topology::resident::Resident,
    };

    use super::*;

    #[derive(Debug, Clone)]
    struct AccError(String);

    impl fmt::Display for AccError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for AccError {}

    impl From<AccError> for Error {
        fn from(e: AccError) -> Self {
            Error::permanent(e.0)
        }
    }

    #[derive(Clone, Debug, Default)]
    struct AccConfig;

    nebula_schema::impl_empty_has_schema!(AccConfig);

    impl ResourceConfig for AccConfig {}

    #[derive(Clone)]
    struct AccResource;

    impl Resource for AccResource {
        type Config = AccConfig;
        type Runtime = Arc<AtomicU64>;
        type Lease = Arc<AtomicU64>;
        type Error = AccError;

        fn key() -> ResourceKey {
            ResourceKey::new("test.engine_accessor.acc").expect("valid resource key in test")
        }

        async fn create(
            &self,
            _config: &AccConfig,
            _ctx: &ResourceContext,
        ) -> Result<Arc<AtomicU64>, AccError> {
            Ok(Arc::new(AtomicU64::new(42)))
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for AccResource {
        fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
            true
        }
    }

    fn make_accessor(manager: Arc<Manager>) -> EngineResourceAccessor {
        EngineResourceAccessor::new(manager, Scope::default(), CancellationToken::new())
    }

    fn rk(key: &str) -> ResourceKey {
        ResourceKey::new(key).expect("valid resource key in test")
    }

    #[tokio::test]
    async fn has_returns_false_for_unregistered_key() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        assert!(!accessor.has(&rk("postgres")));
    }

    #[tokio::test]
    async fn acquire_any_returns_err_for_unregistered_key() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        let result = accessor.acquire_any(&rk("postgres")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialNotFound { .. })),
            "expected CredentialNotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn try_acquire_any_returns_none_for_unregistered_key() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        let result = accessor.try_acquire_any(&rk("postgres")).await;
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn acquire_any_returns_guard_for_registered_resource() {
        let manager = Arc::new(Manager::new());
        manager
            .register(
                AccResource,
                AccConfig,
                ScopeLevel::Global,
                TopologyRuntime::Resident(ResidentRuntime::<AccResource>::new(
                    ResidentConfig::default(),
                )),
                Manager::erased_acquire_resident::<AccResource>(SLOT_IDENTITY_UNBOUND),
                None,
                None,
            )
            .expect("register");

        let accessor = make_accessor(Arc::clone(&manager));
        let key = AccResource::key();
        let boxed = accessor
            .acquire_any(&key)
            .await
            .expect("acquire through accessor");
        let guard = boxed
            .downcast::<nebula_resource::ResourceGuard<AccResource>>()
            .expect("ResourceGuard downcast");
        assert_eq!(guard.load(Ordering::Relaxed), 42);
    }

    #[tokio::test]
    async fn debug_redacts_manager() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        let debug = format!("{accessor:?}");
        assert!(debug.contains("<Manager>"));
    }
}
