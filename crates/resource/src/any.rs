//! Object-safe supertrait for resource dependency declaration.
use std::any::Any;

use nebula_core::ResourceKey;

use crate::metadata::ResourceMetadata;
use crate::resource::Resource;

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
        R::declare_key()
    }

    fn resource_metadata(&self) -> ResourceMetadata {
        self.metadata()
    }
}
