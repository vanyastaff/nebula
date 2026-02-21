//! Persistent state storage trait

use crate::{Result, TriggerState};
use async_trait::async_trait;

/// Trait for persisting trigger state across restarts
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Load trigger state by ID
    async fn load(&self, trigger_id: &str) -> Result<Option<TriggerState>>;

    /// Save trigger state
    async fn save(&self, state: &TriggerState) -> Result<()>;

    /// Delete trigger state
    async fn delete(&self, trigger_id: &str) -> Result<()>;
}

/// In-memory state store (for testing)
pub struct MemoryStateStore {
    states: std::sync::Arc<dashmap::DashMap<String, TriggerState>>,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self {
            states: std::sync::Arc::new(dashmap::DashMap::new()),
        }
    }
}

#[async_trait]
impl StateStore for MemoryStateStore {
    async fn load(&self, trigger_id: &str) -> Result<Option<TriggerState>> {
        Ok(self.states.get(trigger_id).map(|r| r.clone()))
    }

    async fn save(&self, state: &TriggerState) -> Result<()> {
        self.states.insert(state.trigger_id.clone(), state.clone());
        Ok(())
    }

    async fn delete(&self, trigger_id: &str) -> Result<()> {
        self.states.remove(trigger_id);
        Ok(())
    }
}

// TODO: Implement for Redis, PostgreSQL, etcd, etc.
// pub struct RedisStateStore { ... }
// pub struct PostgresStateStore { ... }
