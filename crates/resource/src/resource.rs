//! Core resource traits (bb8-style).
//!
//! The `Resource` trait defines how to create, validate, recycle, and clean up
//! resource instances. Each resource type exposes a **canonical [`ResourceKey`]**
//! used across manager, events, errors, and metrics.

use std::any::Any;
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
/// Each resource type has an associated `Config`, `Instance`, and a canonical
/// key that is the single source of truth for manager indexing, events, and metrics.
pub trait Resource: Send + Sync + 'static {
    /// The configuration type for this resource.
    type Config: Config;

    /// The instance type produced by this resource.
    type Instance: Send + Sync + 'static;

    /// Canonical key identifying this resource type.
    ///
    /// This is the **single source of truth** for the resource's identity.
    /// The manager indexes pools by this key; events and metrics use it too.
    fn key(&self) -> ResourceKey;

    /// Static metadata (display name, description, tags, icon) for UI and discovery.
    ///
    /// Defaults to a minimal [`ResourceMetadata`] derived from [`key()`](Self::key).
    /// Override to provide richer metadata (display name, description, tags, icon).
    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(self.key())
    }

    /// Create a new instance from config and context.
    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl Future<Output = Result<Self::Instance>> + Send;

    /// Check whether an existing instance can be safely reused.
    ///
    /// Return `Ok(true)` when the instance is reusable, `Ok(false)` when it
    /// should be discarded, and `Err(_)` on validation failure.
    fn is_reusable(&self, _instance: &Self::Instance) -> impl Future<Output = Result<bool>> + Send {
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
}

/// Object-safe supertrait for declaring resource dependencies.
///
/// `Resource` and `Action` return `Vec<Box<dyn AnyResource>>` to declare
/// "I need resources of these types." The engine uses `Any::type_id()` on
/// `dyn AnyResource` to identify the resource type at registration time.
///
/// Automatically implemented for all `R: Resource` via the blanket impl below.
pub trait AnyResource: Any + Send + Sync + 'static {
    /// The normalized key identifying this resource type.
    fn resource_key(&self) -> ResourceKey;

    /// Metadata for this resource type.
    fn resource_metadata(&self) -> ResourceMetadata;
}

/// Blanket impl: every `Resource` is automatically an `AnyResource`.
impl<R: Resource + 'static> AnyResource for R {
    fn resource_key(&self) -> ResourceKey {
        self.key()
    }

    fn resource_metadata(&self) -> ResourceMetadata {
        self.metadata()
    }
}

/// Declarative dependency declaration for resources.
///
/// Implement this trait on a `Resource` type to declare which
/// sub-resource dependencies it requires. The engine calls these methods
/// at registration time to build the dependency graph automatically.
///
/// Methods use `where Self: Sized` so they are not in the vtable and can
/// only be called on concrete types at registration time.
pub trait ResourceDependencies {
    /// Sub-resources required by this resource.
    fn resources() -> Vec<Box<dyn AnyResource>>
    where
        Self: Sized,
    {
        vec![]
    }
}

/// Blanket impl: every `Resource` automatically satisfies `ResourceDependencies`
/// with an empty dependency list. Override by adding an explicit
/// `impl ResourceDependencies for MyResource { fn resources() -> … }`.
impl<T: Resource> ResourceDependencies for T {}
