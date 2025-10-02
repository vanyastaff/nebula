//! Core resource traits and types

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::LifecycleState,
    scoping::ResourceScope,
    versioning::Version,
};

/// Unique identifier for a resource type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceId {
    /// The resource type name
    pub name: String,
    /// The resource version
    pub version: String,
    /// Optional namespace for resource organization
    pub namespace: Option<String>,
}

impl ResourceId {
    /// Create a new resource ID
    pub fn new<S: Into<String>>(name: S, version: S) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            namespace: None,
        }
    }

    /// Create a new resource ID with namespace
    pub fn with_namespace<S: Into<String>>(name: S, version: S, namespace: S) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            namespace: Some(namespace.into()),
        }
    }

    /// Get the full qualified name of the resource
    pub fn full_name(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{}/{}", ns, self.name),
            None => self.name.clone(),
        }
    }

    /// Get a unique string representation for this resource ID
    pub fn unique_key(&self) -> String {
        format!("{}:{}", self.full_name(), self.version)
    }

    /// Parse the version string as a semantic version
    pub fn parse_version(&self) -> ResourceResult<Version> {
        self.version.parse()
    }

    /// Check if this resource is compatible with another version
    pub fn is_compatible_with(&self, other: &ResourceId) -> ResourceResult<bool> {
        if self.name != other.name || self.namespace != other.namespace {
            return Ok(false);
        }

        let this_version = self.parse_version()?;
        let other_version = other.parse_version()?;

        Ok(this_version.is_compatible_with(&other_version))
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.unique_key())
    }
}

/// Metadata about a resource type
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceMetadata {
    /// Resource identifier
    pub id: ResourceId,
    /// Human-readable description
    pub description: String,
    /// Resource tags for categorization
    pub tags: std::collections::HashMap<String, String>,
    /// Whether this resource supports pooling
    pub poolable: bool,
    /// Whether this resource supports health checking
    pub health_checkable: bool,
    /// Whether this resource maintains state
    pub stateful: bool,
    /// Resource dependencies
    pub dependencies: Vec<ResourceId>,
    /// Default scope for this resource type
    pub default_scope: ResourceScope,
    /// Minimum compatible version (for migration compatibility)
    pub min_compatible_version: Option<String>,
    /// Deprecated versions
    pub deprecated_versions: Vec<String>,
}

impl ResourceMetadata {
    /// Create new resource metadata
    pub fn new(id: ResourceId, description: String) -> Self {
        Self {
            id,
            description,
            tags: std::collections::HashMap::new(),
            poolable: false,
            health_checkable: false,
            stateful: false,
            dependencies: Vec::new(),
            default_scope: ResourceScope::default(),
            min_compatible_version: None,
            deprecated_versions: Vec::new(),
        }
    }

    /// Add a tag to the metadata
    pub fn with_tag<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Mark the resource as poolable
    pub fn poolable(mut self) -> Self {
        self.poolable = true;
        self
    }

    /// Mark the resource as health checkable
    pub fn health_checkable(mut self) -> Self {
        self.health_checkable = true;
        self
    }

    /// Mark the resource as stateful
    pub fn stateful(mut self) -> Self {
        self.stateful = true;
        self
    }

    /// Add a dependency
    pub fn with_dependency(mut self, dependency: ResourceId) -> Self {
        self.dependencies.push(dependency);
        self
    }

    /// Set the default scope
    pub fn with_default_scope(mut self, scope: ResourceScope) -> Self {
        self.default_scope = scope;
        self
    }

    /// Set minimum compatible version
    pub fn with_min_compatible_version(mut self, version: impl Into<String>) -> Self {
        self.min_compatible_version = Some(version.into());
        self
    }

    /// Add a deprecated version
    pub fn with_deprecated_version(mut self, version: impl Into<String>) -> Self {
        self.deprecated_versions.push(version.into());
        self
    }

    /// Check if a version is deprecated
    pub fn is_version_deprecated(&self, version: &str) -> bool {
        self.deprecated_versions.iter().any(|v| v == version)
    }

    /// Validate version compatibility
    pub fn validate_version(&self, version: &Version) -> ResourceResult<Option<String>> {
        // Check if deprecated
        if self.is_version_deprecated(&version.to_string()) {
            return Ok(Some(format!(
                "Version {} of {} is deprecated",
                version, self.id.name
            )));
        }

        // Check minimum compatible version if specified
        if let Some(ref min_ver_str) = self.min_compatible_version {
            let min_ver = min_ver_str.parse::<Version>()?;
            if version < &min_ver {
                return Err(ResourceError::configuration(format!(
                    "Version {} is below minimum compatible version {}",
                    version, min_ver
                )));
            }
        }

        Ok(None)
    }
}

/// Configuration trait that all resource configurations must implement
pub trait ResourceConfig: Send + Sync + Clone + fmt::Debug {
    /// Validate the configuration
    fn validate(&self) -> ResourceResult<()> {
        Ok(())
    }

    /// Merge another configuration into this one
    fn merge(&mut self, other: Self);

    /// Get configuration as key-value pairs for debugging
    fn to_debug_map(&self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }
}

/// A resource instance that has been created and initialized
pub trait ResourceInstance: Send + Sync + Any {
    /// Get the unique instance ID
    fn instance_id(&self) -> Uuid;

    /// Get the resource ID for this instance
    fn resource_id(&self) -> &ResourceId;

    /// Get the current lifecycle state
    fn lifecycle_state(&self) -> LifecycleState;

    /// Get the context this instance was created with
    fn context(&self) -> &ResourceContext;

    /// Get instance creation timestamp
    fn created_at(&self) -> chrono::DateTime<chrono::Utc>;

    /// Get last access timestamp
    fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>>;

    /// Update last access timestamp
    ///
    /// Uses interior mutability to update timestamp without requiring &mut self
    fn touch(&self);

    /// Check if this instance can be safely terminated
    fn can_terminate(&self) -> bool {
        !matches!(self.lifecycle_state(), LifecycleState::InUse)
    }
}

/// Core trait that all resources must implement
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    /// The configuration type for this resource
    type Config: ResourceConfig;
    /// The instance type that this resource creates
    type Instance: ResourceInstance;

    /// Get metadata about this resource type
    fn metadata(&self) -> ResourceMetadata;

    /// Create a new instance of this resource
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance>;

    /// Initialize a created instance
    async fn initialize(&self, instance: &mut Self::Instance) -> ResourceResult<()> {
        // Default implementation does nothing
        Ok(())
    }

    /// Cleanup an instance when it's no longer needed
    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        // Default implementation does nothing
        Ok(())
    }

    /// Validate that an instance is still healthy and usable
    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        // Default implementation always returns true
        Ok(true)
    }

    /// Get the TypeId for this resource type (for type-safe operations)
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    /// Get dependencies for this resource type
    fn dependencies(&self) -> Vec<ResourceId> {
        self.metadata().dependencies
    }
}

/// Factory for creating resource instances with dependency injection
#[async_trait]
pub trait ResourceFactory: Send + Sync {
    /// Create a resource instance with all dependencies resolved
    async fn create_instance(
        &self,
        config: serde_json::Value,
        context: &ResourceContext,
        dependencies: &std::collections::HashMap<ResourceId, Arc<dyn Any + Send + Sync>>,
    ) -> ResourceResult<Arc<dyn Any + Send + Sync>>;

    /// Get the resource metadata
    fn metadata(&self) -> ResourceMetadata;

    /// Get the configuration schema for this resource
    fn config_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

/// A wrapper that provides type-safe access to resource instances
pub struct TypedResourceInstance<T> {
    /// The underlying instance
    pub instance: Arc<T>,
    /// Instance metadata
    pub metadata: ResourceInstanceMetadata,
}

/// Metadata about a specific resource instance
#[derive(Debug, Clone)]
pub struct ResourceInstanceMetadata {
    /// Unique instance identifier
    pub instance_id: Uuid,
    /// Resource type identifier
    pub resource_id: ResourceId,
    /// Current lifecycle state
    pub state: LifecycleState,
    /// Creation context
    pub context: ResourceContext,
    /// Creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last access timestamp
    pub last_accessed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Instance tags
    pub tags: std::collections::HashMap<String, String>,
}

impl<T> TypedResourceInstance<T>
where
    T: Send + Sync + 'static,
{
    /// Create a new typed resource instance
    pub fn new(instance: Arc<T>, metadata: ResourceInstanceMetadata) -> Self {
        Self { instance, metadata }
    }

    /// Get a reference to the underlying instance
    pub fn as_ref(&self) -> &T {
        &self.instance
    }

    /// Get the instance ID
    pub fn instance_id(&self) -> Uuid {
        self.metadata.instance_id
    }

    /// Get the resource ID
    pub fn resource_id(&self) -> &ResourceId {
        &self.metadata.resource_id
    }

    /// Get the current state
    pub fn state(&self) -> LifecycleState {
        self.metadata.state
    }

    /// Update the last accessed timestamp
    pub fn touch(&mut self) {
        self.metadata.last_accessed_at = Some(chrono::Utc::now());
    }
}

impl<T> Clone for TypedResourceInstance<T> {
    fn clone(&self) -> Self {
        Self {
            instance: Arc::clone(&self.instance),
            metadata: self.metadata.clone(),
        }
    }
}

impl<T> AsRef<T> for TypedResourceInstance<T> {
    fn as_ref(&self) -> &T {
        &self.instance
    }
}

impl<T> fmt::Debug for TypedResourceInstance<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedResourceInstance")
            .field("instance_id", &self.metadata.instance_id)
            .field("resource_id", &self.metadata.resource_id)
            .field("state", &self.metadata.state)
            .field("created_at", &self.metadata.created_at)
            .finish()
    }
}

/// Guard that automatically manages resource lifecycle
pub struct ResourceGuard<T> {
    /// The resource instance
    pub resource: Option<TypedResourceInstance<T>>,
    /// Callback for when the guard is dropped
    on_drop: Option<Box<dyn FnOnce(TypedResourceInstance<T>) + Send>>,
}

impl<T> ResourceGuard<T>
where
    T: Send + Sync + 'static,
{
    /// Create a new resource guard
    pub fn new<F>(resource: TypedResourceInstance<T>, on_drop: F) -> Self
    where
        F: FnOnce(TypedResourceInstance<T>) + Send + 'static,
    {
        Self {
            resource: Some(resource),
            on_drop: Some(Box::new(on_drop)),
        }
    }

    /// Get a reference to the resource
    pub fn as_ref(&self) -> Option<&T> {
        self.resource.as_ref().map(|r| r.as_ref())
    }

    /// Get the resource metadata
    pub fn metadata(&self) -> Option<&ResourceInstanceMetadata> {
        self.resource.as_ref().map(|r| &r.metadata)
    }

    /// Release the resource manually (consumes the guard)
    pub fn release(mut self) -> Option<TypedResourceInstance<T>> {
        self.resource.take()
    }
}

impl<T> Drop for ResourceGuard<T> {
    fn drop(&mut self) {
        if let (Some(resource), Some(on_drop)) = (self.resource.take(), self.on_drop.take()) {
            on_drop(resource);
        }
    }
}

impl<T> std::ops::Deref for ResourceGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.resource
            .as_ref()
            .expect("Resource guard used after release")
            .as_ref()
    }
}

impl<T> fmt::Debug for ResourceGuard<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceGuard")
            .field("resource", &self.resource)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_id() {
        let id = ResourceId::new("database", "1.0");
        assert_eq!(id.name, "database");
        assert_eq!(id.version, "1.0");
        assert_eq!(id.full_name(), "database");
        assert_eq!(id.unique_key(), "database:1.0");

        let namespaced_id = ResourceId::with_namespace("database", "1.0", "core");
        assert_eq!(namespaced_id.full_name(), "core/database");
        assert_eq!(namespaced_id.unique_key(), "core/database:1.0");
    }

    #[test]
    fn test_resource_metadata() {
        let id = ResourceId::new("test", "1.0");
        let metadata = ResourceMetadata::new(id.clone(), "Test resource".to_string())
            .with_tag("type", "test")
            .poolable()
            .health_checkable();

        assert_eq!(metadata.id, id);
        assert!(metadata.poolable);
        assert!(metadata.health_checkable);
        assert!(!metadata.stateful);
        assert_eq!(metadata.tags.get("type"), Some(&"test".to_string()));
    }
}
