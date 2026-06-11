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
//! `expr_engine`, `scope`, `slot_bindings`, and `recovery_gate` are
//! type-agnostic and supplied by the *caller* of
//! [`ErasedResourceRegistrar::register`] (the engine registration loop)
//! through [`RegisterRequest`]. This keeps the trait surface honest:
//! the registrar contributes only what is intrinsically per-`R`; the
//! caller threads everything that depends on engine/activation context.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use nebula_resource::{
    Manager, ScopeLevel, SlotIdentity, TopologyRuntime, recovery::RecoveryGate, resource::Resource,
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
    /// `ResourceFanoutIndex::bind`
    /// is recorded. `bind` is idempotent, so a re-registration of the
    /// same resolved row does not duplicate the entry.
    ///
    /// `fanout_index = None` (or an empty `credential_ids`) skips the
    /// bind entirely — registration is unchanged.
    ///
    /// # Ordering — structural guarantees and the residual window
    ///
    /// The reverse-index bind is recorded **before** the typed
    /// `register` call makes the `Manager` row discoverable, and a
    /// failed `register` removes the staged bind via RAII compensation.
    /// This gives two *structural* guarantees (not caller-discipline
    /// contracts):
    ///
    /// * **No silent miss on a live row.** The fan-out driver can never
    ///   observe a discoverable `Manager` row whose reverse-index row is
    ///   absent: the bind exists no later than the row. The previous
    ///   register-then-bind ordering had the inverse window — a
    ///   rotation/lease-revoke processed between `register` returning and
    ///   the bind being recorded fanned to zero rows on a row that was
    ///   already *live and serving*, which is the worse failure (a
    ///   running resource silently keeps a rotated/revoked credential).
    /// * **No orphan reverse-index row.** If `register` returns `Err`
    ///   (schema / deserialize / slot validation), it created no registry
    ///   row; a `scopeguard` removes the staged bind on that path so no
    ///   reverse-index row survives a failed registration (no bogus
    ///   future `failed` fan-out rows). A prior successful registration
    ///   of an identical resolved row keeps its binding — only entries
    ///   *this* call freshly inserted are rolled back.
    /// * **No dual-derive divergence.** The staged bind's
    ///   [`SlotIdentity`] is derived from `request.slot_bindings` via the
    ///   canonical
    ///   [`SlotIdentity::from_bindings`](nebula_resource::SlotIdentity::from_bindings)
    ///   — the *sanctioned wire form* every consumer must reconstruct
    ///   through, which yields a byte-identical key to the one
    ///   `Manager::register_resolved` derives from the same bindings and
    ///   returns. This is the canonical recompute, not a forbidden second
    ///   construction site: `register_resolved` derives the identity from
    ///   the same `slot_bindings` with the same function, so the staged
    ///   key and the returned key are equal by construction.
    ///
    /// Inverting the order (rather than the weaker
    /// compensation-on-failure-only variant) is possible without any
    /// `Manager` API change precisely because `slot_identity` is a pure
    /// function of `slot_bindings` (which is *not* template-resolved) via
    /// the canonical constructor — so the exact row key is knowable
    /// before `register` runs.
    ///
    /// ## Residual pre-publish window (not closed by either ordering)
    ///
    /// The inversion eliminates the *live-row* silent miss but does not
    /// make registration atomic: there is still a symmetric window where
    /// the staged reverse-index row exists but `register` has not yet
    /// published the `Manager` row. A rotation/lease-revoke for a bound
    /// credential that the fan-out driver processes inside that window
    /// dispatches against a not-yet-discoverable `Manager` row, is counted
    /// as a miss, and is **not** replayed when `register` completes — so
    /// the row can come up bound to a pre-rotation (stale/revoked)
    /// credential. This is strictly less severe than the inverse window
    /// it replaced (the row is not serving yet), but it is real. No pure
    /// bind/register ordering closes both; the only full closure is an
    /// atomic register-then-publish `Manager` surface (a heavy API change,
    /// deferred with the bind-population producer) **or** quiescing the
    /// rotation fan-out for the bound credentials across activation.
    /// Therefore: a caller that runs the rotation fan-out driver
    /// concurrently with `register_and_bind` MUST quiesce it for the
    /// credentials being bound until this returns. Reverting to
    /// register-then-bind does not help — it reintroduces the worse
    /// live-row window.
    ///
    /// # Errors
    ///
    /// Same as [`register`](Self::register): the bind step is an
    /// in-memory index insert and cannot itself fail, so it adds no error
    /// variant. On the `register`-`Err` path the staged bind is rolled
    /// back before the error is returned.
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
        fanout_index: Option<&nebula_resource::ResourceFanoutIndex>,
    ) -> Result<ResourceRegistrationOutcome, RegistrarError> {
        // Resolve the registrar up front (same closed-allowlist lookup +
        // `UnknownKind` fault as `register`). The erased registrar yields
        // the row's `ResourceKey`, and the structural `SlotIdentity` is
        // recomputed from `request.slot_bindings` through the canonical
        // `SlotIdentity::from_bindings` — the sanctioned wire form
        // `Manager::register_resolved` itself uses, so the staged key is
        // byte-identical to the one it derives and returns (no
        // dual-derive divergence; `slot_bindings` is not template-
        // resolved, so the key is knowable before `register` runs).
        let registrar = self
            .registrars
            .get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_owned()))?;
        let resource_key = registrar.resource_key();
        let staged_slot_identity = SlotIdentity::from_bindings(
            request
                .slot_bindings
                .iter()
                .map(|(slot, cred)| (slot.as_str(), cred.as_str())),
        );

        // Stage the reverse-index binds BEFORE the typed register makes
        // the `Manager` row discoverable. Each staging takes a reference
        // on the (refcounted) reverse-index entry; the failure rollback
        // releases exactly this call's reference and the entry is removed
        // only when its last referent is gone. There is therefore no
        // "did I insert this?" decision to make — the previous
        // `affected(cid).contains(&bind)` read before `bind` was a
        // check-then-act race (two concurrent stagings of the identical
        // resolved row could both observe "absent", and the failing one's
        // rollback would then delete the surviving one's live row). The
        // refcount makes the rollback correct without that read.
        let mut staged: Vec<(nebula_credential::CredentialId, _)> = Vec::new();
        if let Some(idx) = fanout_index {
            for (slot_name, cred_id) in &request.credential_ids {
                // Only bind slots the resource actually declared (those
                // present in `slot_bindings`): a stray `credential_ids`
                // entry for an unbound slot must not create a phantom
                // reverse-index row.
                if !request.slot_bindings.contains_key(slot_name) {
                    continue;
                }
                let bind = nebula_resource::Bind {
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

        // Arm RAII compensation: if the typed `register` fails (or this
        // scope unwinds), every freshly-staged entry is removed so a
        // failed registration leaves no orphan reverse-index row. Defused
        // on the success path — the binds are kept and already address
        // the exact `(key, scope, slot_identity)` the manager filed.
        let rollback = scopeguard::guard((fanout_index, staged), |(idx, staged)| {
            if let Some(idx) = idx {
                for (cred_id, bind) in &staged {
                    idx.unbind_staged_entry(cred_id, bind);
                }
            }
        });

        let slot_identity = registrar
            .register(manager, request)
            .await
            .map_err(|source| RegistrarError::Register {
                kind: kind.to_owned(),
                source,
            })?;

        // Registration succeeded — keep the pre-staged binds. They were
        // recorded under `staged_slot_identity`, which equals the
        // manager-returned `slot_identity` by the canonical-wire-form
        // contract (both derive from the same `slot_bindings` via
        // `SlotIdentity::from_bindings`).
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

    // ── Finding #4: register_and_bind ordering has no observable window ──
    //
    // The reverse-index bind must be recorded **before** the `Manager`
    // row becomes discoverable, and a failed registration must leave no
    // reverse-index row. The discriminator is `ResourceConfig::validate`,
    // which `Manager::register` runs *before* it inserts (makes
    // discoverable) the registry row. A `validate` that snapshots
    // `fanout_index.affected(cid)` therefore observes:
    //   * register-then-bind ordering  → empty at validate (the bind has
    //     not run yet — RED), and
    //   * bind-then-register ordering   → non-empty at validate (the bind
    //     was staged first — GREEN).
    // budget-justified: deterministic register-then-bind ordering
    // characterization harness — one cohesive scenario (probe + fake
    // Resource + the no-observable-window assertion). Decomposing it
    // would hide the very ordering window the test exists to prove.
    #[cfg(feature = "rotation")]
    mod register_and_bind_ordering {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        use nebula_core::{
            CredentialKey, DeclaresDependencies, Dependencies, ResourceKey,
            dependencies::{SlotField, SlotKind},
            resource_key,
        };
        use nebula_credential::CredentialId;
        use nebula_expression::ExpressionEngine;
        use nebula_resource::{
            Manager, ScopeLevel,
            error::Error as ResourceError,
            resource::{Resource, ResourceConfig, ResourceMetadata},
            runtime::{TopologyRuntime, resident::ResidentRuntime},
            topology::resident,
        };
        use nebula_schema::HasSchema;

        use super::super::*;
        use nebula_resource::ResourceFanoutIndex;

        const SLOT_KEY: &str = "auth";

        /// Test observation channel: `OConfig::validate` snapshots, into
        /// `seen_at_validate`, how many rows the fanout index has bound
        /// for `cid` *at the instant validate runs* (i.e. before the
        /// `Manager` registry row is inserted / discoverable).
        struct Probe {
            idx: Arc<ResourceFanoutIndex>,
            cid: CredentialId,
            seen_at_validate: AtomicUsize,
            validate_should_fail: bool,
        }

        // One probe per test; each `#[tokio::test]` here installs its own
        // before driving a registration, so the slot is single-writer.
        static PROBE: std::sync::Mutex<Option<Arc<Probe>>> = std::sync::Mutex::new(None);

        fn install_probe(probe: Arc<Probe>) {
            match PROBE.lock() {
                Ok(mut g) => *g = Some(probe),
                Err(poisoned) => *poisoned.into_inner() = Some(probe),
            }
        }

        fn probe() -> Arc<Probe> {
            let guard = match PROBE.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let Some(p) = guard.clone() else {
                // guard-justified: test-only harness; every test here
                // installs a probe before driving a registration, so this
                // branch is structurally unreachable in the test flow.
                unreachable!("probe must be installed before registration")
            };
            p
        }

        fn cred_key(s: &str) -> CredentialKey {
            let Ok(k) = CredentialKey::new(s) else {
                // guard-justified: test-only; the call sites pass static
                // lowercase ASCII keys that always satisfy CredentialKey.
                unreachable!("static test credential key `{s}` must be valid")
            };
            k
        }

        #[derive(Debug, Clone)]
        struct OError(String);
        impl std::fmt::Display for OError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::error::Error for OError {}
        impl From<OError> for ResourceError {
            fn from(e: OError) -> Self {
                ResourceError::transient(e.0)
            }
        }

        #[derive(Clone, Debug, serde::Deserialize)]
        struct OConfig {
            #[serde(default)]
            label: String,
        }
        nebula_schema::impl_empty_has_schema!(OConfig);

        impl ResourceConfig for OConfig {
            fn validate(&self) -> Result<(), ResourceError> {
                // Runs inside `Manager::register`, BEFORE the registry row
                // is inserted. Snapshot the reverse-index state for `cid`:
                // a non-zero count here means the bind was staged before
                // register (the post-fix ordering).
                let p = probe();
                let bound = p.idx.affected(&p.cid).len();
                p.seen_at_validate.store(bound, Ordering::SeqCst);
                if p.validate_should_fail {
                    return Err(ResourceError::permanent("register intentionally fails"));
                }
                if self.label.is_empty() {
                    return Err(ResourceError::permanent("label must not be empty"));
                }
                Ok(())
            }
        }

        #[derive(Clone)]
        struct OResource;

        impl Resource for OResource {
            type Config = OConfig;
            type Runtime = ();
            type Lease = ();
            type Error = OError;

            fn key() -> ResourceKey {
                resource_key!("ordering.widget")
            }

            async fn create(
                &self,
                _config: &OConfig,
                _ctx: &nebula_resource::ResourceContext,
            ) -> Result<(), OError> {
                Ok(())
            }

            fn metadata() -> ResourceMetadata {
                ResourceMetadata::new(
                    <Self as Resource>::key(),
                    "ordering.widget".to_owned(),
                    String::new(),
                    <OConfig as HasSchema>::schema(),
                )
            }
        }

        impl DeclaresDependencies for OResource {
            fn dependencies() -> Dependencies {
                Dependencies::new().slot_field(SlotField {
                    slot_key: SLOT_KEY,
                    default_id: SLOT_KEY,
                    kind: SlotKind::Credential {
                        type_id: std::any::TypeId::of::<()>(),
                        type_name: "test-credential",
                        key: cred_key("auth"),
                    },
                    required: true,
                    lazy: false,
                })
            }
        }

        impl resident::Resident for OResource {
            fn is_alive_sync(&self, _runtime: &()) -> bool {
                true
            }
        }

        fn registry() -> ResourceRegistrarRegistry {
            let mut reg = ResourceRegistrarRegistry::new();
            reg.insert(
                "ordering.widget",
                Arc::new(TypedResourceRegistrar::<OResource, _, _, _>::new(
                    || OResource,
                    || {
                        TopologyRuntime::Resident(ResidentRuntime::<OResource>::new(
                            resident::config::Config::default(),
                        ))
                    },
                    || Manager::erased_acquire_resident_for::<OResource>(),
                )),
            );
            reg
        }

        fn request(expr: &ExpressionEngine, cid: CredentialId) -> RegisterRequest<'_> {
            let mut slot_bindings = HashMap::new();
            slot_bindings.insert(SLOT_KEY.to_owned(), cred_key("cred-tenant-a"));
            let mut credential_ids = HashMap::new();
            credential_ids.insert(SLOT_KEY.to_owned(), cid);
            RegisterRequest {
                config_json: serde_json::json!({ "label": "x" }),
                expr_engine: expr,
                slot_bindings,
                credential_ids,
                scope: ScopeLevel::Global,
                recovery_gate: None,
            }
        }

        /// Two properties, asserted sequentially in **one** test so the
        /// process-global `PROBE` cannot race a sibling test thread:
        ///
        /// 1. *No observable window.* At the instant the resource is being
        ///    registered (`validate`, which `Manager::register` runs
        ///    *before* it inserts / makes discoverable the registry row),
        ///    the reverse-index bind already exists. Equivalent statement:
        ///    the fan-out can never see a discoverable `Manager` row whose
        ///    reverse-index row is missing. (RED against register-then-bind
        ///    ordering: validate observes zero bound rows because
        ///    `idx.bind` has not run yet.) Also pins the staged identity ==
        ///    the manager-returned identity (no dual-derive divergence).
        ///
        /// 2. *Failed register leaves no reverse-index row.* Even though
        ///    the bind is staged *before* `register`, the scopeguard
        ///    compensation removes the staged row when `register` returns
        ///    `Err`; neither the `Manager` row nor the reverse-index row
        ///    survives (no orphan fan-out rows).
        #[tokio::test]
        async fn register_and_bind_has_no_observable_window() {
            // ── Property 1: success path, bind visible pre-discoverable ──
            {
                let manager = Manager::new();
                let expr = ExpressionEngine::with_cache_size(16);
                let reg = registry();
                let idx = Arc::new(ResourceFanoutIndex::new());
                let cid = CredentialId::new();

                install_probe(Arc::new(Probe {
                    idx: Arc::clone(&idx),
                    cid,
                    seen_at_validate: AtomicUsize::new(usize::MAX),
                    validate_should_fail: false,
                }));

                let result = reg
                    .register_and_bind("ordering.widget", &manager, request(&expr, cid), Some(&idx))
                    .await;
                assert!(
                    result.is_ok(),
                    "registration of a credential-bound resource must succeed: {result:?}"
                );
                let Ok(outcome) = result else { return };

                // The reverse-index bind was observable from inside
                // `validate`, which runs strictly before the registry row
                // is inserted / discoverable. Zero == the dangerous window.
                assert_eq!(
                    probe().seen_at_validate.load(Ordering::SeqCst),
                    1,
                    "the reverse-index bind must be recorded BEFORE the \
                     Manager row becomes discoverable — a zero here is the \
                     silent-miss window (row live, reverse-index row absent)"
                );

                assert!(
                    manager.has_registered_for_identity(
                        &<OResource as Resource>::key(),
                        &ScopeLevel::Global,
                        &outcome.slot_identity,
                    ),
                    "registered row must be resolvable under the recorded identity"
                );
                let affected = idx.affected(&cid);
                assert_eq!(affected.len(), 1, "exactly one bound row for the cid");
                assert_eq!(
                    affected[0].slot_identity, outcome.slot_identity,
                    "the staged reverse-index identity must equal the \
                     identity Manager::register_resolved returned (no \
                     dual-derive divergence)"
                );
            }

            // ── Property 2: failed register ⇒ no orphan reverse-index ──
            {
                let manager = Manager::new();
                let expr = ExpressionEngine::with_cache_size(16);
                let reg = registry();
                let idx = Arc::new(ResourceFanoutIndex::new());
                let cid = CredentialId::new();

                install_probe(Arc::new(Probe {
                    idx: Arc::clone(&idx),
                    cid,
                    seen_at_validate: AtomicUsize::new(usize::MAX),
                    validate_should_fail: true,
                }));

                let result = reg
                    .register_and_bind("ordering.widget", &manager, request(&expr, cid), Some(&idx))
                    .await;
                assert!(
                    matches!(result, Err(RegistrarError::Register { .. })),
                    "registration must fail inside validate: {result:?}"
                );

                // The bind WAS staged before register (the inversion is
                // active even on the failure path)…
                assert_eq!(
                    probe().seen_at_validate.load(Ordering::SeqCst),
                    1,
                    "the bind is staged before register even on the failing path"
                );
                // …but the scopeguard compensation removed it.
                assert!(
                    idx.affected(&cid).is_empty(),
                    "a failed registration must leave NO reverse-index row \
                     (scopeguard compensation must undo the staged bind)"
                );
                assert!(
                    !manager.has_registered_for_identity(
                        &<OResource as Resource>::key(),
                        &ScopeLevel::Global,
                        &SlotIdentity::from_bindings([(SLOT_KEY, "cred-tenant-a")]),
                    ),
                    "a failed registration must create no Manager registry row"
                );
            }
        }
    }
}
