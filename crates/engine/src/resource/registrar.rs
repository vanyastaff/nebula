//! Closed-allowlist `kind → typed registrar` bridge.
//!
//! [`nebula_resource::Manager::register_from_value`] is a *typed* entry
//! point (`register_from_value::<R>`): it monomorphizes on the concrete
//! resource type so it can deserialize `R::Config`, schema-validate it,
//! and build a `TopologyRuntime<R>`. A stored resource row only carries a
//! `kind` string and opaque JSON, so the engine needs an erased
//! indirection: a per-kind object-safe registrar that already knows its
//! concrete `R` and dispatches into the typed manager call.
//!
//! The mapping is a **closed allowlist**. The engine performs no
//! reflection on the `kind` string and never materializes a resource
//! type dynamically — a kind is registrable only if it was explicitly
//! inserted. An unknown kind is a caller/wiring misconfiguration caught
//! at activation and surfaced as a typed, matchable error
//! ([`RegistrarError::UnknownKind`]); it is never a silent grab of the
//! wrong type nor a panic (INTEGRATION_MODEL §114-120 — misconfiguration
//! caught at activation; ADR-0030 typed-error discipline; ADR-0036
//! isolation; ADR-0044 slot model).
//!
//! # Erasure mechanism
//!
//! The trait returns a boxed future from a non-`async` method rather than
//! using `#[async_trait]`. This matches the established engine convention
//! for erased async traits that bridge into `nebula_resource::Manager`
//! (see `crate::resource_accessor::EngineResourceAccessor`, which uses
//! the same `Pin<Box<dyn Future + Send>>` return shape): object safety is
//! preserved without an attribute macro, and the erased boundary stays
//! allocation-explicit.
//!
//! # What the erased boundary can and cannot supply
//!
//! [`Manager::register_from_value::<R>`] requires, per its current
//! signature:
//!
//! ```text
//! register_from_value(
//!     config_json:   serde_json::Value,
//!     expr_engine:   &nebula_expression::ExpressionEngine,
//!     slot_bindings: HashMap<String, nebula_core::CredentialKey>,
//!     resource:      R,
//!     scope:         ScopeLevel,
//!     topology:      TopologyRuntime<R>,
//!     resilience:    Option<AcquireResilience>,
//!     recovery_gate: Option<Arc<RecoveryGate>>,
//! ) -> Result<(), nebula_resource::Error>
//! where R: Resource + DeclaresDependencies, R::Config: DeserializeOwned
//! ```
//!
//! Of those, only `config_json` is carried by the stored resource row.
//! `resource: R` and `topology: TopologyRuntime<R>` are **`R`-typed** and
//! cannot be constructed generically at the erased boundary — the
//! resource crate emits no `FromConfig`/constructor and no topology
//! factory from the `#[derive(Resource)]` macro. They must therefore be
//! closed over per concrete `R` when the registrar is built (that is what
//! [`TypedResourceRegistrar`] does: it holds a `resource`-producing
//! factory and a `TopologyRuntime<R>`-producing factory).
//!
//! `expr_engine`, `scope`, `slot_bindings`, `resilience`, and
//! `recovery_gate` are type-agnostic and supplied by the *caller* of
//! [`ErasedResourceRegistrar::register`] (the engine registration loop)
//! through [`RegisterRequest`]. This keeps the trait surface honest:
//! the registrar contributes only what is intrinsically per-`R`; the
//! caller threads everything that depends on engine/activation context.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use nebula_resource::{
    AcquireResilience, Manager, ScopeLevel, TopologyRuntime, recovery::RecoveryGate,
    resource::Resource,
};

/// Boxed, `Send` future returned across the erased registrar boundary.
///
/// Mirrors the `BoxFut` alias used by
/// `crate::resource_accessor::EngineResourceAccessor` so both erased
/// `nebula_resource::Manager` bridges share one async-erasure shape.
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Errors raised by the `kind → typed registrar` bridge.
///
/// `UnknownKind` is a caller/wiring fault caught at activation, **not**
/// an internal invariant breach: a resource row referenced a `kind`
/// string that was never wired into the closed allowlist. It is
/// classified as a client conflict (non-retryable) — the same family the
/// resource subsystem uses for the analogous "caller asked for something
/// the resolver cannot satisfy" fault (`ErrorKind::Ambiguous →
/// ErrorCategory::Conflict`). Retrying byte-for-byte cannot succeed; the
/// operator must register the kind or fix the row.
///
/// `Register` wraps the inner [`nebula_resource::Error`] from the typed
/// `register_from_value` and classifies by delegation, so a deserialize/
/// schema/validation failure keeps its original category instead of
/// being flattened.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistrarError {
    /// The `kind` string is not present in the closed allowlist.
    ///
    /// Caught at activation; the operator must register the kind or
    /// correct the stored resource row. Never auto-retried.
    #[error(
        "unknown resource kind `{0}`: not present in the closed registrar \
         allowlist — register the kind before activating resources of \
         this kind, or correct the stored resource row"
    )]
    UnknownKind(String),

    /// The typed `Manager::register_from_value` call failed.
    ///
    /// Carries the underlying resource error (deserialize / schema /
    /// validation / slot-binding mismatch); classification delegates to
    /// it.
    #[error("registration of resource kind `{kind}` failed: {source}")]
    Register {
        /// The `kind` whose typed registration failed.
        kind: String,
        /// The underlying resource-subsystem error.
        #[source]
        source: nebula_resource::Error,
    },
}

impl nebula_error::Classify for RegistrarError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            // Caller/wiring fault caught at activation, not a 5xx: the
            // requested `kind` is not in the closed allowlist. `Conflict`
            // is the client-error, non-retryable family (it is NOT a
            // server error — see `ErrorCategory::is_client_error` /
            // `is_server_error`), consistent with the resource
            // subsystem's `ErrorKind::Ambiguous → Conflict` precedent for
            // "caller asked for something the resolver cannot satisfy".
            Self::UnknownKind(_) => nebula_error::ErrorCategory::Conflict,
            // Preserve the inner resource error's category instead of
            // flattening it. (Today every register_from_value failure is
            // Error::permanent => Internal — deserialize, schema, and
            // slot-binding mismatch all land there; delegation keeps this
            // honest if the resource crate later differentiates those.)
            Self::Register { source, .. } => nebula_error::Classify::category(source),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::UnknownKind(_) => nebula_error::ErrorCode::new("ENGINE:RESOURCE_UNKNOWN_KIND"),
            Self::Register { source, .. } => nebula_error::Classify::code(source),
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            // Permanent caller error — retrying the same row cannot
            // succeed until the operator wires the kind.
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

/// Type-agnostic inputs the *caller* (the engine registration loop)
/// threads into a typed registration.
///
/// Everything here is independent of the concrete resource type `R`; the
/// per-`R` pieces (`resource: R`, `TopologyRuntime<R>`) are supplied by
/// the registrar itself (see [`TypedResourceRegistrar`]). Borrowed for
/// the duration of the call so the registry can be invoked without
/// cloning the expression engine.
pub struct RegisterRequest<'a> {
    /// Opaque resource-specific config (the stored `ResourceEntry.config`
    /// JSON), resolved + schema-validated inside the typed manager call.
    pub config_json: serde_json::Value,
    /// Engine-held expression engine used to resolve `{{ … }}` templates
    /// in `config_json`.
    pub expr_engine: &'a nebula_expression::ExpressionEngine,
    /// Slot-name → resolved-credential-key bindings (per ADR-0044). The
    /// engine resolves credentials and is expected to have folded them
    /// into the resource value the registrar produces; this map is
    /// asserted against the resource's declared slots inside the typed
    /// call.
    pub slot_bindings: HashMap<String, nebula_core::CredentialKey>,
    /// Registration scope.
    pub scope: ScopeLevel,
    /// Optional acquire-time resilience policy.
    pub resilience: Option<AcquireResilience>,
    /// Optional recovery gate shared across a recovery group.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
}

/// Object-safe, type-erased registrar for a single resource `kind`.
///
/// One implementor exists per concrete resource type; it already knows
/// its `R` and dispatches into the typed
/// [`Manager::register_from_value::<R>`]. The returned future is boxed so
/// the trait stays object-safe without `#[async_trait]` (matching
/// `EngineResourceAccessor`'s erasure shape).
pub trait ErasedResourceRegistrar: Send + Sync {
    /// Registers this kind's resource against `manager` using the
    /// caller-threaded [`RegisterRequest`] plus the per-`R` resource and
    /// topology this registrar owns.
    ///
    /// # Errors
    ///
    /// Returns the inner [`nebula_resource::Error`] verbatim if the typed
    /// `register_from_value` fails (deserialize / schema / validation /
    /// slot-binding mismatch). The registry wraps it in
    /// [`RegistrarError::Register`]; the closed-allowlist `UnknownKind`
    /// check happens in [`ResourceRegistrarRegistry::register`] before
    /// this is ever called.
    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<(), nebula_resource::Error>>;
}

/// Per-`R` [`ErasedResourceRegistrar`] that closes over the two pieces
/// the erased boundary cannot synthesize generically: a factory that
/// produces the `resource: R` value (with its credential slots already
/// resolved by the engine) and a factory that produces the
/// `TopologyRuntime<R>` declared for this kind.
///
/// `register_from_value` consumes both `resource: R` and
/// `TopologyRuntime<R>` by value, and a registrar may be invoked more
/// than once (re-activation, multiple scopes), so both are *factories*
/// rather than stored values.
pub struct TypedResourceRegistrar<R, FRes, FTopo>
where
    R: Resource + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> TopologyRuntime<R> + Send + Sync,
{
    resource_factory: FRes,
    topology_factory: FTopo,
}

impl<R, FRes, FTopo> TypedResourceRegistrar<R, FRes, FTopo>
where
    R: Resource + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> TopologyRuntime<R> + Send + Sync,
{
    /// Builds a typed registrar for resource type `R`.
    ///
    /// `resource_factory` yields the `R` value (with credential slots
    /// already resolved by the engine per ADR-0044);
    /// `topology_factory` yields the `TopologyRuntime<R>` declared for
    /// this kind. Both are invoked once per registration call.
    pub fn new(resource_factory: FRes, topology_factory: FTopo) -> Self {
        Self {
            resource_factory,
            topology_factory,
        }
    }
}

impl<R, FRes, FTopo> ErasedResourceRegistrar for TypedResourceRegistrar<R, FRes, FTopo>
where
    R: Resource + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> TopologyRuntime<R> + Send + Sync,
{
    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<(), nebula_resource::Error>> {
        let resource = (self.resource_factory)();
        let topology = (self.topology_factory)();
        Box::pin(async move {
            manager
                .register_from_value::<R>(
                    request.config_json,
                    request.expr_engine,
                    request.slot_bindings,
                    resource,
                    request.scope,
                    topology,
                    request.resilience,
                    request.recovery_gate,
                )
                .await
        })
    }
}

/// Closed allowlist mapping a resource `kind` string to its erased
/// registrar.
///
/// The map is the only path from a stored resource row to a typed
/// registration. There is no fallback, no reflection, and no dynamic
/// type construction: a `kind` is registrable only if it was explicitly
/// [`insert`](Self::insert)ed. [`register`](Self::register) on an unknown
/// kind returns [`RegistrarError::UnknownKind`] — a typed, matchable
/// activation error, never a panic or a silent grab of the wrong type
/// (INTEGRATION_MODEL §114-120).
#[derive(Default)]
pub struct ResourceRegistrarRegistry {
    registrars: HashMap<String, Arc<dyn ErasedResourceRegistrar>>,
}

impl ResourceRegistrarRegistry {
    /// Creates an empty registry.
    ///
    /// A registry with no entries rejects *every* kind with
    /// [`RegistrarError::UnknownKind`] — fail-closed by construction.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts (or replaces) the registrar for `kind`.
    ///
    /// Returns the previously registered registrar for this kind, if any,
    /// so the caller can detect an unintended override.
    pub fn insert(
        &mut self,
        kind: impl Into<String>,
        registrar: Arc<dyn ErasedResourceRegistrar>,
    ) -> Option<Arc<dyn ErasedResourceRegistrar>> {
        self.registrars.insert(kind.into(), registrar)
    }

    /// Returns `true` if `kind` is in the allowlist.
    #[must_use]
    pub fn contains(&self, kind: &str) -> bool {
        self.registrars.contains_key(kind)
    }

    /// Number of registered kinds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.registrars.len()
    }

    /// Whether the allowlist is empty (rejects every kind).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registrars.is_empty()
    }

    /// Resolves `kind` through the closed allowlist and dispatches into
    /// its typed registrar.
    ///
    /// # Errors
    ///
    /// - [`RegistrarError::UnknownKind`] if `kind` is not in the
    ///   allowlist. This is a caller/wiring fault caught at activation
    ///   (INTEGRATION_MODEL §114-120) — non-retryable, classified as a
    ///   client conflict. The lookup happens **before** any typed call,
    ///   so an unknown kind can never touch a resource type.
    /// - [`RegistrarError::Register`] if the typed
    ///   `Manager::register_from_value` fails (deserialize / schema /
    ///   validation / slot-binding mismatch); classification delegates to
    ///   the inner error.
    pub async fn register(
        &self,
        kind: &str,
        manager: &Manager,
        request: RegisterRequest<'_>,
    ) -> Result<(), RegistrarError> {
        let registrar = self
            .registrars
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        registrar
            .register(manager, request)
            .await
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
    use nebula_resource::{
        Manager, ScopeLevel,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
        runtime::{TopologyRuntime, resident::ResidentRuntime},
        topology::resident,
    };

    use super::*;

    // --- Minimal test resource ------------------------------------------

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
            // Exercises the deserialized field: confirms the JSON config
            // threaded through `register_from_value` was actually parsed
            // into `TestConfig` (mirrors the `name`-non-empty check used
            // by the resource crate's own integration mocks).
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

    impl Resource for TestRes {
        type Config = TestConfig;
        type Runtime = Arc<AtomicU64>;
        type Lease = Arc<AtomicU64>;
        type Error = TestError;

        fn key() -> ResourceKey {
            resource_key!("test-registrar-res")
        }

        fn create(
            &self,
            _config: &TestConfig,
            _ctx: &nebula_resource::ResourceContext,
        ) -> impl Future<Output = Result<Arc<AtomicU64>, TestError>> + Send {
            let counter = self.create_counter.clone();
            async move {
                let id = counter.fetch_add(1, Ordering::Relaxed);
                Ok(Arc::new(AtomicU64::new(id)))
            }
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::new(
                Self::key(),
                "test-registrar-res".to_owned(),
                String::new(),
                <TestConfig as nebula_schema::HasSchema>::schema(),
            )
        }
    }

    // `TestRes` declares no credential slots, so the default
    // `DeclaresDependencies` (empty) impl is correct.
    impl nebula_core::DeclaresDependencies for TestRes {}

    fn test_registrar(create_counter: Arc<AtomicU64>) -> Arc<dyn ErasedResourceRegistrar> {
        Arc::new(TypedResourceRegistrar::<TestRes, _, _>::new(
            move || TestRes::new(create_counter.clone()),
            || {
                TopologyRuntime::Resident(ResidentRuntime::<TestRes>::new(
                    resident::config::Config::default(),
                ))
            },
        ))
    }

    fn request(expr_engine: &ExpressionEngine) -> RegisterRequest<'_> {
        RegisterRequest {
            config_json: serde_json::json!({ "name": "from-registry" }),
            expr_engine,
            slot_bindings: HashMap::new(),
            scope: ScopeLevel::Global,
            resilience: None,
            recovery_gate: None,
        }
    }

    #[tokio::test]
    async fn known_kind_registers_against_manager() {
        let manager = Manager::new();
        let expr_engine = ExpressionEngine::with_cache_size(16);
        let create_counter = Arc::new(AtomicU64::new(0));

        let mut registry = ResourceRegistrarRegistry::new();
        let prev = registry.insert("test-kind", test_registrar(create_counter));
        assert!(prev.is_none(), "no prior registrar for a fresh kind");
        assert!(registry.contains("test-kind"));
        assert_eq!(registry.len(), 1);

        registry
            .register("test-kind", &manager, request(&expr_engine))
            .await
            .expect("known kind registers via the typed manager call");

        // The resource is now in the manager registry under its key.
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

        let mut registry = ResourceRegistrarRegistry::new();
        registry.insert("test-kind", test_registrar(create_counter));

        let err = registry
            .register("ghost", &manager, request(&expr_engine))
            .await
            .expect_err("unknown kind must NOT register and must NOT panic");

        // Typed and matchable — not a stringly opaque failure.
        match &err {
            RegistrarError::UnknownKind(kind) => assert_eq!(kind, "ghost"),
            other => panic!("expected UnknownKind(\"ghost\"), got {other:?}"),
        }

        // Not a silent grab: nothing was registered under any key.
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

        // Caller/wiring fault caught at activation — a client conflict,
        // never a 5xx, consistent with the resource subsystem's
        // `Ambiguous → Conflict` precedent for caller-resolution faults.
        assert_eq!(Classify::category(&err), ErrorCategory::Conflict);
        assert_ne!(
            Classify::category(&err),
            ErrorCategory::Internal,
            "UnknownKind must not surface as a server error"
        );
        assert!(
            ErrorCategory::Conflict.is_client_error(),
            "Conflict must be a client error"
        );
        assert!(
            !ErrorCategory::Conflict.is_server_error(),
            "Conflict must not be a server error"
        );
        assert!(
            !Classify::is_retryable(&err),
            "UnknownKind is a permanent caller error"
        );
        assert!(
            !ErrorCategory::Conflict.is_default_retryable(),
            "Conflict must not be default-retryable"
        );
        assert!(
            Classify::retry_hint(&err).is_none(),
            "UnknownKind carries no retry hint"
        );
        assert_eq!(
            Classify::code(&err).as_str(),
            "ENGINE:RESOURCE_UNKNOWN_KIND"
        );
    }

    #[test]
    fn register_error_delegates_classification_to_inner() {
        // A permanent inner resource error (e.g. schema/deserialize
        // failure) must keep its own category through the wrapper, not
        // be flattened to the UnknownKind conflict.
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
        let registry = ResourceRegistrarRegistry::new();
        assert!(registry.is_empty());

        let err = registry
            .register("anything", &manager, request(&expr_engine))
            .await
            .expect_err("an empty allowlist is fail-closed");
        assert!(matches!(err, RegistrarError::UnknownKind(k) if k == "anything"));
    }
}
