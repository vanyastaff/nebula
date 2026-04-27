//! Core resource trait and supporting types.
//!
//! [`Resource`] is the central abstraction: it describes how to create,
//! health-check, and tear down a single resource type. Implementors supply
//! five associated types and six lifecycle methods. The `Credential`
//! associated type carries the credential-binding contract per ADR-0036;
//! resources without an authenticated binding use
//! `type Credential = NoCredential` (re-exported from `nebula_credential`).

use std::future::Future;

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialContext, CredentialId, SchemeGuard};

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

/// Core resource trait — 5 associated types + 6 lifecycle methods.
///
/// Uses return-position `impl Future` (RPITIT) instead of `async_trait`,
/// which avoids the `Box<dyn Future>` allocation on every call.
///
/// Implementors supply five associated types and six lifecycle methods.
/// The `Credential` associated type carries the credential-binding
/// contract per ADR-0036; resources without an authenticated binding
/// use `type Credential = NoCredential` (re-exported from
/// `nebula_credential`).
///
/// # Associated types
///
/// | Type | Purpose |
/// |------|---------|
/// | `Config` | Operational config (no secrets) |
/// | `Runtime` | The live resource handle (connection, client, etc.) |
/// | `Lease` | What callers hold while using the resource |
/// | `Error` | Resource-specific error, convertible to [`crate::Error`] |
/// | `Credential` | Credential binding per ADR-0036 (use `NoCredential` to opt out) |
///
/// # Lifecycle
///
/// ```text
/// create() → Runtime
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
    /// The credential type bound to this resource per ADR-0036.
    ///
    /// Resources without an authenticated binding use
    /// [`NoCredential`](nebula_credential::NoCredential). The runtime
    /// projects `<Self::Credential as Credential>::Scheme` and threads it
    /// through [`create`](Self::create) and rotation hooks.
    type Credential: Credential;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new runtime instance from config and projected scheme material.
    ///
    /// `scheme` is borrowed from the credential subsystem; resources MUST NOT
    /// retain it past the returned future per `PRODUCT_CANON.md §12.5`
    /// (secret-handling discipline).
    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Called by the engine after a successful credential refresh.
    ///
    /// Default: no-op. Connection-bound resources (Pool, Service, Transport)
    /// override with the blue-green pool swap pattern per credential Tech
    /// Spec §15.7 — build a fresh pool from `new_scheme`, atomically swap
    /// into the resource's `Arc<RwLock<Pool>>`, let RAII drain old handles.
    ///
    /// `new_scheme` and `ctx` share the lifetime `'a`. The shared lifetime
    /// is the compile-time barrier preventing retention — see
    /// [`SchemeGuard`] Probe 6.
    /// Implementations MUST NOT store either argument past this call.
    ///
    /// Cancellation safety: implementations MUST be cancel-safe — if the
    /// returned future is dropped mid-await, the resource MUST remain
    /// consistent (`SchemeGuard`'s `ZeroizeOnDrop` fires deterministically
    /// across the cancellation boundary).
    ///
    /// **П1 status:** Manager-side dispatch is not wired in this PR; this
    /// method exists for impl ergonomics and forward-compat. П2 lands the
    /// reverse-index write + parallel `join_all` dispatcher.
    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let _ = (new_scheme, ctx);
        async { Ok(()) }
    }

    /// Called by the engine after a credential is revoked.
    ///
    /// Default: no-op. Override invariant per ADR-0036 §Decision:
    /// post-invocation, the resource MUST emit no further authenticated
    /// traffic on the revoked credential. The mechanism (destroy pool /
    /// mark-tainted / wait-for-drain / reject-new-acquires) is the
    /// implementor's choice; П2 Tech Spec §5 specifies typical patterns.
    ///
    /// **П1 status:** Manager-side dispatch is not wired in this PR; this
    /// method exists for impl ergonomics and forward-compat. П2 lands the
    /// reverse-index write + revocation dispatcher.
    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
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
    use nebula_schema::Schema;
    use semver::Version;

    use super::{MetadataCompatibilityError, ResourceMetadata};

    fn empty_schema() -> nebula_schema::ValidSchema {
        Schema::builder().build().unwrap()
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
