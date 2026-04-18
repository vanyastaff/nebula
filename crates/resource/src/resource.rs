//! Core resource trait and supporting types.
//!
//! [`Resource`] is the central abstraction: it describes how to create,
//! health-check, and tear down a single resource type. Implementors supply
//! five associated types and four lifecycle methods.

use std::future::Future;

use nebula_core::{AuthScheme, ResourceKey};

use crate::ctx::Ctx;

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
#[derive(Debug, Clone)]
pub struct ResourceMetadata {
    /// The unique key identifying this resource type.
    pub key: ResourceKey,
    /// Human-readable name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Freeform tags for categorization.
    pub tags: Vec<String>,
}

impl ResourceMetadata {
    /// Creates metadata with defaults derived from a resource key.
    pub fn from_key(key: &ResourceKey) -> Self {
        Self {
            key: key.clone(),
            name: key.to_string(),
            description: None,
            tags: Vec::new(),
        }
    }
}

/// Core resource trait — 5 associated types + 4 lifecycle methods.
///
/// Uses return-position `impl Future` (RPITIT) instead of `async_trait`,
/// which avoids the `Box<dyn Future>` allocation on every call.
///
/// # Associated types
///
/// | Type | Purpose |
/// |------|---------|
/// | `Config` | Operational config (no secrets) |
/// | `Runtime` | The live resource handle (connection, client, etc.) |
/// | `Lease` | What callers hold while using the resource |
/// | `Error` | Resource-specific error, convertible to [`crate::Error`] |
/// | `Auth` | Authentication scheme resolved by the credential system |
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
    /// Authentication scheme resolved by the credential system.
    ///
    /// Declares what auth material this resource needs (e.g., `SecretToken`,
    /// `IdentityPassword`). Use `()` for resources that require no authentication.
    type Auth: AuthScheme;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new runtime instance from config and auth material.
    fn create(
        &self,
        config: &Self::Config,
        auth: &Self::Auth,
        ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

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

    /// Returns metadata for UI and diagnostics.
    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}
