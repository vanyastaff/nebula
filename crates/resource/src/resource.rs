//! Core resource trait and supporting types.
//!
//! [`Resource`] is the central abstraction: it describes how to create,
//! health-check, and tear down a single resource type. Implementors supply
//! four associated types and the lifecycle methods (Phase 4 / slot model).
//!
//! Per slot model (supersedes credential isolation) the singular `type Credential`
//! associated type was deleted in favor of typed credential **slot fields**
//! declared on the resource struct via `#[credential(key = "...")]` (the
//! `#[derive(Resource)]` macro emits a `DeclaresDependencies` impl that
//! enumerates them).
//!
//! `Resource::create(&self, ctx)` no longer takes an explicit
//! `scheme: &<R::Credential as Credential>::Scheme` argument: the framework
//! resolves every declared `#[credential]` slot **before** invoking
//! `create`. Each slot field is a `SlotCell<CredentialGuard<C>>` cell; the
//! implementation reads the resolved guard through the `#[derive(Resource)]`-
//! emitted `<field>_slot()` accessor (`Option<Arc<CredentialGuard<C>>>`).
//!
//! Per-credential rotation is exposed via
//! [`Resource::on_credential_refresh`], which receives the **slot name**
//! that rotated and the live `Runtime` handle (so multi-credential
//! resources can choose to refresh only the affected pool, headers, etc.
//! via interior mutability). Revocation is signalled via
//! [`Resource::on_credential_revoke`].

use std::future::Future;

use nebula_core::ResourceKey;

use crate::context::ResourceContext;

/// Trait-object-safe marker for resource type registration and discovery.
///
/// Unlike [`Resource`], this trait carries no associated types and can be
/// used as `dyn AnyResource`. Implementors typically also implement
/// [`Resource`], but this decoupling allows the engine to store heterogeneous
/// resource descriptors without generics.
pub trait AnyResource: Send + Sync + 'static {
    /// Returns the resource key.
    fn key(&self) -> ResourceKey;

    /// Returns resource metadata for UI and diagnostics.
    fn metadata(&self) -> ResourceMetadata;
}

/// Operational configuration for a resource. Contains NO secrets.
///
/// Implementors typically derive `serde::Deserialize` and hold fields like
/// host, port, pool size, timeouts, etc.
///
/// Must implement [`HasSchema`](nebula_schema::HasSchema) so the resource
/// metadata can auto-derive its configuration schema from the config type.
/// Use `()` / `bool` / `String` for schema-less stubs — baseline impls in
/// `nebula-schema` cover primitives with empty schemas.
pub trait ResourceConfig: nebula_schema::HasSchema + Send + Sync + Clone + 'static {
    /// Validates the configuration, returning an error if invalid.
    ///
    /// The default implementation accepts all configurations.
    fn validate(&self) -> Result<(), crate::Error> {
        Ok(())
    }

    /// Returns a fingerprint for change-detection during hot-reload.
    ///
    /// Two configs with the same fingerprint are treated as identical.
    /// The default returns `0` (always different).
    fn fingerprint(&self) -> u64 {
        0
    }
}

/// Resource metadata for UI and diagnostics.
///
/// The shared catalog prefix (`key`, `name`, `description`, `schema`, `icon`,
/// `documentation_url`, `tags`, `maturity`, `deprecation`) lives on the
/// composed [`BaseMetadata`](nebula_metadata::BaseMetadata). Resource has no
/// additional top-level metadata fields today — every catalog-level concern
/// lives on the shared base.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ResourceMetadata {
    /// Shared catalog prefix.
    pub base: nebula_metadata::BaseMetadata<ResourceKey>,
}

impl nebula_metadata::Metadata for ResourceMetadata {
    type Key = ResourceKey;
    fn base(&self) -> &nebula_metadata::BaseMetadata<ResourceKey> {
        &self.base
    }
}

/// Compatibility validation errors for resource metadata evolution.
///
/// Wraps [`nebula_metadata::BaseCompatError`] for parity with the
/// action- and credential-side error shapes. Resource has no
/// entity-specific compat rules today, so the enum currently has a
/// single `Base` variant; new variants will be added here (alongside
/// `Base`) if `ResourceMetadata` later gains entity-specific fields
/// whose changes should break version compatibility.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<ResourceKey>),
}

impl ResourceMetadata {
    /// Build resource metadata with explicit catalog-level fields.
    #[must_use]
    pub fn builder(
        key: ResourceKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> ResourceMetadataBuilder {
        ResourceMetadataBuilder {
            inner: Self::new(key, name, description, nebula_schema::ValidSchema::empty()),
        }
    }

    /// Build resource metadata with explicit catalog-level fields.
    pub fn new(
        key: ResourceKey,
        name: impl Into<String>,
        description: impl Into<String>,
        schema: nebula_schema::ValidSchema,
    ) -> Self {
        Self {
            base: nebula_metadata::BaseMetadata::new(key, name, description, schema),
        }
    }

    /// Build resource metadata whose schema is auto-derived from a
    /// [`Resource`] implementation's `Config` type.
    pub fn for_resource<R>(
        key: ResourceKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self
    where
        R: Resource,
    {
        Self::new(
            key,
            name,
            description,
            <R::Config as nebula_schema::HasSchema>::schema(),
        )
    }

    /// Create minimal metadata derived from a key — uses the key as the
    /// display name, an empty description, and an empty schema. Convenient
    /// for in-process resources that never show up in a user-facing catalog.
    pub fn from_key(key: &ResourceKey) -> Self {
        Self::new(
            key.clone(),
            key.to_string(),
            String::new(),
            nebula_schema::ValidSchema::empty(),
        )
    }

    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Delegates `key immutable / version monotonic / schema-break-requires-
    /// major` to [`nebula_metadata::validate_base_compat`]. Resource has no
    /// entity-specific rules today, so the wrapper enum has only the `Base`
    /// variant; the wrapper exists for shape parity with `nebula-action` and
    /// `nebula-credential`, so callers can match the error across all three
    /// catalog-leaf consumers uniformly.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;
        Ok(())
    }
}

/// Fluent builder for [`ResourceMetadata`].
///
/// Obtain one via [`ResourceMetadata::builder`] and call
/// [`build`](ResourceMetadataBuilder::build) when done.
#[derive(Debug, Clone)]
pub struct ResourceMetadataBuilder {
    inner: ResourceMetadata,
}

impl ResourceMetadataBuilder {
    /// Set the configuration schema for this resource.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_schema(mut self, schema: nebula_schema::ValidSchema) -> Self {
        self.inner.base.schema = schema;
        self
    }

    /// Set the interface version from `(major, minor)` components.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version(mut self, major: u64, minor: u64) -> Self {
        self.inner.base.version = semver::Version::new(major, minor, 0);
        self
    }

    /// Finalise the builder and return the [`ResourceMetadata`].
    #[must_use]
    pub fn build(self) -> ResourceMetadata {
        self.inner
    }
}

/// Core resource trait — 4 associated types + lifecycle methods (Phase 4 / slot model).
///
/// Uses return-position `impl Future` (RPITIT) instead of `async_trait`,
/// which avoids the `Box<dyn Future>` allocation on every call.
///
/// Per slot model (supersedes credential isolation) the singular `type Credential`
/// associated type was removed in favor of typed credential **slot
/// fields** on the resource struct (declared via `#[credential(...)]`
/// field attributes; the `#[derive(Resource)]` macro emits an impl of
/// [`nebula_core::DeclaresDependencies`] enumerating them). Each slot
/// field is a `SlotCell<CredentialGuard<C>>` cell. The framework
/// resolves slot fields **before** calling [`create`](Self::create) —
/// implementors read each resolved guard through the derive-emitted
/// `<field>_slot()` accessor, which returns
/// `Option<Arc<CredentialGuard<C>>>`, never off the raw cell field.
///
/// # Associated types
///
/// | Type | Purpose |
/// |------|---------|
/// | `Config` | Operational config (no secrets) |
/// | `Runtime` | The live resource handle (connection, client, etc.) |
/// | `Lease` | What callers hold while using the resource |
/// | `Error` | Resource-specific error, convertible to [`crate::Error`] |
///
/// # Lifecycle
///
/// ```text
/// create() → Runtime    (slot fields already resolved)
///   ↓
/// check()  → Ok(()) | Err
///   ↓
/// shutdown() → graceful wind-down
///   ↓
/// destroy()  → final cleanup (consumes Runtime)
/// ```
pub trait Resource: Send + Sync + 'static {
    /// Operational configuration type (no secrets).
    type Config: ResourceConfig;
    /// The live resource handle.
    type Runtime: Send + Sync + 'static;
    /// What callers hold during use.
    type Lease: Send + Sync + 'static;
    /// Resource-specific error type.
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new runtime instance from config.
    ///
    /// Credential slot cells declared via `#[credential(key = "...")]`
    /// are already populated on `&self` by the framework before this
    /// call (per slot model). Implementations read each resolved guard
    /// through the derive-emitted `self.<field>_slot()` accessor
    /// (`Option<Arc<CredentialGuard<C>>>`) — handling the `None`
    /// (unbound) case explicitly — never off the raw cell field.
    fn create(
        &self,
        config: &Self::Config,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Called by the engine rotation fan-out after it has swapped the
    /// rotated credential into this resource's slot. `&self`: the resource
    /// impl is an immutable descriptor; blue-green / re-auth acts on
    /// `runtime`'s own interior mutability. `slot_name` identifies which
    /// `#[credential]` slot rotated.
    ///
    /// Multi-credential resources can choose to refresh only the affected
    /// sub-system (e.g. swap a single pool, refresh a single header) rather
    /// than recycling the whole runtime. Connection-bound resources (Pool,
    /// Service, Transport) typically override with the blue-green swap
    /// pattern: build a fresh pool from the rotated credential, atomically
    /// swap into an `Arc<RwLock<Pool>>`, let RAII drain old handles.
    ///
    /// **Invariant** per slot model §Seam: implementer must handle every
    /// declared credential slot name; the engine emits a `WARN
    /// [resource]` if rotation arrives for an unhandled slot.
    ///
    /// Cancellation safety: implementations MUST be cancel-safe — if
    /// the returned future is dropped mid-await, the resource MUST
    /// remain consistent.
    ///
    /// Default: no-op.
    fn on_credential_refresh(
        &self,
        slot_name: &str,
        runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (slot_name, runtime);
        async { Ok(()) }
    }

    /// Called by the engine fan-out when a slot's credential is revoked.
    /// Post-invocation invariant (slot model): the resource emits no further
    /// authenticated traffic on the revoked credential. Default: no-op
    /// (the engine still taints + drains the runtime around this call).
    fn on_credential_revoke(
        &self,
        slot_name: &str,
        runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (slot_name, runtime);
        async { Ok(()) }
    }

    /// An opaque change-token over **all** of this resource's
    /// `#[credential]` slot field generations — the resource's current
    /// *credential epoch*.
    ///
    /// **Contract:** the returned value **changes whenever ANY slot's
    /// generation changes** — not just the slot with the largest
    /// generation. It is compared **only for equality** by the
    /// create-vs-rotate reconcile (built-epoch vs live-epoch), never by
    /// magnitude, so it is a change-token rather than a monotone counter.
    ///
    /// `0` means "no credential slot has ever been bound" (also the default
    /// for resources with no credential slots). Each `SlotCell` transition
    /// (`store` *or* `take`/clear) strictly advances its per-slot
    /// generation, so any post-build slot transition yields a different
    /// epoch — proving a credential rotated/was revoked in between.
    ///
    /// `#[derive(Resource)]` emits the real implementation: an
    /// order-sensitive positional fold
    /// (`acc = acc * K + slot.generation()`, fixed odd `K`) over every
    /// declared `#[credential]` field's
    /// [`SlotCell::generation`](crate::SlotCell::generation). A plain
    /// `max` would be wrong here — a runtime built at
    /// `(slot_a=5, slot_b=10)` then rotated `slot_a→6` still maxes to
    /// `10`, so the reconcile would miss the now-stale runtime; the
    /// positional fold changes on every slot transition regardless of
    /// which slot moved. It is derive-generated, not author-maintained,
    /// so a new slot field cannot be silently omitted from the epoch. The
    /// default `0` keeps hand-written impls (and slot-less resources)
    /// compiling; for such impls the create-vs-rotate reconcile degrades
    /// to the runtime-presence path only (it never *falsely* reports a
    /// stale runtime, it just cannot prove staleness by epoch).
    ///
    /// Used by the per-slot rotation dispatch and the Resident create slow
    /// path to close the create-vs-rotate lost-update race (resource runtime status
    /// §Deferred): the Resident runtime records the epoch it was built
    /// against and the dispatch reconciles a runtime built against an
    /// older epoch instead of silently reporting success.
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    /// Health-checks an existing runtime.
    ///
    /// The default implementation always succeeds.
    fn check(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Gracefully winds down a runtime (e.g., drain connections).
    ///
    /// The default implementation is a no-op.
    fn shutdown(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Final cleanup — consumes the runtime.
    ///
    /// The default implementation drops the runtime.
    fn destroy(
        &self,
        runtime: Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = runtime;
        async { Ok(()) }
    }

    /// Returns the schema for this resource's configuration.
    ///
    /// Default: derives from `Config` via [`HasSchema`](nebula_schema::HasSchema).
    fn schema() -> nebula_schema::ValidSchema
    where
        Self: Sized,
    {
        <Self::Config as nebula_schema::HasSchema>::schema()
    }

    /// Returns metadata for UI and diagnostics.
    fn metadata() -> ResourceMetadata
    where
        Self: Sized,
    {
        ResourceMetadata::new(
            Self::key(),
            Self::key().to_string(),
            String::new(),
            Self::schema(),
        )
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::resource_key;
    use nebula_metadata::BaseCompatError;
    use semver::Version;

    use super::{MetadataCompatibilityError, ResourceMetadata};

    fn empty_schema() -> nebula_schema::ValidSchema {
        nebula_schema::ValidSchema::empty()
    }

    fn md(major: u64, minor: u64) -> ResourceMetadata {
        let mut m = ResourceMetadata::new(resource_key!("postgres"), "pg", "d", empty_schema());
        m.base.version = Version::new(major, minor, 0);
        m
    }

    #[test]
    fn version_monotonic_accepted() {
        let prev = md(1, 0);
        let next = md(1, 1);
        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn version_regression_rejected() {
        let prev = md(2, 1);
        let next = md(2, 0);
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::VersionRegressed { .. })
        ));
    }
}
