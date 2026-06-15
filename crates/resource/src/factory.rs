//! Object-safe, type-erased `ResourceFactory` contribution contract.
//!
//! `ResourceFactory` is the **B+ merged contribution contract** for the
//! resource arm of the plugin system (ADR-0095 D2). It carries **both**:
//!
//! - **Introspection arm** — `key()`, `metadata()`, `validate()`:
//!   side-effect-free, callable from a catalog UI or install-from-repo
//!   pipeline without constructing anything.
//! - **Construction arm** — `register(&Manager, RegisterRequest)`:
//!   the erased, object-safe entry point that constructs and registers a
//!   live typed `R` against the given `Manager`.
//!
//! This replaces the former `ResourceDescriptor` (describe-only, no
//! construction) and the former engine-owned `ResourceActivator` (construct
//! only, no describe) — one type, one fact, one place.
//!
//! # Erasure mechanism
//!
//! The `register` method returns a boxed future (`BoxFut`) so the trait
//! stays object-safe without `#[async_trait]`. This matches the established
//! engine convention for erased async traits that bridge into `Manager`
//! (see `EngineResourceAccessor`'s `Pin<Box<dyn Future + Send>>` shape):
//! object safety is preserved without an attribute macro, and the erased
//! boundary stays allocation-explicit.
//!
//! # Per-`R` implementation
//!
//! [`KindActivator`] is the per-`R` implementor. It closes over two factories:
//! one that produces the `R` value and one that produces `R::Topology`. Both
//! are factories (not stored values) so one `KindActivator` can be invoked
//! multiple times (re-activation, multiple scopes). The `#[derive(Resource)]`
//! macro emits a `<Name>Factory` newtype that wraps a `KindActivator` with
//! the topology kind fixed by a `#[topology(Pooled|Resident, ...)]` attribute
//! on the derive.
//!
//! # Three frozen laws (CI-enforced)
//!
//! 1. **Schema-single-source** — `metadata().base.schema` derives from the
//!    same `<R::Config as HasSchema>::schema()` that `validate` and `register`
//!    use.
//! 2. **Removal-funnel** — raw `Manager` mutation is the caller's concern;
//!    teardown must route through a `PluginHandle` (engine-side).
//! 3. **Key-coherence** — `factory.key() == <R as Provider>::key()`.
//!
//! # Latent-by-design on landing
//!
//! The `register` arm ships with zero production callers (only tests call
//! typed registration today; `api/state.rs` holds the registry for config
//! validation only, never live-registration). This is correct and intentional
//! — the bind-population producer that gives `register` a production caller
//! is the named M12.4 follow-up.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use crate::resource::ResourceMetadata;
use crate::topology::Topology;
use crate::{Manager, ScopeLevel, SlotIdentity, recovery::RecoveryGate, resource::Provider};

/// Boxed, `Send` future returned across the erased factory boundary.
///
/// Mirrors the `BoxFut` alias used by `EngineResourceAccessor` so both erased
/// `Manager` bridges share one async-erasure shape.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type-agnostic inputs the *caller* threads into a typed registration.
///
/// Everything here is independent of the concrete resource type `R`; the
/// per-`R` pieces (`resource: R`, `R::Topology`) are closed over by the
/// factory (see [`KindActivator`]). Borrowed for the duration of the call so
/// the factory can be invoked without cloning the expression engine.
pub struct RegisterRequest<'a> {
    /// Opaque resource-specific config (the stored `ResourceEntry.config`
    /// JSON), resolved + schema-validated inside the typed manager call.
    pub config_json: serde_json::Value,
    /// Engine-held expression engine used to resolve `{{ … }}` templates in
    /// `config_json`.
    pub expr_engine: &'a nebula_expression::ExpressionEngine,
    /// Slot-name → resolved-credential-key bindings. The engine resolves
    /// credentials before calling `register`; this map is asserted against
    /// the resource's declared slots inside the typed call.
    pub slot_bindings: HashMap<String, nebula_core::CredentialKey>,
    /// Slot-name → resolved `CredentialId` for the rotation fan-out reverse
    /// index. Empty when the caller resolved no credential into a slot, or
    /// when no fan-out index is available — registration then proceeds with
    /// no reverse-index row.
    pub credential_ids: HashMap<String, nebula_credential::CredentialId>,
    /// Registration scope.
    pub scope: ScopeLevel,
    /// Optional recovery gate shared across a recovery group.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
}

/// Errors raised by the closed allowlist `kind → factory` bridge.
///
/// `UnknownKind` is a caller/wiring fault caught at activation — the `kind`
/// string was never inserted into the registry. Classified as a client
/// conflict (non-retryable): retrying the same byte sequence cannot succeed
/// until the operator registers the kind or fixes the stored row.
///
/// `Register` wraps the inner [`crate::Error`] from the typed
/// `register_resolved` and classifies by delegation, so a deserialize /
/// schema / validation failure keeps its original category.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistrarError {
    /// The `kind` string is not present in the closed allowlist.
    ///
    /// Caught at activation; the operator must register the kind or correct
    /// the stored resource row. Never auto-retried.
    #[error(
        "unknown resource kind `{0}`: not present in the closed factory \
         allowlist — register the kind before activating resources of \
         this kind, or correct the stored resource row"
    )]
    UnknownKind(String),

    /// The typed `Manager::register_resolved` call failed.
    ///
    /// Carries the underlying resource error (deserialize / schema /
    /// validation / slot-binding mismatch); classification delegates to it.
    #[error("registration of resource kind `{kind}` failed: {source}")]
    Register {
        /// The `kind` whose typed registration failed.
        kind: String,
        /// The underlying resource-subsystem error.
        #[source]
        source: crate::Error,
    },
}

impl nebula_error::Classify for RegistrarError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            // Caller/wiring fault caught at activation: non-retryable client
            // conflict. Consistent with ErrorKind::Ambiguous → Conflict.
            Self::UnknownKind(_) => nebula_error::ErrorCategory::Conflict,
            // Delegate to the inner error so schema / deserialize / slot
            // failures keep their own categories.
            Self::Register { source, .. } => nebula_error::Classify::category(source),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::UnknownKind(_) => nebula_error::ErrorCode::new("RESOURCE:FACTORY_UNKNOWN_KIND"),
            Self::Register { source, .. } => nebula_error::Classify::code(source),
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::UnknownKind(_) => false,
            Self::Register { source, .. } => nebula_error::Classify::is_retryable(source),
        }
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::UnknownKind(_) => None,
            Self::Register { source, .. } => nebula_error::Classify::retry_hint(source),
        }
    }
}

/// Metadata returned after a successful live registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRegistrationOutcome {
    /// Catalog key the resource was registered under.
    pub resource_key: nebula_core::ResourceKey,
    /// The **collision-free structural** slot identity the manager derived
    /// from `slot_bindings` — the exact value `Manager::register_resolved`
    /// returned (single construction site, no dual-derive divergence risk).
    pub slot_identity: SlotIdentity,
}

/// Object-safe, type-erased **B+ merged contribution contract** for one
/// resource type.
///
/// Carries the introspection arm (`key`, `metadata`, `validate`) and the
/// construction arm (`register`). Object-safe: stored as
/// `Arc<dyn ResourceFactory>` in `Plugin::resources()`.
///
/// # Key coherence invariant
///
/// Implementations MUST satisfy `self.key() == <R as Provider>::key()`.
/// The derive-emitted `<Name>Factory` enforces this structurally (it
/// delegates to `<R as Provider>::key()`).
pub trait ResourceFactory: Send + Sync + 'static {
    /// The static `ResourceKey` identifying the concrete resource type.
    ///
    /// Pure and side-effect-free — a `const`-ish accessor, not I/O.
    fn key(&self) -> nebula_core::ResourceKey;

    /// Resource metadata for catalog display, schema introspection, and
    /// install-from-repo pre-install enumeration.
    ///
    /// Side-effect-free. The schema MUST derive from the same
    /// `<R::Config as HasSchema>::schema()` that `validate` and `register`
    /// use (schema-single-source law).
    fn metadata(&self) -> ResourceMetadata;

    /// Validate `config_json` against this resource's `R::Config` schema
    /// **without registering anything**.
    ///
    /// Runs the same schema pass + closed-set guard + `R::Config` deserialize
    /// as the live `register` path (shared via `Manager::validate_config_value`),
    /// but performs **no** `Manager` mutation, constructs **no** `resource: R`
    /// or `R::Topology`, and resolves **no** `{{ … }}` templates.
    ///
    /// Synchronous: validation is pure (schema + serde, no I/O).
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error`] verbatim if the config fails the `R::Config`
    /// schema, carries an undeclared field, or fails to deserialize.
    fn validate(&self, config_json: serde_json::Value) -> Result<(), crate::Error>;

    /// Construct and register this resource type against `manager` using the
    /// caller-threaded [`RegisterRequest`] plus the per-`R` resource and
    /// topology this factory owns.
    ///
    /// On success returns the **collision-free structural** [`SlotIdentity`]
    /// the manager derived for this row — the exact value
    /// `Manager::register_resolved` returned — so the caller can record the
    /// row key without an independent recompute.
    ///
    /// The returned future is boxed so the trait stays object-safe without
    /// `#[async_trait]`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error`] verbatim if the typed `register_resolved`
    /// fails (deserialize / schema / validation / slot-binding mismatch).
    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, crate::Error>>;
}

/// Per-`R` [`ResourceFactory`] that closes over the pieces the erased
/// boundary cannot synthesize generically: a factory that produces the
/// `resource: R` value and a factory that produces `R::Topology`.
///
/// Both are factories rather than stored values so one `KindActivator` can be
/// invoked multiple times (re-activation, multiple scopes). The topology
/// factory builds an `R::Topology` via `Resident::new(...)` or
/// `Pooled::new(...)` — acquire dispatch is baked into the topology at
/// construction.
///
/// Used directly by the engine's `ResourceActivatorRegistry` and emitted
/// internally by the `#[derive(Resource)]`-generated `<Name>Factory`.
pub struct KindActivator<R, FRes, FTopo>
where
    R: Provider + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> R::Topology + Send + Sync,
{
    resource_factory: FRes,
    topology_factory: FTopo,
    // Zero-sized marker — `R` is not stored but bounds the `impl`.
    _marker: std::marker::PhantomData<fn() -> R>,
}

impl<R, FRes, FTopo> KindActivator<R, FRes, FTopo>
where
    R: Provider + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> R::Topology + Send + Sync,
{
    /// Builds a `KindActivator` for resource type `R`.
    ///
    /// - `resource_factory` — yields the `R` value with credential slots
    ///   already resolved by the engine per registration scope.
    /// - `topology_factory` — yields the `R::Topology` for this kind. Use
    ///   `Resident::new(...)` for resident topologies or `Pooled::new(...)`
    ///   for pooled ones; acquire dispatch is baked into the topology at
    ///   construction.
    pub fn new(resource_factory: FRes, topology_factory: FTopo) -> Self {
        Self {
            resource_factory,
            topology_factory,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<R, FRes, FTopo> ResourceFactory for KindActivator<R, FRes, FTopo>
where
    R: Provider + crate::resource::HasCredentialSlots + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    R::Instance: Clone,
    R::Topology: Topology<R>,
    FRes: Fn() -> R + Send + Sync + 'static,
    FTopo: Fn() -> R::Topology + Send + Sync + 'static,
{
    fn key(&self) -> nebula_core::ResourceKey {
        <R as Provider>::key()
    }

    fn metadata(&self) -> ResourceMetadata {
        <R as Provider>::metadata()
    }

    fn validate(&self, config_json: serde_json::Value) -> Result<(), crate::Error> {
        // No resource_factory / topology_factory invoked: validation is purely
        // a function of the config JSON and the monomorphized `R::Config`
        // schema — exactly the live path's pre-register checks, minus template
        // resolution and typed-runtime construction.
        Manager::validate_config_value::<R>(config_json).map(|_| ())
    }

    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, crate::Error>> {
        let resource = (self.resource_factory)();
        let topology = (self.topology_factory)();
        Box::pin(async move {
            manager
                .register_resolved::<R>(
                    request.config_json,
                    request.expr_engine,
                    request.slot_bindings,
                    resource,
                    request.scope,
                    topology,
                    request.recovery_gate,
                )
                .await
        })
    }
}

/// Closed allowlist mapping a resource `kind` string to its erased factory.
///
/// The map is the only path from a stored resource row to a typed
/// registration. There is no fallback, no reflection, and no dynamic type
/// construction: a `kind` is registrable only if it was explicitly
/// [`insert`](Self::insert)ed. [`register`](Self::register) on an unknown
/// kind returns [`RegistrarError::UnknownKind`] — a typed, matchable
/// activation error, never a panic or a silent grab of the wrong type.
#[derive(Default)]
pub struct ResourceActivatorRegistry {
    factories: HashMap<String, Arc<dyn ResourceFactory>>,
}

impl ResourceActivatorRegistry {
    /// Creates an empty registry. Fail-closed by construction: rejects every
    /// kind until at least one factory is inserted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts (or replaces) the factory for `kind`.
    ///
    /// Returns the previously registered factory for this kind, if any, so
    /// the caller can detect an unintended override.
    pub fn insert(
        &mut self,
        kind: impl Into<String>,
        factory: Arc<dyn ResourceFactory>,
    ) -> Option<Arc<dyn ResourceFactory>> {
        self.factories.insert(kind.into(), factory)
    }

    /// Returns `true` if `kind` is in the allowlist.
    #[must_use]
    pub fn contains(&self, kind: &str) -> bool {
        self.factories.contains_key(kind)
    }

    /// Number of registered kinds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Whether the allowlist is empty (rejects every kind).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    /// Resolves `kind` through the closed allowlist and dispatches into its
    /// typed factory.
    ///
    /// # Errors
    ///
    /// - [`RegistrarError::UnknownKind`] — `kind` not in allowlist; never
    ///   auto-retried, classified as a client conflict.
    /// - [`RegistrarError::Register`] — typed `register_resolved` failed;
    ///   classification delegates to the inner error.
    pub async fn register(
        &self,
        kind: &str,
        manager: &Manager,
        request: RegisterRequest<'_>,
    ) -> Result<ResourceRegistrationOutcome, RegistrarError> {
        let factory = self
            .factories
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        let resource_key = factory.key();
        let slot_identity = factory.register(manager, request).await.map_err(|source| {
            RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            }
        })?;
        Ok(ResourceRegistrationOutcome {
            resource_key,
            slot_identity,
        })
    }

    /// Resolves `kind` through the closed allowlist, dispatches into its
    /// typed factory, and on success records the resolved row in the rotation
    /// fan-out reverse index.
    ///
    /// The reverse-index bind is recorded **before** the typed `register`
    /// call makes the `Manager` row discoverable, and a failed `register`
    /// removes the staged bind via RAII compensation (no orphan reverse-index
    /// rows). See `registrar.rs`'s documentation for the full ordering
    /// argument (bind-before-publish → no silent live-row miss; scopeguard
    /// compensation → no orphan on failure; residual pre-publish window
    /// documented there).
    ///
    /// Feature-gated with the reverse index itself (`rotation`).
    ///
    /// # Errors
    ///
    /// Same as [`register`](Self::register). The bind step is an in-memory
    /// insert and cannot itself fail.
    #[cfg(feature = "rotation")]
    pub async fn register_and_bind(
        &self,
        kind: &str,
        manager: &Manager,
        request: RegisterRequest<'_>,
        fanout_index: Option<&crate::ResourceFanoutIndex>,
    ) -> Result<ResourceRegistrationOutcome, RegistrarError> {
        let factory = self
            .factories
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        let resource_key = factory.key();
        let staged_slot_identity = SlotIdentity::from_bindings(
            request
                .slot_bindings
                .iter()
                .map(|(slot, cred)| (slot.as_str(), cred.as_str())),
        );

        // Stage reverse-index binds BEFORE the typed register makes the
        // Manager row discoverable.
        let mut staged: Vec<(nebula_credential::CredentialId, _)> = Vec::new();
        if let Some(idx) = fanout_index {
            for (slot_name, cred_id) in &request.credential_ids {
                if !request.slot_bindings.contains_key(slot_name) {
                    continue;
                }
                let bind = crate::Bind {
                    resource_key: resource_key.clone(),
                    scope: request.scope.clone(),
                    slot_name: slot_name.clone(),
                    slot_identity: staged_slot_identity.clone(),
                };
                idx.bind(
                    *cred_id,
                    resource_key.clone(),
                    request.scope.clone(),
                    slot_name.clone(),
                    staged_slot_identity.clone(),
                );
                staged.push((*cred_id, bind));
            }
        }

        // RAII compensation: remove staged binds if register fails.
        let rollback = scopeguard::guard((fanout_index, staged), |(idx, staged)| {
            if let Some(idx) = idx {
                for (cred_id, bind) in &staged {
                    idx.unbind_staged_entry(cred_id, bind);
                }
            }
        });

        let slot_identity = factory.register(manager, request).await.map_err(|source| {
            RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            }
        })?;

        debug_assert_eq!(
            slot_identity, staged_slot_identity,
            "register_resolved returned a slot identity that diverged from \
             the canonical from_bindings recompute — cross-tenant aliasing risk"
        );
        scopeguard::ScopeGuard::into_inner(rollback);
        Ok(ResourceRegistrationOutcome {
            resource_key,
            slot_identity,
        })
    }

    /// Resolves `kind` through the closed allowlist and validates `config_json`
    /// against that kind's `R::Config` schema **without registering anything**.
    ///
    /// The config-CRUD seam: a writer persisting a resource definition validates
    /// the config here *before* the row is stored. Validation is live-register
    /// equivalent (schema + deserialize) but mutates no `Manager` and resolves
    /// no templates.
    ///
    /// # Errors
    ///
    /// - [`RegistrarError::UnknownKind`] — not in allowlist; non-retryable client conflict.
    /// - [`RegistrarError::Register`] — config fails schema / deserialize.
    pub fn validate(
        &self,
        kind: &str,
        config_json: serde_json::Value,
    ) -> Result<(), RegistrarError> {
        let factory = self
            .factories
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        factory
            .validate(config_json)
            .map_err(|source| RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use nebula_core::{ResourceKey, resource_key};
    use nebula_error::{Classify, ErrorCategory};
    use nebula_expression::ExpressionEngine;

    use super::*;
    use crate::{
        Manager, Resident, ScopeLevel,
        error::Error as ResourceError,
        resource::{Provider, ResourceConfig, ResourceMetadata},
        topology::resident::{self, ResidentProvider},
    };

    // ── Minimal test resource ────────────────────────────────────────────────

    #[derive(Debug, Clone)]
    struct TestError(String);

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for TestError {}

    impl From<TestError> for ResourceError {
        fn from(e: TestError) -> Self {
            ResourceError::transient(e.0)
        }
    }

    #[derive(Clone, Debug, serde::Deserialize)]
    struct TestConfig {
        #[serde(default)]
        name: String,
    }

    nebula_schema::impl_empty_has_schema!(TestConfig);

    impl ResourceConfig for TestConfig {
        fn validate(&self) -> Result<(), ResourceError> {
            if self.name.is_empty() {
                return Err(ResourceError::permanent("name must not be empty"));
            }
            Ok(())
        }

        fn fingerprint(&self) -> u64 {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.name.hash(&mut h);
            h.finish()
        }
    }

    #[derive(Clone)]
    struct TestRes {
        create_counter: Arc<AtomicU64>,
    }

    impl TestRes {
        fn new(create_counter: Arc<AtomicU64>) -> Self {
            Self { create_counter }
        }
    }

    #[async_trait::async_trait]
    impl Provider for TestRes {
        type Config = TestConfig;
        type Instance = Arc<AtomicU64>;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("test-factory-res")
        }

        async fn create(
            &self,
            _config: &TestConfig,
            _ctx: &crate::ResourceContext,
        ) -> Result<Arc<AtomicU64>, ResourceError> {
            let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::new(
                Self::key(),
                "test-factory-res".to_owned(),
                String::new(),
                <TestConfig as nebula_schema::HasSchema>::schema(),
            )
        }
    }

    impl nebula_core::DeclaresDependencies for TestRes {}

    impl crate::HasCredentialSlots for TestRes {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    #[async_trait::async_trait]
    impl ResidentProvider for TestRes {
        fn is_alive_sync(&self, runtime: &Arc<AtomicU64>) -> bool {
            runtime.load(Ordering::Relaxed) < u64::MAX
        }
    }

    fn test_factory(create_counter: Arc<AtomicU64>) -> Arc<dyn ResourceFactory> {
        Arc::new(KindActivator::<TestRes, _, _>::new(
            move || TestRes::new(create_counter.clone()),
            || Resident::<TestRes>::new(resident::config::Config::default()),
        ))
    }

    fn request(expr_engine: &ExpressionEngine) -> RegisterRequest<'_> {
        RegisterRequest {
            config_json: serde_json::json!({ "name": "from-factory" }),
            expr_engine,
            slot_bindings: HashMap::new(),
            credential_ids: HashMap::new(),
            scope: ScopeLevel::Global,
            recovery_gate: None,
        }
    }

    // ── Factory introspection arm ────────────────────────────────────────────

    #[test]
    fn key_coherence_law() {
        let create_counter = Arc::new(AtomicU64::new(0));
        let factory = test_factory(create_counter);
        assert_eq!(
            factory.key(),
            TestRes::key(),
            "factory.key() must equal <R as Provider>::key() (key-coherence law)"
        );
    }

    #[test]
    fn metadata_schema_matches_provider_schema() {
        let create_counter = Arc::new(AtomicU64::new(0));
        let factory = test_factory(create_counter);
        let md = factory.metadata();
        let expected = TestRes::metadata();
        assert_eq!(
            md.base.schema, expected.base.schema,
            "metadata().schema must derive from the same HasSchema as validate/register \
             (schema-single-source law)"
        );
    }

    // ── Construction arm ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn known_kind_registers_against_manager() {
        let manager = Manager::new();
        let expr_engine = ExpressionEngine::with_cache_size(16);
        let create_counter = Arc::new(AtomicU64::new(0));

        let mut registry = ResourceActivatorRegistry::new();
        let prev = registry.insert("test-kind", test_factory(create_counter));
        assert!(prev.is_none(), "no prior factory for a fresh kind");
        assert!(registry.contains("test-kind"));
        assert_eq!(registry.len(), 1);

        registry
            .register("test-kind", &manager, request(&expr_engine))
            .await
            .expect("known kind registers via the typed manager call");

        assert!(
            manager
                .get_any(&TestRes::key(), &ScopeLevel::Global)
                .is_some(),
            "registered resource must be resolvable in the manager"
        );
    }

    #[tokio::test]
    async fn unknown_kind_is_typed_error_not_panic_not_silent() {
        let manager = Manager::new();
        let expr_engine = ExpressionEngine::with_cache_size(16);
        let create_counter = Arc::new(AtomicU64::new(0));

        let mut registry = ResourceActivatorRegistry::new();
        registry.insert("test-kind", test_factory(create_counter));

        let err = registry
            .register("ghost", &manager, request(&expr_engine))
            .await
            .expect_err("unknown kind must NOT register and must NOT panic");

        match &err {
            RegistrarError::UnknownKind(kind) => assert_eq!(kind, "ghost"),
            other => panic!("expected UnknownKind(\"ghost\"), got {other:?}"),
        }

        assert!(
            manager
                .get_any(&TestRes::key(), &ScopeLevel::Global)
                .is_none(),
            "an unknown kind must never have touched a resource type"
        );
    }

    #[test]
    fn unknown_kind_classifies_as_nonretryable_client_conflict() {
        let err = RegistrarError::UnknownKind("ghost".to_owned());

        assert_eq!(Classify::category(&err), ErrorCategory::Conflict);
        assert_ne!(Classify::category(&err), ErrorCategory::Internal);
        assert!(ErrorCategory::Conflict.is_client_error());
        assert!(!ErrorCategory::Conflict.is_server_error());
        assert!(!Classify::is_retryable(&err));
        assert!(!ErrorCategory::Conflict.is_default_retryable());
        assert!(Classify::retry_hint(&err).is_none());
        assert_eq!(
            Classify::code(&err).as_str(),
            "RESOURCE:FACTORY_UNKNOWN_KIND"
        );
    }

    #[test]
    fn register_error_delegates_classification_to_inner() {
        let err = RegistrarError::Register {
            kind: "test-kind".to_owned(),
            source: ResourceError::permanent("schema validation failed"),
        };
        let inner = ResourceError::permanent("schema validation failed");
        assert_eq!(
            Classify::category(&err),
            Classify::category(&inner),
            "Register must delegate its category to the inner resource error"
        );
        assert_eq!(
            Classify::code(&err).as_str(),
            Classify::code(&inner).as_str()
        );
    }

    #[tokio::test]
    async fn empty_registry_rejects_every_kind() {
        let manager = Manager::new();
        let expr_engine = ExpressionEngine::with_cache_size(16);
        let registry = ResourceActivatorRegistry::new();
        assert!(registry.is_empty());

        let err = registry
            .register("anything", &manager, request(&expr_engine))
            .await
            .expect_err("an empty allowlist is fail-closed");
        assert!(matches!(err, RegistrarError::UnknownKind(k) if k == "anything"));
    }
}
