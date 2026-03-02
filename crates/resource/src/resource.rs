//! Core resource traits (bb8-style).
//!
//! The `Resource` trait defines how to create, validate, recycle, and clean up
//! resource instances. Each resource type exposes a **canonical [`ResourceKey`]**
//! used across manager, events, errors, and metrics.

use std::future::Future;

use crate::context::Context;
use crate::error::Result;
use crate::metadata::ResourceMetadata;
use nebula_core::ResourceKey;
use nebula_core::deps::FromRegistry;

/// Configuration trait for resource types.
pub trait Config: Send + Sync + 'static {
    /// Validate the configuration, returning an error if invalid.
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

/// Core resource trait (bb8-style).
///
/// Defines the full lifecycle: create, validate, recycle, cleanup.
/// Each resource type has an associated `Config`, `Instance`, and static
/// metadata describing the resource for UI and discovery.
pub trait Resource: Send + Sync + 'static {
    /// The configuration type for this resource.
    type Config: Config;

    /// The instance type produced by this resource.
    type Instance: Send + Sync + 'static;

    /// Statically declared dependencies for this resource type.
    ///
    /// Implementations must declare this explicitly using the
    /// [`Requires`](nebula_core::deps::Requires) marker and the
    /// [`deps!`](nebula_core::deps) macro. Use `deps![]` (or `()`) for
    /// leaf resources without dependencies.
    type Deps: FromRegistry;

    /// Static metadata (display name, description, tags, icon) for UI and discovery.
    ///
    /// Implementations **must** return a fully-populated [`ResourceMetadata`]
    /// value with a canonical [`ResourceKey`] in `metadata.key`. This key is
    /// used everywhere in `nebula-resource` as the logical identifier for the
    /// resource type.
    fn metadata(&self) -> ResourceMetadata;

    /// Canonical resource key for this resource type.
    ///
    /// Default implementation delegates to [`Self::metadata`] and clones
    /// the key. Override only if you need a cheaper representation; the
    /// recommended pattern is to keep `metadata()` as the single source
    /// of truth.
    fn key(&self) -> ResourceKey {
        self.metadata().key.clone()
    }

    /// Create a new instance from config and context.
    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl Future<Output = Result<Self::Instance>> + Send;

    /// Check whether an existing instance is still valid/healthy.
    fn is_valid(&self, _instance: &Self::Instance) -> impl Future<Output = Result<bool>> + Send {
        async { Ok(true) }
    }

    /// Recycle an instance before returning it to the pool.
    fn recycle(&self, _instance: &mut Self::Instance) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Clean up an instance when it is permanently removed.
    fn cleanup(&self, instance: Self::Instance) -> impl Future<Output = Result<()>> + Send {
        async {
            drop(instance);
            Ok(())
        }
    }

    // Existing ad-hoc dependency listing is kept for now for compatibility
    // with older code paths. Prefer using `type Deps` and the `deps!` macro.
    /// List resource keys that this resource depends on.
    fn dependencies(&self) -> Vec<ResourceKey> {
        Vec::new()
    }
}
