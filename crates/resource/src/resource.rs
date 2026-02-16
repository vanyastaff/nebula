//! Core resource traits (bb8-style)
//!
//! The `Resource` trait defines how to create, validate, recycle, and clean up
//! resource instances.

use std::future::Future;

use crate::context::Context;
use crate::error::Result;

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
/// Each resource type has an associated `Config` and `Instance`.
pub trait Resource: Send + Sync + 'static {
    /// The configuration type for this resource.
    type Config: Config;

    /// The instance type produced by this resource.
    type Instance: Send + Sync + 'static;

    /// Unique string identifier for this resource type (e.g. "postgres", "redis").
    fn id(&self) -> &str;

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

    /// List resource IDs that this resource depends on.
    fn dependencies(&self) -> Vec<&str> {
        Vec::new()
    }
}
