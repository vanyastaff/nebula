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
//! [`KindActivator`] is the sole production per-`R` implementor. The trait is
//! sealed, so downstream plugins receive erased factory authority from this
//! crate instead of self-attesting metadata, type identity, or registration
//! outcomes. It closes over two factories:
//! one that produces the `R` value and one that produces `R::Topology`. Both
//! are factories (not stored values) so one `KindActivator` can be invoked
//! multiple times (re-activation, multiple scopes). The `#[derive(Resource)]`
//! macro emits a `<Name>Factory` newtype that wraps a `KindActivator` with
//! the topology kind fixed by a `#[topology(Pooled|Resident, ...)]` attribute
//! on the derive; `into_contribution()` yields its sealed erased capability.
//!
//! # Four frozen laws (CI-enforced)
//!
//! 1. **Schema-single-source** — `metadata().base.schema()` derives from the
//!    same `<R::Config as HasSchema>::schema()` that `validate` and `register`
//!    use.
//! 2. **Removal-funnel** — raw `Manager` mutation is the caller's concern;
//!    teardown must route through a `PluginHandle` (engine-side).
//! 3. **Key-coherence** — `factory.key() == <R as Provider>::key()`.
//! 4. **Factory provenance** — downstream code cannot implement
//!    `ResourceFactory`; metadata, `TypeId`, and registration identity are
//!    projections of one crate-issued typed activator.
//!
//! # Latent-by-design on landing
//!
//! The `register` arm ships with zero production callers (only tests call
//! typed registration today; `api/state.rs` holds the registry for config
//! validation only, never live-registration). This is correct and intentional
//! — the bind-population producer that gives `register` a production caller
//! is the named M12.4 follow-up.

use std::{
    any::TypeId,
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};

use crate::resource::{ResourceMetadata, ResourceMetadataDraft};
use crate::topology::Topology;
use crate::{Manager, ScopeLevel, SlotIdentity, recovery::RecoveryGate, resource::Provider};
use nebula_core::Dependencies;

mod private {
    pub trait Sealed {}
}

/// Boxed, `Send` future returned across the erased factory boundary.
///
/// Mirrors the `BoxFut` alias used by `EngineResourceAccessor` so both erased
/// `Manager` bridges share one async-erasure shape.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One resolved credential slot binding the caller threads into a registration.
///
/// The slot name, its resolved [`CredentialKey`](nebula_core::CredentialKey),
/// and (when the credential participates in rotation) its resolved
/// [`CredentialId`](nebula_credential::CredentialId) travel together as one
/// unit, rather than as two parallel `HashMap<String, _>` keyed by slot name.
///
/// Co-locating them makes a key↔id divergence for the same slot structurally
/// unrepresentable: the rotation fan-out reverse-index row and the structural
/// [`SlotIdentity`] are derived from the **same** binding. This closes the
/// confused-deputy gap where a revoke routed by `CredentialId` (from one map)
/// could taint a row whose `SlotIdentity` was built from a different
/// credential (from a divergent second map), and removes the silent
/// `continue` that dropped a `CredentialId` whose slot was absent from the
/// other map.
#[derive(Debug, Clone)]
pub struct SlotBinding {
    /// The declared `#[credential(key = ...)]` slot name on the resource.
    pub slot_name: String,
    /// The resolved credential key bound to this slot — contributes to the
    /// structural [`SlotIdentity`].
    pub credential_key: nebula_core::CredentialKey,
    /// The resolved `CredentialId` for the rotation fan-out reverse index, or
    /// `None` when this credential does not participate in rotation (no
    /// reverse-index row is staged for it).
    pub credential_id: Option<nebula_credential::CredentialId>,
}

/// One resolved projected guard to install before resource publication.
///
/// This travels separately from [`SlotBinding`]: structural identity and
/// fan-out routing remain cloneable metadata, while the guard itself has one
/// owner and cannot be cloned.
pub struct CredentialSlotInstall {
    /// Declared resource slot receiving the guard.
    pub slot_name: String,
    /// Opaque projected guard carrying its authoritative material epoch.
    pub guard: nebula_credential::ErasedCredentialGuard,
}

impl std::fmt::Debug for CredentialSlotInstall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialSlotInstall")
            .field("slot_name", &self.slot_name)
            .field("guard", &self.guard)
            .finish()
    }
}

/// Explicit ingress for resource configuration registration.
///
/// Literal JSON and authored expressions are different capabilities. Use
/// [`Self::data`] for persisted or transport JSON; strings and objects in that
/// input are always data, even when they resemble template syntax or an
/// expression envelope. Use [`Self::authored`] only when an authoring layer has
/// deliberately constructed an [`nebula_schema::AuthoredValue`] with expression
/// nodes.
pub struct ResourceConfigInput {
    source: ResourceConfigSource,
}

pub(crate) enum ResourceConfigSource {
    Data(serde_json::Value),
    Authored(nebula_schema::AuthoredValue),
}

impl ResourceConfigInput {
    /// Admit literal JSON data without interpreting any string or object as code.
    #[must_use]
    pub fn data(value: serde_json::Value) -> Self {
        Self {
            source: ResourceConfigSource::Data(value),
        }
    }

    /// Admit explicitly authored values, including deliberate expression nodes.
    #[must_use]
    pub fn authored(value: nebula_schema::AuthoredValue) -> Self {
        Self {
            source: ResourceConfigSource::Authored(value),
        }
    }

    pub(crate) const fn kind(&self) -> &'static str {
        match self.source {
            ResourceConfigSource::Data(_) => "data",
            ResourceConfigSource::Authored(_) => "authored",
        }
    }

    pub(crate) fn into_source(self) -> ResourceConfigSource {
        self.source
    }
}

impl std::fmt::Debug for ResourceConfigInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ResourceConfigInput")
            .field(&self.kind())
            .finish()
    }
}

/// Type-agnostic inputs the *caller* threads into a typed registration.
///
/// Everything here is independent of the concrete resource type `R`; the
/// per-`R` pieces (`resource: R`, `R::Topology`) are closed over by the
/// factory (see [`KindActivator`]). Borrowed for the duration of the call so
/// the factory can be invoked without cloning the expression engine.
pub struct RegisterRequest<'a> {
    /// Resource-specific config with an explicit data or authored ingress.
    pub config: ResourceConfigInput,
    /// Engine-held expression engine used only for explicitly authored
    /// expressions in [`Self::config`].
    pub expr_engine: &'a nebula_expression::ExpressionEngine,
    /// Resolved credential slot bindings — each a [`SlotBinding`] carrying the
    /// slot name, credential key, and optional rotation `CredentialId`
    /// together. Asserted against the resource's declared slots inside the
    /// typed call; both the reverse-index row and the structural identity are
    /// derived from these same bindings, so the two cannot diverge.
    pub slot_bindings: Vec<SlotBinding>,
    /// Projected guards installed into the freshly constructed resource
    /// before the manager can publish its registry row. Each guard carries
    /// its own authoritative material epoch; callers cannot pair them
    /// independently.
    pub slot_installs: Vec<CredentialSlotInstall>,
    /// Registration scope.
    pub scope: ScopeLevel,
    /// Optional recovery gate shared across a recovery group.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
}

impl std::fmt::Debug for RegisterRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Config and binding payloads are opaque caller data. Neither values
        // nor field/slot names are guaranteed secret-free before validation.
        // Keep diagnostics useful without formatting either payload.
        f.debug_struct("RegisterRequest")
            .field("config", &self.config)
            .field("expr_engine", &"<ExpressionEngine>")
            .field("slot_binding_count", &self.slot_bindings.len())
            .field("slot_install_count", &self.slot_installs.len())
            .field("scope", &self.scope)
            .field("recovery_gate", &self.recovery_gate.is_some())
            .finish()
    }
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

    /// A sealed factory returned a row identity different from the canonical
    /// identity derived from the registration request.
    ///
    /// This is an internal invariant failure, not invalid caller input. The
    /// registry removes the published manager row before returning it; a
    /// compensation failure is reported separately rather than hidden.
    #[error(
        "resource kind `{kind}` returned a non-canonical slot identity: \
         expected {expected:?}, got {actual:?}"
    )]
    IdentityMismatch {
        /// The allowlist kind being registered.
        kind: String,
        /// Canonical identity derived from the request's slot bindings.
        expected: SlotIdentity,
        /// Identity returned by the erased factory.
        actual: SlotIdentity,
    },

    /// Compensation after an identity mismatch could not remove the row.
    #[error(
        "resource kind `{kind}` returned a non-canonical slot identity and \
         rollback failed: {source}"
    )]
    IdentityMismatchRollback {
        /// The allowlist kind being registered.
        kind: String,
        /// Canonical identity derived from the request's slot bindings.
        expected: SlotIdentity,
        /// Identity returned by the erased factory.
        actual: SlotIdentity,
        /// Failure from removing the just-published manager row.
        #[source]
        source: Box<crate::Error>,
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
            Self::IdentityMismatch { .. } | Self::IdentityMismatchRollback { .. } => {
                nebula_error::ErrorCategory::Internal
            },
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::UnknownKind(_) => nebula_error::ErrorCode::new("RESOURCE:FACTORY_UNKNOWN_KIND"),
            Self::Register { source, .. } => nebula_error::Classify::code(source),
            Self::IdentityMismatch { .. } => {
                nebula_error::ErrorCode::new("RESOURCE:FACTORY_IDENTITY_MISMATCH")
            },
            Self::IdentityMismatchRollback { .. } => {
                nebula_error::ErrorCode::new("RESOURCE:FACTORY_IDENTITY_ROLLBACK_FAILED")
            },
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::UnknownKind(_) => false,
            Self::Register { source, .. } => nebula_error::Classify::is_retryable(source),
            Self::IdentityMismatch { .. } | Self::IdentityMismatchRollback { .. } => false,
        }
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::UnknownKind(_) => None,
            Self::Register { source, .. } => nebula_error::Classify::retry_hint(source),
            Self::IdentityMismatch { .. } | Self::IdentityMismatchRollback { .. } => None,
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

/// Internal reverse-index ownership carried through the sealed registration
/// boundary. Public only because it appears on the sealed factory trait.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct RegistrationBindings<'a> {
    #[cfg(feature = "rotation")]
    index: Option<&'a crate::ResourceFanoutIndex>,
    #[cfg(feature = "rotation")]
    staged: &'a [(nebula_credential::CredentialId, crate::Bind)],
    marker: std::marker::PhantomData<&'a ()>,
}

impl RegistrationBindings<'_> {
    pub(crate) const fn empty() -> Self {
        Self {
            #[cfg(feature = "rotation")]
            index: None,
            #[cfg(feature = "rotation")]
            staged: &[],
            marker: std::marker::PhantomData,
        }
    }

    #[cfg(feature = "rotation")]
    pub(crate) fn staged<'a>(
        index: Option<&'a crate::ResourceFanoutIndex>,
        staged: &'a [(nebula_credential::CredentialId, crate::Bind)],
    ) -> RegistrationBindings<'a> {
        RegistrationBindings {
            index,
            staged,
            marker: std::marker::PhantomData,
        }
    }
}

#[cfg(feature = "rotation")]
impl<'a> RegistrationBindings<'a> {
    pub(crate) fn rotation_index(self) -> Option<&'a crate::ResourceFanoutIndex> {
        self.index
    }

    pub(crate) fn staged_entries(self) -> &'a [(nebula_credential::CredentialId, crate::Bind)] {
        self.staged
    }
}

/// Object-safe, type-erased **B+ merged contribution contract** for one
/// resource type.
///
/// Carries the introspection arm (`key`, `metadata`, `validate`) and the
/// construction arm (`register`). Object-safe: stored as
/// `Arc<dyn ResourceFactory>` in `Plugin::resources()`.
///
/// The private supertrait seals implementations to this crate. Plugins must
/// use a typed [`KindActivator`] or a derive-emitted factory wrapper, so the
/// key, metadata, type identity, validation, and registration arms all come
/// from the same `R`.
pub trait ResourceFactory: private::Sealed + Send + Sync + 'static {
    /// The static `ResourceKey` identifying the concrete resource type.
    ///
    /// Pure and side-effect-free — a `const`-ish accessor, not I/O.
    fn key(&self) -> nebula_core::ResourceKey;

    /// Declared resource and credential dependencies for this resource type.
    fn dependencies(&self) -> &Dependencies;

    /// Local process type identity of the concrete resource type.
    ///
    /// This value is used only for activation-time coherence checks. It is
    /// never a durable or transport identity.
    fn resource_type_id(&self) -> TypeId;

    /// Resource metadata for catalog display, schema introspection, and
    /// install-from-repo pre-install enumeration.
    ///
    /// The first call admits the schema-free author draft by deriving the
    /// canonical schema from `R::Config`; later calls return that immutable,
    /// cached definition. `validate` and `register` consume its schema rather
    /// than deriving another copy (schema-single-source law).
    ///
    /// # Errors
    /// Returns a typed catalog error if the definition or configuration schema is invalid.
    fn metadata(&self) -> Result<&ResourceMetadata, crate::MetadataBuildError>;

    /// Validate `config_json` against this resource's `R::Config` schema
    /// **without registering anything**.
    ///
    /// Runs the same schema pass + closed-set guard + `R::Config` deserialize
    /// as the live `register` path (shared through the same private manager helper),
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
    /// topology this factory owns. `expected_slot_identity` is the canonical
    /// identity derived by the registry from that exact request; the typed
    /// manager must verify it before making the row discoverable.
    ///
    /// On success returns the **collision-free structural** [`SlotIdentity`]
    /// the manager derived for this row — the exact value
    /// `Manager::register_resolved` returned — so the caller can record the
    /// row key without an independent recompute. The typed manager checks
    /// this identity against the request-derived expectation before publishing
    /// the row, and the erased registry checks the returned postcondition.
    ///
    /// The returned future is boxed so the trait stays object-safe without
    /// `#[async_trait]`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error`] verbatim if the typed `register_resolved`
    /// fails (deserialize / schema / validation / slot-binding or canonical
    /// identity mismatch).
    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
        expected_slot_identity: &'a SlotIdentity,
    ) -> BoxFut<'a, Result<SlotIdentity, crate::Error>>;

    #[doc(hidden)]
    fn register_with_bindings<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
        expected_slot_identity: &'a SlotIdentity,
        _registration_bindings: RegistrationBindings<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, crate::Error>> {
        self.register(manager, request, expected_slot_identity)
    }
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
    dependencies: Dependencies,
    metadata_draft: Option<ResourceMetadataDraft>,
    metadata: OnceLock<Result<ResourceMetadata, crate::MetadataBuildError>>,
    // Zero-sized marker — `R` is not stored but bounds the `impl`.
    _marker: std::marker::PhantomData<fn() -> R>,
}

impl<R, FRes, FTopo> std::fmt::Debug for KindActivator<R, FRes, FTopo>
where
    R: Provider + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> R::Topology + Send + Sync,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `resource_factory` / `topology_factory` are closures — neither is
        // `Debug` and neither should be invoked just to print it. Identify
        // the activator by the resource key it constructs instead.
        f.debug_struct("KindActivator")
            .field("key", &<R as Provider>::key())
            .finish_non_exhaustive()
    }
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
            dependencies: R::dependencies(),
            metadata_draft: None,
            metadata: OnceLock::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Builds an activator from explicit schema-free resource author intent.
    ///
    /// The supplied draft cannot carry a schema. Admission still derives the
    /// canonical schema from `R::Config` at this factory boundary.
    pub fn with_metadata(
        metadata_draft: ResourceMetadataDraft,
        resource_factory: FRes,
        topology_factory: FTopo,
    ) -> Self {
        Self {
            resource_factory,
            topology_factory,
            dependencies: R::dependencies(),
            metadata_draft: Some(metadata_draft),
            metadata: OnceLock::new(),
            _marker: std::marker::PhantomData,
        }
    }

    fn admit_metadata(&self) -> Result<ResourceMetadata, crate::MetadataBuildError> {
        let schema = nebula_schema::schema_of::<R::Config>()?;
        let draft = self.metadata_draft.clone().unwrap_or_else(R::metadata);
        let metadata = draft.admit(schema)?;
        let expected = R::key();
        if metadata.base().key() != &expected {
            let actual = metadata.base().key().clone();
            tracing::warn!(
                error_code = "RESOURCE:METADATA_KEY_MISMATCH",
                expected_key = %expected,
                actual_key = %actual,
                "resource metadata admission rejected"
            );
            return Err(crate::MetadataBuildError::KeyMismatch { expected, actual });
        }
        Ok(metadata)
    }
}

impl<R, FRes, FTopo> private::Sealed for KindActivator<R, FRes, FTopo>
where
    R: Provider + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    R::Topology: Topology<R>,
    FRes: Fn() -> R + Send + Sync + 'static,
    FTopo: Fn() -> R::Topology + Send + Sync + 'static,
{
}

impl<R, FRes, FTopo> ResourceFactory for KindActivator<R, FRes, FTopo>
where
    R: Provider + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    R::Topology: Topology<R>,
    FRes: Fn() -> R + Send + Sync + 'static,
    FTopo: Fn() -> R::Topology + Send + Sync + 'static,
{
    fn key(&self) -> nebula_core::ResourceKey {
        <R as Provider>::key()
    }

    fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }

    fn resource_type_id(&self) -> TypeId {
        TypeId::of::<R>()
    }

    #[tracing::instrument(
        level = "debug",
        name = "resource.factory.admit_metadata",
        skip_all,
        fields(resource_key = %R::key()),
        err
    )]
    fn metadata(&self) -> Result<&ResourceMetadata, crate::MetadataBuildError> {
        self.metadata
            .get_or_init(|| self.admit_metadata())
            .as_ref()
            .map_err(Clone::clone)
    }

    fn validate(&self, config_json: serde_json::Value) -> Result<(), crate::Error> {
        // No resource_factory / topology_factory invoked: validation is purely
        // a function of the config JSON and the monomorphized `R::Config`
        // schema — exactly the live path's pre-register checks, minus template
        // resolution and typed-runtime construction.
        let metadata = self.metadata().map_err(|source| {
            crate::Error::permanent("resource factory metadata admission failed")
                .with_source(source)
        })?;
        Manager::validate_config_value_against::<R>(metadata.base().schema(), config_json)
            .map(|_| ())
    }

    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
        expected_slot_identity: &'a SlotIdentity,
    ) -> BoxFut<'a, Result<SlotIdentity, crate::Error>> {
        self.register_with_bindings(
            manager,
            request,
            expected_slot_identity,
            RegistrationBindings::empty(),
        )
    }

    fn register_with_bindings<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
        expected_slot_identity: &'a SlotIdentity,
        registration_bindings: RegistrationBindings<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, crate::Error>> {
        Box::pin(async move {
            let metadata = self.metadata().map_err(|source| {
                crate::Error::permanent("resource factory metadata admission failed")
                    .with_source(source)
            })?;
            let resource = (self.resource_factory)();
            let topology = (self.topology_factory)();
            // The typed register validates declared slots and derives the
            // structural identity from the (slot → credential-key) view; the
            // rotation `CredentialId` lives on the same bindings and is
            // consumed by the reverse index in `register_and_bind`.
            let slot_keys: HashMap<String, nebula_core::CredentialKey> = request
                .slot_bindings
                .iter()
                .map(|binding| (binding.slot_name.clone(), binding.credential_key.clone()))
                .collect();
            for install in request.slot_installs {
                match resource.install_credential_slot(&install.slot_name, install.guard) {
                    Ok(crate::SlotUpdate::Installed) => {},
                    Ok(
                        crate::SlotUpdate::Stale { .. }
                        | crate::SlotUpdate::Revoked
                        | crate::SlotUpdate::AlreadyRevoked,
                    ) => {
                        return Err(crate::Error::permanent(
                            "initial credential slot population was not installed",
                        )
                        .with_resource_key(R::key()));
                    },
                    Err(source) => {
                        return Err(crate::Error::permanent(
                            "initial credential slot population failed",
                        )
                        .with_source(source)
                        .with_resource_key(R::key()));
                    },
                }
            }
            manager
                .register_resolved::<R>(
                    metadata.base().schema(),
                    request.config,
                    request.expr_engine,
                    slot_keys,
                    resource,
                    request.scope,
                    topology,
                    request.recovery_gate,
                    expected_slot_identity,
                    registration_bindings,
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

impl std::fmt::Debug for ResourceActivatorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn ResourceFactory` is not `Debug` (and closes over per-`R`
        // closures via `KindActivator`) — list the registered kinds only.
        let mut kinds: Vec<&str> = self.factories.keys().map(String::as_str).collect();
        kinds.sort_unstable();
        f.debug_struct("ResourceActivatorRegistry")
            .field("kinds", &kinds)
            .finish()
    }
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
    ///
    /// Admission happens before the allowlist is mutated. A rejected resource
    /// definition therefore cannot become discoverable through this registry.
    ///
    /// # Errors
    ///
    /// Returns [`crate::MetadataBuildError`] when the resource's canonical
    /// configuration schema cannot be derived.
    pub fn insert(
        &mut self,
        kind: impl Into<String>,
        factory: Arc<dyn ResourceFactory>,
    ) -> Result<Option<Arc<dyn ResourceFactory>>, crate::MetadataBuildError> {
        factory.metadata()?;
        Ok(self.factories.insert(kind.into(), factory))
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
        let scope = request.scope.clone();
        let expected_slot_identity = slot_identity_from_request(&request);
        let slot_identity = factory
            .register(manager, request, &expected_slot_identity)
            .await
            .map_err(|source| RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            })?;
        ensure_registration_identity(
            kind,
            manager,
            &resource_key,
            &scope,
            &expected_slot_identity,
            &slot_identity,
            None,
        )?;
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
        fanout_index: Option<&Arc<crate::ResourceFanoutIndex>>,
    ) -> Result<ResourceRegistrationOutcome, RegistrarError> {
        let factory = self
            .factories
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        let resource_key = factory.key();
        let staged_slot_identity = slot_identity_from_request(&request);
        let scope = request.scope.clone();

        if let Some(index) = fanout_index {
            manager.attach_rotation_index(index);
        }

        // Stage reverse-index binds BEFORE the typed register makes the
        // Manager row discoverable. Each `CredentialId` rides on the SAME
        // `SlotBinding` whose `credential_key` fed `staged_slot_identity`, so
        // the index row and the structural identity can never be derived from
        // different credentials (confused-deputy close), and a slot without a
        // rotation `CredentialId` is simply skipped — no silent drop of a
        // mismatched parallel-map entry.
        let mut staged: Vec<(nebula_credential::CredentialId, _)> = Vec::new();
        if let Some(idx) = fanout_index {
            for binding in &request.slot_bindings {
                let Some(cred_id) = binding.credential_id else {
                    continue;
                };
                let bind = crate::Bind {
                    resource_key: resource_key.clone(),
                    scope: request.scope.clone(),
                    slot_name: binding.slot_name.clone(),
                    slot_identity: staged_slot_identity.clone(),
                };
                idx.stage_bind(cred_id, bind.clone());
                staged.push((cred_id, bind));
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

        let slot_identity = factory
            .register_with_bindings(
                manager,
                request,
                &staged_slot_identity,
                RegistrationBindings::staged(fanout_index.map(Arc::as_ref), &rollback.1),
            )
            .await
            .map_err(|source| RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            })?;
        ensure_registration_identity(
            kind,
            manager,
            &resource_key,
            &scope,
            &staged_slot_identity,
            &slot_identity,
            fanout_index.map(Arc::as_ref),
        )?;
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

fn slot_identity_from_request(request: &RegisterRequest<'_>) -> SlotIdentity {
    SlotIdentity::from_bindings(
        request
            .slot_bindings
            .iter()
            .map(|binding| (binding.slot_name.as_str(), binding.credential_key.as_str())),
    )
}

fn ensure_registration_identity(
    kind: &str,
    manager: &Manager,
    resource_key: &nebula_core::ResourceKey,
    scope: &ScopeLevel,
    expected: &SlotIdentity,
    actual: &SlotIdentity,
    #[cfg(feature = "rotation")] fanout_index: Option<&crate::ResourceFanoutIndex>,
    #[cfg(not(feature = "rotation"))] _fanout_index: Option<&()>,
) -> Result<(), RegistrarError> {
    if actual == expected {
        return Ok(());
    }

    tracing::error!(
        error_code = "RESOURCE:FACTORY_IDENTITY_MISMATCH",
        resource.kind = kind,
        resource.key = %resource_key,
        ?scope,
        ?expected,
        ?actual,
        "sealed resource factory returned a non-canonical registration identity"
    );

    if let Err(source) = manager.remove_for(resource_key, scope, expected)
        && source.kind() != &crate::ErrorKind::NotFound
    {
        return Err(RegistrarError::IdentityMismatchRollback {
            kind: kind.to_owned(),
            expected: expected.clone(),
            actual: actual.clone(),
            source: Box::new(source),
        });
    }

    #[cfg(feature = "rotation")]
    if let Some(index) = fanout_index {
        index.unbind_resource_identity(resource_key, scope, expected);
    }

    Err(RegistrarError::IdentityMismatch {
        kind: kind.to_owned(),
        expected: expected.clone(),
        actual: actual.clone(),
    })
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
