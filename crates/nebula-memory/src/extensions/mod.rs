//! Extensions module for nebula-memory
//!
//! This module provides an extension system that allows customizing and
//! extending the memory management functionality without modifying the core
//! codebase.

use core::any::{Any, TypeId};
use core::fmt::Debug;
use parking_lot::RwLock;
use std::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use crate::error::{MemoryError, MemoryResult};

pub mod async_support;
/// Specialized extension modules
pub mod logging;
pub mod metrics;
pub mod serialization;

/// Trait that all memory extensions must implement
pub trait MemoryExtension: Any + Send + Sync {
    /// Returns the unique name of the extension
    fn name(&self) -> &str;

    /// Returns the version of the extension
    fn version(&self) -> &str;

    /// Returns the category of the extension
    fn category(&self) -> &str {
        "general"
    }

    /// Returns a list of tags for this extension
    fn tags(&self) -> Vec<&str> {
        Vec::new()
    }

    /// Called when the extension is registered
    fn on_register(&self) -> MemoryResult<()> {
        Ok(())
    }

    /// Called when the extension is unregistered
    fn on_unregister(&self) -> MemoryResult<()> {
        Ok(())
    }

    /// Returns extension metadata as a map of key-value pairs
    fn metadata(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Cast this extension to Any for dynamic downcasting
    fn as_any(&self) -> &dyn Any;
}

/// Registry for managing memory extensions
#[derive(Default)]
pub struct ExtensionRegistry {
    extensions: RwLock<BTreeMap<String, Arc<dyn MemoryExtension>>>,
    type_registry: RwLock<BTreeMap<TypeId, String>>,
}

impl ExtensionRegistry {
    /// Create a new empty extension registry
    pub fn new() -> Self {
        Self {
            extensions: RwLock::new(BTreeMap::new()),
            type_registry: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register a new extension
    pub fn register<E: MemoryExtension + 'static>(&self, extension: E) -> MemoryResult<()> {
        let ext_arc = Arc::new(extension);
        let name = ext_arc.name().to_string();
        let type_id = TypeId::of::<E>();

        // Check for duplicate extension name
        if self.extensions.read().contains_key(&name) {
            return Err(MemoryError::InvalidOperation(format!(
                "Extension with name '{}' is already registered",
                name
            )));
        }

        // Call extension's registration hook
        ext_arc.on_register()?;

        // Register the extension
        self.extensions.write().insert(name.clone(), ext_arc);
        self.type_registry.write().insert(type_id, name);

        Ok(())
    }

    /// Unregister an extension by name
    pub fn unregister(&self, name: &str) -> MemoryResult<()> {
        let mut extensions = self.extensions.write();

        if let Some(ext) = extensions.remove(name) {
            // Call extension's unregistration hook
            ext.on_unregister()?;

            // Remove from type registry
            let mut type_registry = self.type_registry.write();
            let type_ids_to_remove: Vec<TypeId> = type_registry
                .iter()
                .filter_map(|(type_id, ext_name)| {
                    if ext_name == name {
                        Some(*type_id)
                    } else {
                        None
                    }
                })
                .collect();

            for type_id in type_ids_to_remove {
                type_registry.remove(&type_id);
            }

            Ok(())
        } else {
            Err(MemoryError::NotFound(format!(
                "Extension with name '{}' not found",
                name
            )))
        }
    }

    /// Get extension by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn MemoryExtension>> {
        self.extensions.read().get(name).cloned()
    }

    /// Get extension by type
    pub fn get_by_type<E: MemoryExtension + 'static>(&self) -> Option<Arc<dyn MemoryExtension>> {
        let type_id = TypeId::of::<E>();
        let type_registry = self.type_registry.read();

        if let Some(name) = type_registry.get(&type_id) {
            let extensions = self.extensions.read();
            if let Some(ext) = extensions.get(name) {
                return Some(ext.clone());
            }
        }

        None
    }

    /// Get extension by type with downcast
    pub fn get_by_type_downcast<E: MemoryExtension + Clone + 'static>(&self) -> Option<E> {
        let extension = self.get_by_type::<E>()?;

        // Try to downcast the extension to the specific type
        extension.as_any().downcast_ref::<E>().cloned()
    }

    /// List all registered extensions
    pub fn list(&self) -> Vec<String> {
        self.extensions.read().keys().cloned().collect()
    }

    /// Find extensions by category
    pub fn find_by_category(&self, category: &str) -> Vec<Arc<dyn MemoryExtension>> {
        let extensions = self.extensions.read();
        extensions
            .values()
            .filter(|ext| ext.category() == category)
            .cloned()
            .collect()
    }

    /// Find extensions by tag
    pub fn find_by_tag(&self, tag: &str) -> Vec<Arc<dyn MemoryExtension>> {
        let extensions = self.extensions.read();
        extensions
            .values()
            .filter(|ext| ext.tags().contains(&tag))
            .cloned()
            .collect()
    }

    /// Find extensions using a predicate function
    pub fn find<F>(&self, predicate: F) -> Vec<Arc<dyn MemoryExtension>>
    where
        F: Fn(&Arc<dyn MemoryExtension>) -> bool,
    {
        let extensions = self.extensions.read();
        extensions
            .values()
            .filter(|ext| predicate(ext))
            .cloned()
            .collect()
    }

    /// Check if an extension is registered
    pub fn is_registered(&self, name: &str) -> bool {
        self.extensions.read().contains_key(name)
    }

    /// Get the number of registered extensions
    pub fn count(&self) -> usize {
        self.extensions.read().len()
    }
}

/// Global extension registry
#[derive(Debug, Clone, Copy)]
pub struct GlobalExtensions;

impl GlobalExtensions {
    fn registry() -> &'static ExtensionRegistry {
        // Use a static variable to store the global registry
        use std::sync::Once;
        static mut REGISTRY: Option<ExtensionRegistry> = None;
        static INIT: Once = Once::new();

        // SAFETY: Singleton pattern using Once for thread-safe initialization.
        // - call_once guarantees REGISTRY initialized exactly once
        // - No races: call_once blocks other threads until initialization completes
        // - After initialization, REGISTRY is only read (never written)
        // - unreachable!() branch never executes (call_once guarantees initialization)
        unsafe {
            INIT.call_once(|| {
                REGISTRY = Some(ExtensionRegistry::new());
            });

            // Access the static variable
            match &REGISTRY {
                Some(registry) => registry,
                None => unreachable!("REGISTRY should be initialized via INIT.call_once"),
            }
        }
    }

    /// Register a global extension
    pub fn register<E: MemoryExtension + 'static>(extension: E) -> MemoryResult<()> {
        Self::registry().register(extension)
    }

    /// Unregister a global extension
    pub fn unregister(name: &str) -> MemoryResult<()> {
        Self::registry().unregister(name)
    }

    /// Get a global extension by name
    pub fn get(name: &str) -> Option<Arc<dyn MemoryExtension>> {
        Self::registry().get(name)
    }

    /// Get a global extension by type
    pub fn get_by_type<E: MemoryExtension + 'static>() -> Option<Arc<dyn MemoryExtension>> {
        Self::registry().get_by_type::<E>()
    }

    /// Get a global extension by type with downcast
    pub fn get_by_type_downcast<E: MemoryExtension + Clone + 'static>() -> Option<E> {
        Self::registry().get_by_type_downcast::<E>()
    }

    /// Find global extensions by category
    pub fn find_by_category(category: &str) -> Vec<Arc<dyn MemoryExtension>> {
        Self::registry().find_by_category(category)
    }

    /// Find global extensions by tag
    pub fn find_by_tag(tag: &str) -> Vec<Arc<dyn MemoryExtension>> {
        Self::registry().find_by_tag(tag)
    }

    /// Find global extensions using a predicate function
    pub fn find<F>(predicate: F) -> Vec<Arc<dyn MemoryExtension>>
    where
        F: Fn(&Arc<dyn MemoryExtension>) -> bool,
    {
        Self::registry().find(predicate)
    }

    /// List all global extensions
    pub fn list() -> Vec<String> {
        Self::registry().list()
    }
}

// Helper for implementing MemoryExtension trait
#[macro_export]
macro_rules! impl_memory_extension {
    // Базовая реализация с именем и версией
    ($type:ty, $name:expr, $version:expr) => {
        impl $crate::extensions::MemoryExtension for $type {
            fn name(&self) -> &str {
                $name
            }

            fn version(&self) -> &str {
                $version
            }

            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
        }
    };

    // Расширенная реализация с категорией
    ($type:ty, $name:expr, $version:expr, category: $category:expr) => {
        impl $crate::extensions::MemoryExtension for $type {
            fn name(&self) -> &str {
                $name
            }

            fn version(&self) -> &str {
                $version
            }

            fn category(&self) -> &str {
                $category
            }

            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
        }
    };

    // Полная реализация с категорией и тегами
    ($type:ty, $name:expr, $version:expr, category: $category:expr, tags: [$($tag:expr),*]) => {
        impl $crate::extensions::MemoryExtension for $type {
            fn name(&self) -> &str {
                $name
            }

            fn version(&self) -> &str {
                $version
            }

            fn category(&self) -> &str {
                $category
            }

            fn tags(&self) -> Vec<&str> {
                vec![$($tag),*]
            }

            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
        }
    };
}

// Export the macro
pub use crate::impl_memory_extension;
