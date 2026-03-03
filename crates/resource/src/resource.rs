//! Core resource traits (bb8-style).
//!
//! The `Resource` trait defines how to create, validate, recycle, and clean up
//! resource instances. Each resource type exposes a **canonical [`ResourceKey`]**
//! used across manager, events, errors, and metrics.

use std::future::Future;

use nebula_core::ResourceKey;

use crate::context::Context;
use crate::error::Result;
use crate::metadata::ResourceMetadata;

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

    /// Static metadata (display name, description, tags, icon) for UI and discovery.
    ///
    /// Implementations **must** return a fully-populated [`ResourceMetadata`]
    /// value with a canonical [`ResourceKey`] in `metadata.key`. This key is
    /// used everywhere in `nebula-resource` as the logical identifier for the
    /// resource type.
    fn metadata(&self) -> ResourceMetadata;

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

    /// Default key for dependency declaration (e.g. `ResourceRef::of`).
    ///
    /// Override to provide a stable key; default derives from type name (snake_case).
    fn declare_key() -> ResourceKey
    where
        Self: Sized,
    {
        let name = std::any::type_name::<Self>();
        let short = name.rsplit("::").next().unwrap_or(name);
        let snake = camel_to_snake(short);
        ResourceKey::try_from(snake).expect("Resource type name must form valid ResourceKey")
    }
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else if c.is_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        }
    }
    out
}
