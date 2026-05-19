//! Closed-allowlist `kind → typed registrar` bridge.
//!
//! [`nebula_resource::Manager::register_resolved`] is a *typed* entry
//! point (`register_resolved::<R>`): it monomorphizes on the concrete
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
//! wrong type nor a panic ( — misconfiguration
//! caught at activation; typed-error discipline;//! isolation; slot model).
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
//! [`Manager::register_resolved::<R>`] requires, per its signature:
//!
//! ```text
//! register_resolved(
//!     config_json:   serde_json::Value,
//!     expr_engine:   &nebula_expression::ExpressionEngine,
//!     slot_bindings: HashMap<String, nebula_core::CredentialKey>,
//!     resource:      R,
//!     scope:         ScopeLevel,
//!     topology:      TopologyRuntime<R>,
//!     acquire:       ErasedAcquireFn,
//!     resilience:    Option<AcquireResilience>,
//!     recovery_gate: Option<Arc<RecoveryGate>>,
//! ) -> Result<SlotIdentity, nebula_resource::Error>
//! where R: Resource + DeclaresDependencies, R::Config: DeserializeOwned
//! ```
//!
//! It derives and **returns** the collision-free structural
//! [`SlotIdentity`] the registry row is
//! filed under, so the engine records the *exact* manager-side value (no
//! independent recompute, no dual-derive divergence).
//!
//! Of those, only `config_json` is carried by the stored resource row.
//! `resource: R` and `topology: TopologyRuntime<R>` are **`R`-typed** and
//! cannot be constructed generically at the erased boundary — the
//! resource crate emits no `FromConfig`/constructor and no topology
//! factory from the `#[derive(Resource)]` macro. They must therefore be
//! closed over per concrete `R` when the registrar is built (that is what
//! [`TypedResourceRegistrar`] does: it holds a `resource`-producing
//! factory, a `TopologyRuntime<R>`-producing factory, and an erased
//! `acquire`-hook factory). The acquire hook is **identity-independent**
//! (the single-walk acquire resolution pins the row by the caller's
//! runtime slot identity), so it is no longer parameterised by the
//! registration-time digest.
//!
//! `expr_engine`, `scope`, `slot_bindings`, `resilience`, and
//! `recovery_gate` are type-agnostic and supplied by the *caller* of
//! [`ErasedResourceRegistrar::register`] (the engine registration loop)
//! through [`RegisterRequest`]. This keeps the trait surface honest:
//! the registrar contributes only what is intrinsically per-`R`; the
//! caller threads everything that depends on engine/activation context.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use nebula_resource::{
    AcquireResilience, Manager, ScopeLevel, SlotIdentity, TopologyRuntime, recovery::RecoveryGate,
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
/// `register_resolved` and classifies by delegation, so a deserialize/
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

    /// The typed `Manager::register_resolved` call failed.
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
            // flattening it. (Today every register_resolved failure is
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

/// Metadata returned after a successful live registration through the
/// closed allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRegistrationOutcome {
    /// Catalog key the row was registered under.
    pub resource_key: nebula_core::ResourceKey,
    /// The **collision-free structural** slot identity
    /// ([`SlotIdentity`]) the manager derived
    /// from `slot_bindings` while registering this row.
    ///
    /// This is the value `Manager::register_resolved` *returned* — not an
    /// independent recompute — so it is the *exact* structural key the
    /// manager registry row is filed under (single construction site, no
    /// dual-derive divergence risk). The engine records it for the acquire
    /// path and the rotation fan-out reverse index so both address the same
    /// row.
    pub slot_identity: SlotIdentity,
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
    /// Slot-name → resolved-credential-key bindings (per ). The
    /// engine resolves credentials and is expected to have folded them
    /// into the resource value the registrar produces; this map is
    /// asserted against the resource's declared slots inside the typed
    /// call.
    pub slot_bindings: HashMap<String, nebula_core::CredentialKey>,
    /// Slot-name → resolved `CredentialId` for the rotation fan-out
    /// reverse index.
    ///
    /// `slot_bindings` carries the `CredentialKey` (the credential
    /// *name*, what `Manager::register_resolved` folds into the
    /// structural `slot_identity`); the rotation fan-out index and the
    /// rotation/lease-revoke events are keyed by `CredentialId` (the
    /// resolved credential *record* — `CredentialEvent` /
    /// `LeaseEvent`). Both are needed to bind a row: the `CredentialId`
    /// is the index key, the `CredentialKey` recomputes the matching
    /// `slot_identity`. **Only the caller that resolved the credential
    /// knows both**, so it threads this map per slot it bound; an entry
    /// is added to the reverse index for every `(slot_name,
    /// CredentialId)` whose `slot_name` also appears in `slot_bindings`.
    ///
    /// Empty (the default via [`HashMap::new`]) when the caller resolved
    /// no credential into a slot, or when no fan-out index is threaded
    /// to [`ResourceRegistrarRegistry::register`] — registration then
    /// proceeds exactly as before with no reverse-index row (a later
    /// rotation for an unbound credential is a correct no-op fan-out).
    /// Never fabricated: a slot whose `CredentialId` the caller does not
    /// have is simply absent here rather than bound under a guessed id.
    pub credential_ids: HashMap<String, nebula_credential::CredentialId>,
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
/// [`Manager::register_resolved::<R>`](nebula_resource::Manager::register_resolved).
/// The returned future is boxed so the trait stays object-safe without
/// `#[async_trait]` (matching `EngineResourceAccessor`'s erasure shape).
pub trait ErasedResourceRegistrar: Send + Sync {
    /// Registers this kind's resource against `manager` using the
    /// caller-threaded [`RegisterRequest`] plus the per-`R` resource and
    /// topology this registrar owns.
    ///
    /// On success returns the **collision-free structural**
    /// [`SlotIdentity`] the manager derived
    /// for this row (the value `Manager::register_resolved` returned), so
    /// the registry records the exact manager-side key with no independent
    /// recompute.
    ///
    /// # Errors
    ///
    /// Returns the inner [`nebula_resource::Error`] verbatim if the typed
    /// `register_resolved` fails (deserialize / schema / validation /
    /// slot-binding mismatch). The registry wraps it in
    /// [`RegistrarError::Register`]; the closed-allowlist `UnknownKind`
    /// check happens in [`ResourceRegistrarRegistry::register`] before
    /// this is ever called.
    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, nebula_resource::Error>>;

    /// The static, type-level [`ResourceKey`](nebula_core::ResourceKey)
    /// (`R::key()`) of the concrete resource this registrar registers.
    ///
    /// The erased boundary hides `R`, but the rotation reverse index
    /// (`ResourceFanoutIndex`, the `rotation`-feature-gated reverse
    /// index in `crate::credential::rotation`)
    /// keys a bound row by `(ResourceKey, ScopeLevel, slot_identity)`.
    /// Without `R` the registry cannot name the row it just registered,
    /// so the erased registrar exposes the key explicitly (the same
    /// `R::key()` `register_resolved` registers the row under). Cheap,
    /// pure, and type-stable — it is a `const`-ish accessor, not I/O.
    fn resource_key(&self) -> nebula_core::ResourceKey;

    /// Validate `config_json` against this kind's `R::Config` schema
    /// **without registering anything**.
    ///
    /// Runs the schema pass + closed-set guard + `R::Config` deserialize
    /// (the validation core shared with the live `register` path via
    /// [`Manager::validate_config_value`]), but performs **no** `Manager`
    /// mutation, constructs **no** `resource: R` / `TopologyRuntime<R>`,
    /// and does **no** `{{ … }}` template resolution. This is the seam a
    /// config-CRUD writer uses to reject a bad resource config *before*
    /// persistence — config validation is strictly separate from
    /// engine-activation live registration (.1).
    ///
    /// Synchronous: validation is pure (schema + serde, no I/O), so unlike
    /// [`register`](Self::register) it needs no boxed future.
    ///
    /// # Errors
    ///
    /// Returns the inner [`nebula_resource::Error`] verbatim if the config
    /// is not a field tree, fails the `R::Config` schema, carries an
    /// undeclared (e.g. secret-shaped) field, or fails to deserialize. The
    /// registry wraps it in [`RegistrarError::Register`]; the
    /// closed-allowlist `UnknownKind` check happens in
    /// [`ResourceRegistrarRegistry::validate`] before this is ever called.
    fn validate(&self, config_json: serde_json::Value) -> Result<(), nebula_resource::Error>;
}

/// Per-`R` [`ErasedResourceRegistrar`] that closes over the two pieces
/// the erased boundary cannot synthesize generically: a factory that
/// produces the `resource: R` value (with its credential slots already
/// resolved by the engine) and a factory that produces the
/// `TopologyRuntime<R>` declared for this kind.
///
/// `register_resolved` consumes `resource: R`, `TopologyRuntime<R>`, and
/// the erased `acquire` hook by value, and a registrar may be invoked
/// more than once (re-activation, multiple scopes), so all three are
/// *factories* rather than stored values. The acquire factory takes no
/// slot-identity argument: the single-walk acquire resolution pins the
/// row by the caller's runtime slot identity, so the hook is
/// identity-independent (the legacy `Fn(u64)` digest threading is gone).
pub struct TypedResourceRegistrar<R, FRes, FTopo, FAcq>
where
    R: Resource + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> TopologyRuntime<R> + Send + Sync,
    FAcq: Fn() -> nebula_resource::ErasedAcquireFn + Send + Sync,
{
    resource_factory: FRes,
    topology_factory: FTopo,
    acquire_factory: FAcq,
}

impl<R, FRes, FTopo, FAcq> TypedResourceRegistrar<R, FRes, FTopo, FAcq>
where
    R: Resource + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> TopologyRuntime<R> + Send + Sync,
    FAcq: Fn() -> nebula_resource::ErasedAcquireFn + Send + Sync,
{
    /// Builds a typed registrar for resource type `R`.
    ///
    /// `resource_factory` yields the `R` value (with credential slots
    /// already resolved by the engine per registration scope).
    /// `topology_factory` yields the `TopologyRuntime<R>` declared for
    /// this kind. `acquire_factory` builds the erased acquire hook (for
    /// example
    /// [`Manager::erased_acquire_resident_for`](nebula_resource::manager::Manager::erased_acquire_resident_for))
    /// — it takes no slot-identity argument because the acquire hook is
    /// identity-independent. All three are invoked once per registration
    /// call.
    pub fn new(resource_factory: FRes, topology_factory: FTopo, acquire_factory: FAcq) -> Self {
        Self {
            resource_factory,
            topology_factory,
            acquire_factory,
        }
    }
}

impl<R, FRes, FTopo, FAcq> ErasedResourceRegistrar for TypedResourceRegistrar<R, FRes, FTopo, FAcq>
where
    R: Resource + nebula_core::DeclaresDependencies,
    R::Config: serde::de::DeserializeOwned,
    FRes: Fn() -> R + Send + Sync,
    FTopo: Fn() -> TopologyRuntime<R> + Send + Sync,
    FAcq: Fn() -> nebula_resource::ErasedAcquireFn + Send + Sync,
{
    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, nebula_resource::Error>> {
        let resource = (self.resource_factory)();
        let topology = (self.topology_factory)();
        let acquire = (self.acquire_factory)();
        Box::pin(async move {
            manager
                .register_resolved::<R>(
                    request.config_json,
                    request.expr_engine,
                    request.slot_bindings,
                    resource,
                    request.scope,
                    topology,
                    acquire,
                    request.resilience,
                    request.recovery_gate,
                )
                .await
        })
    }

    fn resource_key(&self) -> nebula_core::ResourceKey {
        <R as Resource>::key()
    }

    fn validate(&self, config_json: serde_json::Value) -> Result<(), nebula_resource::Error> {
        // No `resource_factory` / `topology_factory` are invoked: validation
        // is purely a function of the config JSON and the monomorphized
        // `R::Config` schema — exactly the live path's pre-register checks,
        // minus template resolution and the typed-runtime construction.
        // `validate_config_value` returns the parsed `R::Config` for the
        // live register path; the config-CRUD seam only needs the
        // pass/fail outcome, so the typed value is discarded here.
        Manager::validate_config_value::<R>(config_json).map(|_| ())
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
///.
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
    /// Equivalent to `register_and_bind` (the `rotation`-feature-gated
    /// variant) with no rotation fan-out index — registration only, no
    /// reverse-index row recorded.
    ///
    /// # Errors
    ///
    /// - [`RegistrarError::UnknownKind`] if `kind` is not in the
    ///   allowlist. This is a caller/wiring fault caught at activation — non-retryable, classified as a
    ///   client conflict. The lookup happens **before** any typed call,
    ///   so an unknown kind can never touch a resource type.
    /// - [`RegistrarError::Register`] if the typed
    ///   `Manager::register_resolved` fails (deserialize / schema /
    ///   validation / slot-binding mismatch); classification delegates to
    ///   the inner error.
    pub async fn register(
        &self,
        kind: &str,
        manager: &Manager,
        request: RegisterRequest<'_>,
    ) -> Result<ResourceRegistrationOutcome, RegistrarError> {
        let registrar = self
            .registrars
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        let resource_key = registrar.resource_key();
        // The structural identity comes from `register_resolved`'s return
        // value — the *exact* key the manager filed the registry row under.
        // There is deliberately **no** independent recompute here: a second
        // construction site is the divergence risk the structural-identity
        // change exists to eliminate (a digest mismatch between two
        // construction sites would silently re-alias tenants). Single
        // source of truth, propagated.
        let slot_identity = registrar
            .register(manager, request)
            .await
            .map_err(|source| RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            })?;
        Ok(ResourceRegistrationOutcome {
            resource_key,
            slot_identity,
        })
    }

    /// Resolves `kind` through the closed allowlist, dispatches into its
    /// typed registrar, and — on success — records the resolved row in
    /// the rotation fan-out reverse index.
    ///
    /// This is the §M11.5 `bind` seam: the registered row resolved one
    /// or more `#[credential]` slots, so a later rotation / lease-revoke
    /// of any of those credentials must fan out to *this exact resolved
    /// row*. The structural row identity
    /// ([`SlotIdentity`]) is **not**
    /// recomputed here — it is the value `Manager::register_resolved`
    /// returned (`outcome.slot_identity`), the exact key the manager filed
    /// the registry row under. A second construction site is precisely the
    /// dual-derive divergence risk the collision-free structural identity
    /// exists to eliminate, so the single manager-side value is propagated.
    /// For every `(slot_name, CredentialId)` in `request.credential_ids`
    /// whose `slot_name` also appears in `slot_bindings`, one
    /// [`ResourceFanoutIndex::bind`](crate::credential::rotation::ResourceFanoutIndex::bind)
    /// is recorded. `bind` is idempotent, so a re-registration of the
    /// same resolved row does not duplicate the entry.
    ///
    /// Binding happens **only after `register` returns `Ok`**: a
    /// registration that failed schema / deserialize / slot validation
    /// created no registry row, so no reverse-index row may exist for it
    /// either (no orphan binds, no bogus future `failed` fan-out rows).
    /// `fanout_index = None` (or an empty `credential_ids`) skips the
    /// bind entirely — registration is unchanged.
    ///
    /// # Ordering contract — caller must quiesce fan-out during activation
    ///
    /// `register(...).await` makes the `Manager` row **discoverable**
    /// before this method records the reverse-index bind. Those two
    /// steps are deliberately **not** atomic (atomicity would require a
    /// transactional `Manager::register_resolved` "register-then-
    /// publish" surface — a heavy Manager API change with no production
    /// consumer yet). There is therefore a window in which the Manager
    /// row exists but the reverse-index row does not: a credential
    /// rotation / lease-revoke for this row that the fan-out driver
    /// processes *inside that window* would fan to zero rows (a silent
    /// miss for this row) even though the row is live.
    ///
    /// **The caller MUST ensure the rotation fan-out is quiescent for
    /// the credentials being bound while it activates a row through this
    /// seam** (e.g. activate before the driver is spawned, or serialise
    /// activation against rotation for the affected credentials). This
    /// is a documented contract, not an enforced invariant, because the
    /// seam has **no production caller today**: *bind-population* (the
    /// production credential→slot resolution that would call this) is the
    /// deferred resource-activation path ( — *bind-
    /// population producer*). The driver cannot observe a row that
    /// nothing activated, so the window is not reachable in production
    /// until that deferred producer lands; when it does, it must honour
    /// this contract (or the seam must first gain a transactional
    /// register+bind surface).
    ///
    /// # Errors
    ///
    /// Same as [`register`](Self::register): the bind step runs only on
    /// the `Ok` path and cannot itself fail (it is an in-memory index
    /// insert), so it does not add an error variant.
    ///
    /// Feature-gated with the reverse index itself (`rotation`): the
    /// `ResourceFanoutIndex` type only exists under that feature, so the
    /// bind seam follows it. A non-`rotation` build registers via
    /// [`register`](Self::register) (no reverse-index row — there is no
    /// fan-out to feed).
    #[cfg(feature = "rotation")]
    pub async fn register_and_bind(
        &self,
        kind: &str,
        manager: &Manager,
        request: RegisterRequest<'_>,
        fanout_index: Option<&crate::credential::rotation::ResourceFanoutIndex>,
    ) -> Result<ResourceRegistrationOutcome, RegistrarError> {
        // Snapshot the bind inputs before `request` is moved into the
        // typed `register` call. Cheap clones (a small slot map + scope);
        // the resource key comes from the erased registrar and the
        // structural slot identity from `register_resolved`'s return, so
        // the reverse index addresses the same `(key, scope, slot_identity)`
        // row the manager registers under.
        let bind_plan = fanout_index.map(|idx| {
            (
                idx,
                request.scope.clone(),
                request.slot_bindings.clone(),
                request.credential_ids.clone(),
            )
        });

        let outcome = self.register(kind, manager, request).await?;

        // Registration succeeded — the registry row exists. Record the
        // reverse-index binding so a later rotation / lease-revoke fans
        // to this exact resolved row.
        if let Some((idx, scope, slot_bindings, credential_ids)) = bind_plan {
            for (slot_name, cred_id) in &credential_ids {
                // Only bind slots the resource actually declared (those
                // present in `slot_bindings`): a stray `credential_ids`
                // entry for an unbound slot must not create a phantom
                // reverse-index row.
                if slot_bindings.contains_key(slot_name) {
                    idx.bind(
                        *cred_id,
                        outcome.resource_key.clone(),
                        scope.clone(),
                        slot_name.clone(),
                        outcome.slot_identity.clone(),
                    );
                }
            }
        }
        Ok(outcome)
    }

    /// Resolves `kind` through the closed allowlist and validates
    /// `config_json` against that kind's `R::Config` schema **without
    /// registering anything**.
    ///
    /// This is the config-CRUD seam: a writer persisting a resource
    /// definition validates the config here *before* the row is stored.
    /// Validation runs the same schema pass + closed-set guard + deserialize
    /// the live [`register`](Self::register) path runs (shared via
    /// [`Manager::validate_config_value`]), but mutates **no** `Manager`,
    /// builds **no** runtime, and resolves **no** `{{ … }}` templates —
    /// live registration stays an engine-activation concern
    /// (.1), distinct from config validation.
    ///
    /// The closed-allowlist lookup is identical to `register`'s: an unknown
    /// `kind` is rejected *before* any typed call, so it can never touch a
    /// resource type (closed dependency graph —    /// the type-confusion abuse).
    ///
    /// # Errors
    ///
    /// - [`RegistrarError::UnknownKind`] if `kind` is not in the allowlist
    ///   — a caller/wiring fault, non-retryable, classified as a client
    ///   conflict (not a 5xx).
    /// - [`RegistrarError::Register`] if the config fails the `R::Config`
    ///   schema, carries an undeclared (e.g. secret-shaped) field, or fails
    ///   to deserialize; classification delegates to the inner error.
    pub fn validate(
        &self,
        kind: &str,
        config_json: serde_json::Value,
    ) -> Result<(), RegistrarError> {
        let registrar = self
            .registrars
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        registrar
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
    use nebula_resource::{
        Manager, ScopeLevel,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
        runtime::{TopologyRuntime, resident::ResidentRuntime},
        topology::resident::{self, Resident},
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
            // threaded through `register_resolved` was actually parsed
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

    impl Resident for TestRes {
        fn is_alive_sync(&self, runtime: &Arc<AtomicU64>) -> bool {
            runtime.load(Ordering::Relaxed) < u64::MAX
        }
    }

    fn test_registrar(create_counter: Arc<AtomicU64>) -> Arc<dyn ErasedResourceRegistrar> {
        Arc::new(TypedResourceRegistrar::<TestRes, _, _, _>::new(
            move || TestRes::new(create_counter.clone()),
            || {
                TopologyRuntime::Resident(ResidentRuntime::<TestRes>::new(
                    resident::config::Config::default(),
                ))
            },
            || Manager::erased_acquire_resident_for::<TestRes>(),
        ))
    }

    fn request(expr_engine: &ExpressionEngine) -> RegisterRequest<'_> {
        RegisterRequest {
            config_json: serde_json::json!({ "name": "from-registry" }),
            expr_engine,
            slot_bindings: HashMap::new(),
            credential_ids: HashMap::new(),
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
