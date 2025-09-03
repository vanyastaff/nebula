//! Base traits for Nebula entities
//!
//! These traits provide common functionality that can be implemented
//! by various types throughout the system.

use std::fmt;

use super::id::{ExecutionId, NodeId, TenantId, UserId, WorkflowId};
use super::scope::ScopeLevel;

/// Trait for entities that have a scope
pub trait Scoped {
    /// Get the scope level for this entity
    fn scope(&self) -> &ScopeLevel;

    /// Check if this entity is in the given scope
    fn is_in_scope(&self, scope: &ScopeLevel) -> bool {
        self.scope().is_contained_in(scope)
    }

    /// Check if this entity is global
    fn is_global(&self) -> bool {
        self.scope().is_global()
    }

    /// Check if this entity is workflow-scoped
    fn is_workflow(&self) -> bool {
        self.scope().is_workflow()
    }

    /// Check if this entity is execution-scoped
    fn is_execution(&self) -> bool {
        self.scope().is_execution()
    }

    /// Check if this entity is action-scoped
    fn is_action(&self) -> bool {
        self.scope().is_action()
    }
}

/// Trait for entities that have execution context
pub trait HasContext {
    /// Get the execution ID if available
    fn execution_id(&self) -> Option<&ExecutionId>;

    /// Get the workflow ID if available
    fn workflow_id(&self) -> Option<&WorkflowId>;

    /// Get the node ID if available
    fn node_id(&self) -> Option<&NodeId>;

    /// Get the user ID if available
    fn user_id(&self) -> Option<&UserId>;

    /// Get the tenant ID if available
    fn tenant_id(&self) -> Option<&TenantId>;

    /// Check if this entity has execution context
    fn has_execution_context(&self) -> bool {
        self.execution_id().is_some()
    }

    /// Check if this entity has workflow context
    fn has_workflow_context(&self) -> bool {
        self.workflow_id().is_some()
    }

    /// Check if this entity has user context
    fn has_user_context(&self) -> bool {
        self.user_id().is_some()
    }

    /// Check if this entity has tenant context
    fn has_tenant_context(&self) -> bool {
        self.tenant_id().is_some()
    }
}

/// Trait for entities that can be identified
pub trait Identifiable {
    /// Get the unique identifier for this entity
    fn id(&self) -> &str;

    /// Get the display name for this entity
    fn name(&self) -> Option<&str> {
        None
    }

    /// Get the description for this entity
    fn description(&self) -> Option<&str> {
        None
    }

    /// Get the version of this entity
    fn version(&self) -> Option<&str> {
        None
    }

    /// Check if this entity has a name
    fn has_name(&self) -> bool {
        self.name().is_some()
    }

    /// Check if this entity has a description
    fn has_description(&self) -> bool {
        self.description().is_some()
    }

    /// Check if this entity has a version
    fn has_version(&self) -> bool {
        self.version().is_some()
    }
}

/// Trait for entities that can be validated
pub trait Validatable {
    /// The type of validation error
    type Error: std::error::Error + Send + Sync;

    /// Validate this entity
    fn validate(&self) -> Result<(), Self::Error>;

    /// Check if this entity is valid
    fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

/// Trait for entities that can be serialized and deserialized
pub trait Serializable: serde::Serialize + serde::de::DeserializeOwned {
    /// Serialize to JSON string
    fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize to JSON string with pretty formatting
    fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON string
    fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize to binary format
    fn to_binary(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize from binary format
    fn from_binary(data: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(data)
    }
}

/// Trait for entities that can be cloned
pub trait Cloneable: Clone {
    /// Create a deep copy of this entity
    fn clone_deep(&self) -> Self {
        self.clone()
    }
}

/// Trait for entities that can be compared
pub trait Comparable: PartialEq + Eq {
    /// Check if this entity equals another
    fn equals(&self, other: &Self) -> bool {
        self == other
    }

    /// Check if this entity is different from another
    fn differs_from(&self, other: &Self) -> bool {
        self != other
    }
}

/// Trait for entities that can be hashed
pub trait Hashable: std::hash::Hash {
    /// Get a hash of this entity
    fn hash_value(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        Hash::hash(self, &mut hasher);
        hasher.finish()
    }
}

/// Trait for entities that can be displayed
pub trait Displayable: fmt::Display {
    /// Get the display string
    fn display(&self) -> String {
        self.to_string()
    }

    /// Get a short display string
    fn display_short(&self) -> String {
        self.to_string()
    }

    /// Get a detailed display string
    fn display_detailed(&self) -> String {
        self.to_string()
    }
}

/// Trait for entities that can be debugged
pub trait Debuggable: fmt::Debug {
    /// Get a debug string
    fn debug(&self) -> String {
        format!("{:?}", self)
    }

    /// Get a pretty debug string
    fn debug_pretty(&self) -> String {
        format!("{:#?}", self)
    }
}

/// Trait for entities that can be converted to and from strings
pub trait StringConvertible {
    /// Convert to string
    fn to_string(&self) -> String;

    /// Convert from string
    fn from_string(s: &str) -> Result<Self, String>
    where
        Self: Sized;

    /// Check if the string representation is valid
    fn is_valid_string(s: &str) -> bool
    where
        Self: Sized;
}

/// Trait for entities that have metadata
pub trait HasMetadata {
    /// Get the metadata for this entity
    fn metadata(&self) -> &EntityMetadata;

    /// Get the creation timestamp
    fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata().created_at
    }

    /// Get the last modification timestamp
    fn modified_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata().modified_at
    }

    /// Get the tags for this entity
    fn tags(&self) -> &[String] {
        &self.metadata().tags
    }

    /// Check if this entity has a specific tag
    fn has_tag(&self, tag: &str) -> bool {
        self.metadata().tags.iter().any(|t| t == tag)
    }
}

/// Metadata for entities
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntityMetadata {
    /// When the entity was created
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,

    /// When the entity was last modified
    pub modified_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Tags associated with the entity
    pub tags: Vec<String>,

    /// Additional custom metadata
    pub custom: std::collections::HashMap<String, String>,
}

impl EntityMetadata {
    /// Create new metadata
    pub fn new() -> Self {
        Self {
            created_at: Some(chrono::Utc::now()),
            modified_at: Some(chrono::Utc::now()),
            tags: Vec::new(),
            custom: std::collections::HashMap::new(),
        }
    }

    /// Add a tag
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Add multiple tags
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }

    /// Add custom metadata
    pub fn with_custom(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom.insert(key.into(), value.into());
        self
    }

    /// Mark as modified
    pub fn mark_modified(&mut self) {
        self.modified_at = Some(chrono::Utc::now());
    }
}

impl Default for EntityMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl HasMetadata for EntityMetadata {
    fn metadata(&self) -> &EntityMetadata {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::super::id::{ExecutionId, NodeId, WorkflowId};
    use super::*;

    // Test implementation of Scoped
    #[derive(Debug)]
    struct TestScopedEntity {
        scope: ScopeLevel,
    }

    impl Scoped for TestScopedEntity {
        fn scope(&self) -> &ScopeLevel {
            &self.scope
        }
    }

    // Test implementation of HasContext
    #[derive(Debug)]
    struct TestContextEntity {
        execution_id: Option<ExecutionId>,
        workflow_id: Option<WorkflowId>,
        node_id: Option<NodeId>,
    }

    impl HasContext for TestContextEntity {
        fn execution_id(&self) -> Option<&ExecutionId> {
            self.execution_id.as_ref()
        }

        fn workflow_id(&self) -> Option<&WorkflowId> {
            self.workflow_id.as_ref()
        }

        fn node_id(&self) -> Option<&NodeId> {
            self.node_id.as_ref()
        }

        fn user_id(&self) -> Option<&UserId> {
            None
        }

        fn tenant_id(&self) -> Option<&TenantId> {
            None
        }
    }

    // Test implementation of Identifiable
    #[derive(Debug)]
    struct TestIdentifiableEntity {
        id: String,
        name: Option<String>,
        description: Option<String>,
        version: Option<String>,
    }

    impl Identifiable for TestIdentifiableEntity {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> Option<&str> {
            self.name.as_deref()
        }

        fn description(&self) -> Option<&str> {
            self.description.as_deref()
        }

        fn version(&self) -> Option<&str> {
            self.version.as_deref()
        }
    }

    #[test]
    fn test_scoped_trait() {
        let execution_id = ExecutionId::new();
        let entity = TestScopedEntity {
            scope: ScopeLevel::Execution(execution_id.clone()),
        };

        assert!(entity.is_execution());
        assert!(!entity.is_global());
        assert!(!entity.is_workflow());
        assert!(!entity.is_action());
    }

    #[test]
    fn test_has_context_trait() {
        let execution_id = ExecutionId::new();
        let workflow_id = WorkflowId::new("test-workflow");
        let node_id = NodeId::new("test-node");

        let entity = TestContextEntity {
            execution_id: Some(execution_id.clone()),
            workflow_id: Some(workflow_id.clone()),
            node_id: Some(node_id.clone()),
        };

        assert!(entity.has_execution_context());
        assert!(entity.has_workflow_context());
        assert_eq!(entity.execution_id(), Some(&execution_id));
        assert_eq!(entity.workflow_id(), Some(&workflow_id));
        assert_eq!(entity.node_id(), Some(&node_id));
    }

    #[test]
    fn test_identifiable_trait() {
        let entity = TestIdentifiableEntity {
            id: "test-id".to_string(),
            name: Some("Test Entity".to_string()),
            description: Some("A test entity".to_string()),
            version: Some("1.0.0".to_string()),
        };

        assert_eq!(entity.id(), "test-id");
        assert_eq!(entity.name(), Some("Test Entity"));
        assert_eq!(entity.description(), Some("A test entity"));
        assert_eq!(entity.version(), Some("1.0.0"));
        assert!(entity.has_name());
        assert!(entity.has_description());
        assert!(entity.has_version());
    }

    #[test]
    fn test_entity_metadata() {
        let mut metadata = EntityMetadata::new()
            .with_tag("test")
            .with_tag("example")
            .with_custom("key", "value");

        assert!(metadata.has_tag("test"));
        assert!(metadata.has_tag("example"));
        assert!(!metadata.has_tag("nonexistent"));
        assert_eq!(metadata.custom.get("key"), Some(&"value".to_string()));

        metadata.mark_modified();
        assert!(metadata.modified_at.is_some());
    }
}
