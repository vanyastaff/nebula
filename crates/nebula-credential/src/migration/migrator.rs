use anyhow::Result;
use dashmap::DashMap;
use serde_json::Value;

/// State migrator trait
pub trait StateMigrator: Send + Sync {
    /// Credential kind
    fn kind(&self) -> &'static str;

    /// Source version
    #[allow(clippy::wrong_self_convention)]
    fn from_version(&self) -> u16;

    /// Target version
    fn to_version(&self) -> u16;

    /// Perform migration
    fn migrate(&self, state: Value) -> Result<Value>;
}

/// Migration registry
pub struct MigrationRegistry {
    migrators: DashMap<(String, u16, u16), Box<dyn StateMigrator>>,
}

impl MigrationRegistry {
    /// Create new registry
    pub fn new() -> Self {
        Self {
            migrators: DashMap::new(),
        }
    }

    /// Register a migrator
    pub fn register(&self, migrator: Box<dyn StateMigrator>) {
        let key = (
            migrator.kind().to_string(),
            migrator.from_version(),
            migrator.to_version(),
        );
        self.migrators.insert(key, migrator);
    }

    /// Migrate state from version to version
    pub fn migrate(
        &self,
        kind: &str,
        mut state: Value,
        from_version: u16,
        to_version: u16,
    ) -> Result<Value> {
        let mut current_version = from_version;

        while current_version < to_version {
            let next_version = current_version + 1;
            let key = (kind.to_string(), current_version, next_version);

            let migrator = self.migrators.get(&key).ok_or_else(|| {
                anyhow::anyhow!(
                    "No migration from {} v{} to v{}",
                    kind,
                    current_version,
                    next_version
                )
            })?;

            state = migrator.migrate(state)?;
            current_version = next_version;
        }

        Ok(state)
    }

    /// Check if migration path exists
    pub fn has_migration_path(&self, kind: &str, from: u16, to: u16) -> bool {
        let mut current = from;
        while current < to {
            let next = current + 1;
            if !self
                .migrators
                .contains_key(&(kind.to_string(), current, next))
            {
                return false;
            }
            current = next;
        }
        true
    }
}

impl Default for MigrationRegistry {
    fn default() -> Self {
        Self::new()
    }
}
