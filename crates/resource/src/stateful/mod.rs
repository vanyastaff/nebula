//! Stateful resource management

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::core::{
    error::{ResourceError, ResourceResult},
    resource::ResourceId,
    traits::Stateful,
};

/// Version information for state migration
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StateVersion {
    /// Major version number
    pub major: u32,
    /// Minor version number
    pub minor: u32,
    /// Patch version number
    pub patch: u32,
}

impl StateVersion {
    /// Create a new state version
    #[must_use]
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Check if this version is compatible with another
    #[must_use]
    pub fn is_compatible_with(&self, other: &StateVersion) -> bool {
        self.major == other.major
    }

    /// Check if this version is newer than another
    #[must_use]
    pub fn is_newer_than(&self, other: &StateVersion) -> bool {
        self.major > other.major
            || (self.major == other.major && self.minor > other.minor)
            || (self.major == other.major && self.minor == other.minor && self.patch > other.patch)
    }
}

impl std::fmt::Display for StateVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Persisted state information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PersistedState {
    /// Resource identifier
    pub resource_id: ResourceId,
    /// State version
    pub version: StateVersion,
    /// State data
    pub data: serde_json::Value,
    /// Timestamp when state was saved
    pub saved_at: chrono::DateTime<chrono::Utc>,
    /// Checksum for integrity verification
    pub checksum: String,
    /// Metadata
    pub metadata: HashMap<String, String>,
}

impl PersistedState {
    /// Create a new persisted state
    #[must_use]
    pub fn new(resource_id: ResourceId, version: StateVersion, data: serde_json::Value) -> Self {
        let saved_at = chrono::Utc::now();
        let checksum = Self::calculate_checksum(&data, &saved_at);

        Self {
            resource_id,
            version,
            data,
            saved_at,
            checksum,
            metadata: HashMap::new(),
        }
    }

    /// Verify the integrity of the persisted state
    #[must_use]
    pub fn verify_integrity(&self) -> bool {
        let expected_checksum = Self::calculate_checksum(&self.data, &self.saved_at);
        self.checksum == expected_checksum
    }

    /// Add metadata
    #[must_use]
    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }

    fn calculate_checksum(
        data: &serde_json::Value,
        timestamp: &chrono::DateTime<chrono::Utc>,
    ) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        data.to_string().hash(&mut hasher);
        timestamp.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

/// Trait for state persistence backends
#[async_trait]
pub trait StatePersistence: Send + Sync {
    /// Save state to the persistence backend
    async fn save_state(&self, state: &PersistedState) -> ResourceResult<()>;

    /// Load state from the persistence backend
    async fn load_state(&self, resource_id: &ResourceId) -> ResourceResult<Option<PersistedState>>;

    /// Delete state from the persistence backend
    async fn delete_state(&self, resource_id: &ResourceId) -> ResourceResult<()>;

    /// List all states for a resource type
    async fn list_states(&self, resource_type: &str) -> ResourceResult<Vec<ResourceId>>;

    /// Get the backend name
    fn backend_name(&self) -> &str;
}

/// In-memory state persistence (for testing and development)
#[derive(Debug)]
pub struct InMemoryStatePersistence {
    states: Arc<RwLock<HashMap<ResourceId, PersistedState>>>,
}

impl InMemoryStatePersistence {
    /// Create a new in-memory persistence backend
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryStatePersistence {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StatePersistence for InMemoryStatePersistence {
    async fn save_state(&self, state: &PersistedState) -> ResourceResult<()> {
        let mut states = self.states.write();
        states.insert(state.resource_id.clone(), state.clone());
        Ok(())
    }

    async fn load_state(&self, resource_id: &ResourceId) -> ResourceResult<Option<PersistedState>> {
        let states = self.states.read();
        Ok(states.get(resource_id).cloned())
    }

    async fn delete_state(&self, resource_id: &ResourceId) -> ResourceResult<()> {
        let mut states = self.states.write();
        states.remove(resource_id);
        Ok(())
    }

    async fn list_states(&self, resource_type: &str) -> ResourceResult<Vec<ResourceId>> {
        let states = self.states.read();
        Ok(states
            .keys()
            .filter(|id| id.name == resource_type)
            .cloned()
            .collect())
    }

    fn backend_name(&self) -> &'static str {
        "in_memory"
    }
}

/// State migration handler
#[async_trait]
pub trait StateMigration: Send + Sync {
    /// Migrate state from one version to another
    async fn migrate(
        &self,
        state: serde_json::Value,
        from_version: &StateVersion,
        to_version: &StateVersion,
    ) -> ResourceResult<serde_json::Value>;

    /// Check if migration is supported between versions
    fn supports_migration(&self, from: &StateVersion, to: &StateVersion) -> bool;

    /// Get the migration path between versions
    fn migration_path(&self, from: &StateVersion, to: &StateVersion) -> Vec<StateVersion>;
}

/// Default state migration that only supports identical versions
#[derive(Debug)]
pub struct NoOpStateMigration;

#[async_trait]
impl StateMigration for NoOpStateMigration {
    async fn migrate(
        &self,
        state: serde_json::Value,
        from_version: &StateVersion,
        to_version: &StateVersion,
    ) -> ResourceResult<serde_json::Value> {
        if from_version == to_version {
            Ok(state)
        } else {
            Err(ResourceError::internal(
                "migration",
                format!("Migration from {from_version} to {to_version} is not supported"),
            ))
        }
    }

    fn supports_migration(&self, from: &StateVersion, to: &StateVersion) -> bool {
        from == to
    }

    fn migration_path(&self, _from: &StateVersion, _to: &StateVersion) -> Vec<StateVersion> {
        vec![]
    }
}

/// Manager for stateful resources
pub struct StateManager {
    /// State persistence backend
    persistence: Arc<dyn StatePersistence>,
    /// State migration handler
    migration: Arc<dyn StateMigration>,
    /// Cache of loaded states
    cache: Arc<RwLock<HashMap<ResourceId, PersistedState>>>,
}

impl std::fmt::Debug for StateManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cache_size = self.cache.read().len();
        f.debug_struct("StateManager")
            .field("persistence", &"<trait object>")
            .field("migration", &"<trait object>")
            .field("cache_size", &cache_size)
            .finish()
    }
}

impl StateManager {
    /// Create a new state manager with in-memory persistence
    #[must_use]
    pub fn new() -> Self {
        Self::with_persistence(Arc::new(InMemoryStatePersistence::new()))
    }

    /// Create a new state manager with custom persistence
    pub fn with_persistence(persistence: Arc<dyn StatePersistence>) -> Self {
        Self {
            persistence,
            migration: Arc::new(NoOpStateMigration),
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the migration handler
    pub fn with_migration(mut self, migration: Arc<dyn StateMigration>) -> Self {
        self.migration = migration;
        self
    }

    /// Save state for a resource
    pub async fn save_state<T>(
        &self,
        resource_id: &ResourceId,
        resource: &T,
        version: StateVersion,
    ) -> ResourceResult<()>
    where
        T: Stateful,
    {
        let state_data = resource.save_state().await?;
        let serialized = serde_json::to_value(&state_data).map_err(|e| {
            ResourceError::internal(
                resource_id.unique_key(),
                format!("Failed to serialize state: {e}"),
            )
        })?;

        let persisted_state = PersistedState::new(resource_id.clone(), version, serialized);

        // Save to persistence backend
        self.persistence.save_state(&persisted_state).await?;

        // Update cache
        {
            let mut cache = self.cache.write();
            cache.insert(resource_id.clone(), persisted_state);
        }

        Ok(())
    }

    /// Load state for a resource
    pub async fn load_state<T>(
        &self,
        resource_id: &ResourceId,
        resource: &mut T,
        current_version: StateVersion,
    ) -> ResourceResult<bool>
    where
        T: Stateful,
    {
        // Try cache first
        if let Some(cached_state) = {
            let cache = self.cache.read();
            cache.get(resource_id).cloned()
        } {
            return self
                .restore_state_from_persisted(resource, &cached_state, &current_version)
                .await;
        }

        // Load from persistence
        let persisted_state = match self.persistence.load_state(resource_id).await? {
            Some(state) => state,
            None => return Ok(false), // No state found
        };

        // Verify integrity
        if !persisted_state.verify_integrity() {
            return Err(ResourceError::internal(
                resource_id.unique_key(),
                "State integrity check failed",
            ));
        }

        // Cache the loaded state
        {
            let mut cache = self.cache.write();
            cache.insert(resource_id.clone(), persisted_state.clone());
        }

        self.restore_state_from_persisted(resource, &persisted_state, &current_version)
            .await
    }

    /// Delete state for a resource
    pub async fn delete_state(&self, resource_id: &ResourceId) -> ResourceResult<()> {
        // Remove from cache
        {
            let mut cache = self.cache.write();
            cache.remove(resource_id);
        }

        // Delete from persistence
        self.persistence.delete_state(resource_id).await
    }

    /// List all states for a resource type
    pub async fn list_states(&self, resource_type: &str) -> ResourceResult<Vec<ResourceId>> {
        self.persistence.list_states(resource_type).await
    }

    /// Get statistics about the state manager
    #[must_use]
    pub fn stats(&self) -> StateManagerStats {
        let cache = self.cache.read();
        StateManagerStats {
            cached_states: cache.len(),
            backend_name: self.persistence.backend_name().to_string(),
        }
    }

    async fn restore_state_from_persisted<T>(
        &self,
        resource: &mut T,
        persisted_state: &PersistedState,
        current_version: &StateVersion,
    ) -> ResourceResult<bool>
    where
        T: Stateful,
    {
        let mut state_data = persisted_state.data.clone();

        // Migrate if necessary
        if &persisted_state.version != current_version {
            if !self
                .migration
                .supports_migration(&persisted_state.version, current_version)
            {
                return Err(ResourceError::internal(
                    persisted_state.resource_id.unique_key(),
                    format!(
                        "Migration from {} to {} is not supported",
                        persisted_state.version, current_version
                    ),
                ));
            }

            state_data = self
                .migration
                .migrate(state_data, &persisted_state.version, current_version)
                .await?;
        }

        // Deserialize and restore
        let typed_state: T::State = serde_json::from_value(state_data).map_err(|e| {
            ResourceError::internal(
                persisted_state.resource_id.unique_key(),
                format!("Failed to deserialize state: {e}"),
            )
        })?;

        resource.restore_state(typed_state).await?;
        Ok(true)
    }
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the state manager
#[derive(Debug, Clone)]
pub struct StateManagerStats {
    /// Number of states in cache
    pub cached_states: usize,
    /// Name of the persistence backend
    pub backend_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_version() {
        let v1 = StateVersion::new(1, 0, 0);
        let v2 = StateVersion::new(1, 1, 0);
        let v3 = StateVersion::new(2, 0, 0);

        assert!(v1.is_compatible_with(&v2));
        assert!(!v1.is_compatible_with(&v3));
        assert!(v2.is_newer_than(&v1));
        assert!(v3.is_newer_than(&v2));
    }

    #[test]
    fn test_persisted_state_integrity() {
        let resource_id = ResourceId::new("test", "1.0");
        let version = StateVersion::new(1, 0, 0);
        let data = serde_json::json!({"key": "value"});

        let state = PersistedState::new(resource_id, version, data);
        assert!(state.verify_integrity());

        // Modify the data (simulating corruption)
        let mut corrupted_state = state.clone();
        corrupted_state.data = serde_json::json!({"key": "different_value"});
        assert!(!corrupted_state.verify_integrity());
    }

    #[tokio::test]
    async fn test_in_memory_persistence() {
        let persistence = InMemoryStatePersistence::new();
        let resource_id = ResourceId::new("test", "1.0");
        let version = StateVersion::new(1, 0, 0);
        let data = serde_json::json!({"key": "value"});

        let state = PersistedState::new(resource_id.clone(), version, data);

        // Save and load
        persistence.save_state(&state).await.unwrap();
        let loaded = persistence.load_state(&resource_id).await.unwrap();
        assert!(loaded.is_some());

        // Delete
        persistence.delete_state(&resource_id).await.unwrap();
        let deleted = persistence.load_state(&resource_id).await.unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_state_manager() {
        let manager = StateManager::new();
        let stats = manager.stats();

        assert_eq!(stats.cached_states, 0);
        assert_eq!(stats.backend_name, "in_memory");
    }
}
