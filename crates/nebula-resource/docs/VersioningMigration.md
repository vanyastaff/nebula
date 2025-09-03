---
title:  VersioningMigration
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# State Versioning and Migration

## Overview

State versioning and migration in nebula-resource enables seamless evolution of resource state schemas while maintaining backward compatibility. The system supports automatic migration, rollback capabilities, and zero-downtime upgrades.

## Core Concepts

### Version Schema

````rust
use serde::{Deserialize, Serialize}

### Migration Manager Implementation

```rust
impl MigrationManager {
    /// Create new migration manager
    pub fn new(strategy: MigrationStrategy) -> Self {
        Self {
            migrations: BTreeMap::new(),
            strategy,
            validators: HashMap::new(),
            hooks: Vec::new(),
        }
    }
    
    /// Register a migration
    pub fn register_migration<M: StateMigration + 'static>(
        &mut self,
        migration: M,
    ) -> Result<()> {
        let key = (migration.from_version(), migration.to_version());
        
        if self.migrations.contains_key(&key) {
            return Err(Error::MigrationAlreadyExists {
                from: key.0,
                to: key.1,
            });
        }
        
        self.migrations.insert(key, Box::new(migration));
        Ok(())
    }
    
    /// Migrate state from one version to another
    pub async fn migrate_state(
        &self,
        state: &mut serde_json::Value,
        from_version: &Version,
        to_version: &Version,
    ) -> Result<MigrationReport> {
        let start_time = Instant::now();
        let mut report = MigrationReport::new(from_version.clone(), to_version.clone());
        
        // Find migration path
        let path = self.find_migration_path(from_version, to_version)?;
        
        if path.is_empty() {
            return Ok(report); // Already at target version
        }
        
        // Create backup for rollback
        let backup = state.clone();
        
        // Execute pre-migration hooks
        for hook in &self.hooks {
            hook.pre_migration(state, from_version, to_version).await?;
        }
        
        // Apply migrations in sequence
        let mut current_version = from_version.clone();
        
        for (from, to) in path {
            let migration = self.migrations
                .get(&(from.clone(), to.clone()))
                .ok_or_else(|| Error::MigrationNotFound { from, to })?;
            
            // Validate pre-conditions
            migration.validate_pre(state).await?;
            
            // Apply migration
            let result = migration.migrate(state).await?;
            
            // Validate post-conditions
            migration.validate_post(state).await?;
            
            // Record migration
            report.add_step(MigrationStep {
                from_version: from.clone(),
                to_version: to.clone(),
                migration_id: migration.migration_id(),
                result,
                duration: Instant::now() - start_time,
            });
            
            current_version = to;
        }
        
        // Execute post-migration hooks
        for hook in &self.hooks {
            hook.post_migration(state, from_version, to_version).await?;
        }
        
        // Validate final state
        if let Some(validator) = self.validators.get(to_version) {
            if let Err(e) = validator.validate(state).await {
                // Rollback on validation failure
                *state = backup;
                return Err(Error::MigrationValidationFailed {
                    version: to_version.clone(),
                    error: e.to_string(),
                });
            }
        }
        
        report.duration = Instant::now() - start_time;
        report.success = true;
        
        Ok(report)
    }
    
    /// Find migration path between versions
    fn find_migration_path(
        &self,
        from: &Version,
        to: &Version,
    ) -> Result<Vec<(Version, Version)>> {
        match &self.strategy {
            MigrationStrategy::Sequential => {
                self.find_sequential_path(from, to)
            }
            MigrationStrategy::ShortestPath => {
                self.find_shortest_path(from, to)
            }
            MigrationStrategy::Explicit => {
                self.find_explicit_path(from, to)
            }
            MigrationStrategy::Custom(finder) => {
                finder.find_path(from, to, &self.migrations)
            }
        }
    }
    
    /// Find sequential migration path
    fn find_sequential_path(
        &self,
        from: &Version,
        to: &Version,
    ) -> Result<Vec<(Version, Version)>> {
        let mut path = Vec::new();
        let mut current = from.clone();
        
        // Find all versions
        let mut versions: Vec<Version> = self.migrations
            .keys()
            .flat_map(|(f, t)| vec![f.clone(), t.clone()])
            .collect();
        versions.sort();
        versions.dedup();
        
        // Build path through versions
        while current < *to {
            let next = versions
                .iter()
                .find(|v| **v > current && self.migrations.contains_key(&(current.clone(), (*v).clone())))
                .ok_or_else(|| Error::NoMigrationPath {
                    from: from.clone(),
                    to: to.clone(),
                })?;
            
            path.push((current.clone(), next.clone()));
            current = next.clone();
        }
        
        Ok(path)
    }
    
    /// Find shortest migration path using BFS
    fn find_shortest_path(
        &self,
        from: &Version,
        to: &Version,
    ) -> Result<Vec<(Version, Version)>> {
        use std::collections::{VecDeque, HashMap};
        
        if from == to {
            return Ok(vec![]);
        }
        
        let mut queue = VecDeque::new();
        let mut visited = HashMap::new();
        
        queue.push_back(from.clone());
        visited.insert(from.clone(), None);
        
        // BFS to find shortest path
        while let Some(current) = queue.pop_front() {
            // Find all possible next versions
            for (f, t) in self.migrations.keys() {
                if f == &current && !visited.contains_key(t) {
                    visited.insert(t.clone(), Some(current.clone()));
                    queue.push_back(t.clone());
                    
                    if t == to {
                        // Reconstruct path
                        let mut path = Vec::new();
                        let mut node = to.clone();
                        
                        while let Some(Some(prev)) = visited.get(&node) {
                            path.push((prev.clone(), node.clone()));
                            node = prev.clone();
                        }
                        
                        path.reverse();
                        return Ok(path);
                    }
                }
            }
        }
        
        Err(Error::NoMigrationPath {
            from: from.clone(),
            to: to.clone(),
        })
    }
    
    fn find_explicit_path(
        &self,
        from: &Version,
        to: &Version,
    ) -> Result<Vec<(Version, Version)>> {
        // Only use direct migration if it exists
        if self.migrations.contains_key(&(from.clone(), to.clone())) {
            Ok(vec![(from.clone(), to.clone())])
        } else {
            Err(Error::NoDirectMigration {
                from: from.clone(),
                to: to.clone(),
            })
        }
    }
}
````

### Rollback Support

```rust
impl MigrationManager {
    /// Rollback migrations to a previous version
    pub async fn rollback_state(
        &self,
        state: &mut serde_json::Value,
        from_version: &Version,
        to_version: &Version,
    ) -> Result<MigrationReport> {
        if to_version >= from_version {
            return Err(Error::InvalidRollback {
                from: from_version.clone(),
                to: to_version.clone(),
            });
        }
        
        let mut report = MigrationReport::new(from_version.clone(), to_version.clone());
        
        // Find forward path (we'll reverse it)
        let forward_path = self.find_migration_path(to_version, from_version)?;
        
        // Apply rollbacks in reverse order
        for (from, to) in forward_path.iter().rev() {
            let migration = self.migrations
                .get(&(from.clone(), to.clone()))
                .ok_or_else(|| Error::MigrationNotFound {
                    from: from.clone(),
                    to: to.clone(),
                })?;
            
            // Apply rollback
            let result = migration.rollback(state).await?;
            
            report.add_step(MigrationStep {
                from_version: to.clone(),
                to_version: from.clone(),
                migration_id: format!("rollback_{}", migration.migration_id()),
                result,
                duration: Duration::from_secs(0),
            });
        }
        
        report.success = true;
        Ok(report)
    }
}
```

### Automatic Migration on Load

```rust
/// Resource with automatic state migration
pub struct MigratingResource<T: Serialize + for<'a> Deserialize<'a>> {
    state: T,
    version: Version,
    migration_manager: Arc<MigrationManager>,
    target_version: Version,
}

impl<T: Serialize + for<'a> Deserialize<'a>> MigratingResource<T> {
    /// Load resource with automatic migration
    pub async fn load(
        data: Vec<u8>,
        migration_manager: Arc<MigrationManager>,
        target_version: Version,
    ) -> Result<Self> {
        // Deserialize to JSON first to get version
        let mut json_state: serde_json::Value = serde_json::from_slice(&data)?;
        
        // Extract version
        let current_version = json_state
            .get("version")
            .and_then(|v| v.as_str())
            .and_then(|s| Version::parse(s).ok())
            .ok_or_else(|| Error::MissingVersion)?;
        
        // Migrate if needed
        if current_version != target_version {
            migration_manager.migrate_state(
                &mut json_state,
                &current_version,
                &target_version,
            ).await?;
            
            // Update version in state
            json_state["version"] = json!(target_version.to_string());
        }
        
        // Deserialize to target type
        let state: T = serde_json::from_value(json_state)?;
        
        Ok(Self {
            state,
            version: target_version.clone(),
            migration_manager,
            target_version,
        })
    }
    
    /// Save resource with version
    pub async fn save(&self) -> Result<Vec<u8>> {
        let mut json_state = serde_json::to_value(&self.state)?;
        json_state["version"] = json!(self.version.to_string());
        Ok(serde_json::to_vec(&json_state)?)
    }
}
```

### Complex Migration Example

```rust
/// Example: Database schema migration
pub struct DatabaseSchemaMigration {
    from_schema: DatabaseSchema,
    to_schema: DatabaseSchema,
}

#[derive(Debug, Clone)]
pub struct DatabaseSchema {
    version: Version,
    tables: HashMap<String, TableSchema>,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    columns: HashMap<String, ColumnSchema>,
    indexes: Vec<IndexSchema>,
    constraints: Vec<ConstraintSchema>,
}

#[async_trait]
impl StateMigration for DatabaseSchemaMigration {
    fn from_version(&self) -> Version {
        self.from_schema.version.clone()
    }
    
    fn to_version(&self) -> Version {
        self.to_schema.version.clone()
    }
    
    fn migration_id(&self) -> String {
        format!("db_schema_{}_{}", self.from_version(), self.to_version())
    }
    
    async fn migrate(&self, state: &mut serde_json::Value) -> Result<MigrationResult> {
        let mut changes = Vec::new();
        
        // Analyze schema differences
        let diff = self.analyze_schema_diff();
        
        // Apply table changes
        for table_change in diff.table_changes {
            match table_change {
                TableChange::Create(table_name) => {
                    state["tables"][&table_name] = json!({
                        "columns": {},
                        "data": []
                    });
                    changes.push(StateChange::FieldAdded {
                        path: format!("tables.{}", table_name),
                        value: state["tables"][&table_name].clone(),
                    });
                }
                TableChange::Drop(table_name) => {
                    let old_value = state["tables"][&table_name].clone();
                    state["tables"].as_object_mut().unwrap().remove(&table_name);
                    changes.push(StateChange::FieldRemoved {
                        path: format!("tables.{}", table_name),
                        old_value,
                    });
                }
                TableChange::Rename { from, to } => {
                    let value = state["tables"][&from].clone();
                    state["tables"][&to] = value;
                    state["tables"].as_object_mut().unwrap().remove(&from);
                    changes.push(StateChange::FieldRenamed {
                        old_path: format!("tables.{}", from),
                        new_path: format!("tables.{}", to),
                    });
                }
            }
        }
        
        // Apply column changes
        for column_change in diff.column_changes {
            self.apply_column_change(state, &column_change, &mut changes)?;
        }
        
        // Migrate data
        for data_migration in diff.data_migrations {
            self.apply_data_migration(state, &data_migration)?;
        }
        
        Ok(MigrationResult {
            success: true,
            warnings: diff.warnings,
            changes,
        })
    }
    
    async fn rollback(&self, state: &mut serde_json::Value) -> Result<MigrationResult> {
        // Implement reverse migration
        todo!()
    }
    
    async fn validate_pre(&self, state: &serde_json::Value) -> Result<()> {
        // Validate against from_schema
        self.validate_schema(state, &self.from_schema)
    }
    
    async fn validate_post(&self, state: &serde_json::Value) -> Result<()> {
        // Validate against to_schema
        self.validate_schema(state, &self.to_schema)
    }
}
```

### Migration Hooks

```rust
#[async_trait]
pub trait MigrationHook: Send + Sync {
    /// Called before migration starts
    async fn pre_migration(
        &self,
        state: &serde_json::Value,
        from: &Version,
        to: &Version,
    ) -> Result<()>;
    
    /// Called after migration completes
    async fn post_migration(
        &self,
        state: &serde_json::Value,
        from: &Version,
        to: &Version,
    ) -> Result<()>;
    
    /// Called on migration failure
    async fn on_failure(
        &self,
        state: &serde_json::Value,
        error: &Error,
        from: &Version,
        to: &Version,
    );
}

/// Backup hook for safety
pub struct BackupHook {
    storage: Arc<dyn BackupStorage>,
}

#[async_trait]
impl MigrationHook for BackupHook {
    async fn pre_migration(
        &self,
        state: &serde_json::Value,
        from: &Version,
        _to: &Version,
    ) -> Result<()> {
        // Create backup before migration
        let backup_id = format!("backup_{}_{}", from, chrono::Utc::now().timestamp());
        self.storage.store_backup(&backup_id, state).await?;
        Ok(())
    }
    
    async fn post_migration(
        &self,
        _state: &serde_json::Value,
        _from: &Version,
        _to: &Version,
    ) -> Result<()> {
        // Optionally clean up old backups
        Ok(())
    }
    
    async fn on_failure(
        &self,
        _state: &serde_json::Value,
        error: &Error,
        from: &Version,
        to: &Version,
    ) {
        error!("Migration failed from {} to {}: {}", from, to, error);
    }
}
```

## Advanced Features

### Lazy Migration

```rust
/// Lazy migration that migrates on access
pub struct LazyMigratingResource {
    raw_data: Vec<u8>,
    cached_state: Option<serde_json::Value>,
    version: Version,
    target_version: Version,
    migration_manager: Arc<MigrationManager>,
}

impl LazyMigratingResource {
    /// Get state, migrating if necessary
    pub async fn get_state(&mut self) -> Result<&serde_json::Value> {
        if self.cached_state.is_none() {
            // Load and migrate on first access
            let mut state: serde_json::Value = serde_json::from_slice(&self.raw_data)?;
            
            if self.version != self.target_version {
                self.migration_manager.migrate_state(
                    &mut state,
                    &self.version,
                    &self.target_version,
                ).await?;
                
                self.version = self.target_version.clone();
            }
            
            self.cached_state = Some(state);
        }
        
        Ok(self.cached_state.as_ref().unwrap())
    }
}
```

### Parallel Migration

```rust
/// Migrate multiple resources in parallel
pub struct ParallelMigrator {
    migration_manager: Arc<MigrationManager>,
    max_concurrency: usize,
}

impl ParallelMigrator {
    pub async fn migrate_batch(
        &self,
        resources: Vec<(ResourceId, serde_json::Value, Version)>,
        target_version: Version,
    ) -> Result<Vec<(ResourceId, serde_json::Value)>> {
        use futures::stream::{self, StreamExt};
        
        let results = stream::iter(resources)
            .map(|(id, mut state, from_version)| {
                let manager = self.migration_manager.clone();
                let target = target_version.clone();
                
                async move {
                    manager.migrate_state(&mut state, &from_version, &target).await?;
                    Ok((id, state))
                }
            })
            .buffer_unordered(self.max_concurrency)
            .collect::<Vec<Result<_>>>()
            .await;
        
        results.into_iter().collect()
    }
}
```

## Configuration Example

```yaml
migration:
  # Migration strategy
  strategy: shortest_path
  
  # Version compatibility
  compatibility:
    min_version: "1.0.0"
    max_version: "3.0.0"
    current_version: "2.0.0"
  
  # Auto-migration settings
  auto_migrate:
    enabled: true
    on_load: true
    on_save: false
    lazy: false
  
  # Backup configuration
  backup:
    enabled: true
    retention_days: 30
    storage:
      type: s3
      bucket: migrations-backup
      prefix: nebula/
  
  # Migration paths
  migrations:
    - from: "1.0.0"
      to: "1.1.0"
      class: V1_0_to_1_1_Migration
      
    - from: "1.1.0"
      to: "2.0.0"
      class: V1_1_to_2_0_Migration
      
    - from: "2.0.0"
      to: "2.1.0"
      class: V2_0_to_2_1_Migration
      
    - from: "2.1.0"
      to: "3.0.0"
      class: V2_1_to_3_0_Migration
  
  # Validation rules
  validation:
    strict: true
    fail_on_warning: false
    custom_validators:
      - version: "2.0.0"
        class: V2SchemaValidator
      - version: "3.0.0"
        class: V3SchemaValidator
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_simple_migration() {
        let mut manager = MigrationManager::new(MigrationStrategy::Sequential);
        manager.register_migration(V1ToV2Migration).unwrap();
        
        let mut state = json!({
            "user_name": "john_doe",
            "settings": "theme=dark;lang=en"
        });
        
        let report = manager.migrate_state(
            &mut state,
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("2.0.0").unwrap(),
        ).await.unwrap();
        
        assert!(report.success);
        assert_eq!(state["username"], "john_doe");
        assert!(state.get("metadata").is_some());
        assert_eq!(state["settings"]["theme"], "dark");
    }
    
    #[tokio::test]
    async fn test_migration_path_finding() {
        let mut manager = MigrationManager::new(MigrationStrategy::ShortestPath);
        
        // Register multiple paths
        manager.register_migration(MockMigration::new("1.0.0", "1.1.0")).unwrap();
        manager.register_migration(MockMigration::new("1.1.0", "2.0.0")).unwrap();
        manager.register_migration(MockMigration::new("1.0.0", "2.0.0")).unwrap(); // Direct path
        
        let path = manager.find_migration_path(
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("2.0.0").unwrap(),
        ).unwrap();
        
        // Should find direct path
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].1, Version::parse("2.0.0").unwrap());
    }
    
    #[tokio::test]
    async fn test_rollback() {
        let mut manager = MigrationManager::new(MigrationStrategy::Sequential);
        manager.register_migration(V1ToV2Migration).unwrap();
        
        let mut state = json!({
            "username": "john_doe",
            "metadata": {
                "created_at": "2024-01-01",
                "tags": ["test"]
            },
            "settings": {
                "theme": "dark",
                "lang": "en"
            }
        });
        
        let report = manager.rollback_state(
            &mut state,
            &Version::parse("2.0.0").unwrap(),
            &Version::parse("1.0.0").unwrap(),
        ).await.unwrap();
        
        assert!(report.success);
        assert_eq!(state["user_name"], "john_doe");
        assert!(state.get("username").is_none());
        assert!(state.get("metadata").is_none());
    }
    
    #[tokio::test]
    async fn test_lazy_migration() {
        let manager = Arc::new(MigrationManager::new(MigrationStrategy::Sequential));
        manager.register_migration(V1ToV2Migration).unwrap();
        
        let raw_data = serde_json::to_vec(&json!({
            "user_name": "john_doe",
            "settings": "theme=dark"
        })).unwrap();
        
        let mut resource = LazyMigratingResource {
            raw_data,
            cached_state: None,
            version: Version::parse("1.0.0").unwrap(),
            target_version: Version::parse("2.0.0").unwrap(),
            migration_manager: manager,
        };
        
        // Migration happens on first access
        let state = resource.get_state().await.unwrap();
        assert_eq!(state["username"], "john_doe");
        assert!(state.get("metadata").is_some());
    }
}
```

## Best Practices

1. **Always version your state** - Include version field in all stateful resources
2. **Write bidirectional migrations** - Support both upgrade and rollback
3. **Test migration paths** - Ensure all version combinations work
4. **Validate before and after** - Catch issues early
5. **Keep migrations simple** - Complex logic increases failure risk
6. **Document schema changes** - Help future developers understand evolution
7. **Use semantic versioning** - Clear version progression
8. **Backup before migration** - Enable recovery from failures
9. **Monitor migration performance** - Track duration and success rates
10. **Plan for zero-downtime** - Support multiple versions simultaneously; use semver::Version;

/// Versioned state container #[derive(Debug, Clone, Serialize, Deserialize)] pub struct VersionedState<T> { /// Current version of the state pub version: Version,

```
/// The actual state data
pub data: T,

/// Migration metadata
pub metadata: MigrationMetadata,
```

}

#[derive(Debug, Clone, Serialize, Deserialize)] pub struct MigrationMetadata { /// Original version this state was created with pub created_version: Version,

```
/// Last migration applied
pub last_migration: Option<MigrationInfo>,

/// Migration history
pub migration_history: Vec<MigrationInfo>,

/// Checksum for integrity verification
pub checksum: Option<String>,
```

}

#[derive(Debug, Clone, Serialize, Deserialize)] pub struct MigrationInfo { pub from_version: Version, pub to_version: Version, pub migration_id: String, pub applied_at: chrono::DateTime[chrono::Utc](https://claude.ai/chat/a897f31f-2078-4a85-ad93-3e6541209669), pub duration_ms: u64, pub status: MigrationStatus, }

#[derive(Debug, Clone, Serialize, Deserialize)] pub enum MigrationStatus { Success, PartialSuccess { warnings: Vec<String> }, Failed { error: String }, RolledBack { reason: String }, }

````

## Implementation

### Migration System

```rust
use nebula_resource::prelude::*;
use async_trait::async_trait;
use std::collections::BTreeMap;

/// Trait for state migration
#[async_trait]
pub trait StateMigration: Send + Sync {
    /// Source version this migration applies from
    fn from_version(&self) -> Version;
    
    /// Target version this migration produces
    fn to_version(&self) -> Version;
    
    /// Unique identifier for this migration
    fn migration_id(&self) -> String;
    
    /// Apply forward migration
    async fn migrate(&self, state: &mut serde_json::Value) -> Result<MigrationResult>;
    
    /// Apply backward migration (rollback)
    async fn rollback(&self, state: &mut serde_json::Value) -> Result<MigrationResult>;
    
    /// Validate state before migration
    async fn validate_pre(&self, state: &serde_json::Value) -> Result<()>;
    
    /// Validate state after migration
    async fn validate_post(&self, state: &serde_json::Value) -> Result<()> {
        // Validate v2.0.0 schema
        if !state.get("username").is_some() {
            return Err(Error::ValidationFailed("Missing required field: username".into()));
        }
        if !state.get("metadata").is_some() {
            return Err(Error::ValidationFailed("Missing required field: metadata".into()));
        }
        Ok(())
    }
    
    fn parse_legacy_settings(&self, settings: &str) -> Result<serde_json::Value> {
        // Parse legacy format: "key1=value1;key2=value2"
        let mut parsed = serde_json::Map::new();
        for pair in settings.split(';') {
            if let Some((key, value)) = pair.split_once('=') {
                parsed.insert(key.to_string(), json!(value));
            }
        }
        Ok(json!(parsed))
    }
} &serde_json::Value) -> Result<()>;
}

pub struct MigrationResult {
    pub success: bool,
    pub warnings: Vec<String>,
    pub changes: Vec<StateChange>,
}

#[derive(Debug, Clone)]
pub enum StateChange {
    FieldAdded { path: String, value: serde_json::Value },
    FieldRemoved { path: String, old_value: serde_json::Value },
    FieldRenamed { old_path: String, new_path: String },
    FieldModified { path: String, old_value: serde_json::Value, new_value: serde_json::Value },
    StructureChanged { description: String },
}

/// Migration manager for handling version upgrades
pub struct MigrationManager {
    /// All registered migrations
    migrations: BTreeMap<(Version, Version), Box<dyn StateMigration>>,
    
    /// Migration strategies
    strategy: MigrationStrategy,
    
    /// State validators
    validators: HashMap<Version, Box<dyn StateValidator>>,
    
    /// Migration hooks
    hooks: Vec<Box<dyn MigrationHook>>,
}

#[derive(Debug, Clone)]
pub enum MigrationStrategy {
    /// Apply migrations one by one
    Sequential,
    
    /// Find shortest migration path
    ShortestPath,
    
    /// Use explicit migration paths only
    Explicit,
    
    /// Custom strategy
    Custom(Arc<dyn MigrationPathFinder>),
}
````

### Basic Migration Example

```rust
/// Example: Migrating from v1.0.0 to v2.0.0
pub struct V1ToV2Migration;

#[async_trait]
impl StateMigration for V1ToV2Migration {
    fn from_version(&self) -> Version {
        Version::parse("1.0.0").unwrap()
    }
    
    fn to_version(&self) -> Version {
        Version::parse("2.0.0").unwrap()
    }
    
    fn migration_id(&self) -> String {
        "v1_to_v2_add_metadata".to_string()
    }
    
    async fn migrate(&self, state: &mut serde_json::Value) -> Result<MigrationResult> {
        let mut changes = Vec::new();
        
        // Add new metadata field
        if !state.get("metadata").is_some() {
            state["metadata"] = json!({
                "created_at": chrono::Utc::now(),
                "tags": [],
                "version": "2.0.0"
            });
            
            changes.push(StateChange::FieldAdded {
                path: "metadata".to_string(),
                value: state["metadata"].clone(),
            });
        }
        
        // Rename old field
        if let Some(old_value) = state.get("user_name") {
            state["username"] = old_value.clone();
            state.as_object_mut().unwrap().remove("user_name");
            
            changes.push(StateChange::FieldRenamed {
                old_path: "user_name".to_string(),
                new_path: "username".to_string(),
            });
        }
        
        // Transform data structure
        if let Some(settings) = state.get("settings").and_then(|s| s.as_str()) {
            // Parse old string format into structured object
            let parsed_settings = self.parse_legacy_settings(settings)?;
            state["settings"] = parsed_settings;
            
            changes.push(StateChange::StructureChanged {
                description: "Converted settings from string to object".to_string(),
            });
        }
        
        Ok(MigrationResult {
            success: true,
            warnings: vec![],
            changes,
        })
    }
    
    async fn rollback(&self, state: &mut serde_json::Value) -> Result<MigrationResult> {
        let mut changes = Vec::new();
        
        // Remove metadata field
        if state.get("metadata").is_some() {
            let old_value = state["metadata"].clone();
            state.as_object_mut().unwrap().remove("metadata");
            
            changes.push(StateChange::FieldRemoved {
                path: "metadata".to_string(),
                old_value,
            });
        }
        
        // Rename field back
        if let Some(username) = state.get("username") {
            state["user_name"] = username.clone();
            state.as_object_mut().unwrap().remove("username");
            
            changes.push(StateChange::FieldRenamed {
                old_path: "username".to_string(),
                new_path: "user_name".to_string(),
            });
        }
        
        Ok(MigrationResult {
            success: true,
            warnings: vec![],
            changes,
        })
    }
    
    async fn validate_pre(&self, state: &serde_json::Value) -> Result<()> {
        // Validate v1.0.0 schema
        if !state.get("user_name").is_some() {
            return Err(Error::ValidationFailed("Missing required field: user_name".into()));
        }
        Ok(())
    }
    
    async fn validate_post(&self, state:
```