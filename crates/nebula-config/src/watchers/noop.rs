//! No-operation watcher that does nothing

use crate::core::{ConfigResult, ConfigSource, ConfigWatcher};
use async_trait::async_trait;

/// No-op watcher that does nothing
#[derive(Debug, Clone, Default)]
pub struct NoOpWatcher;

impl NoOpWatcher {
    /// Create a new no-op watcher
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ConfigWatcher for NoOpWatcher {
    async fn start_watching(&self, _sources: &[ConfigSource]) -> ConfigResult<()> {
        nebula_log::debug!("NoOpWatcher: start_watching called (no-op)");
        Ok(())
    }

    async fn stop_watching(&self) -> ConfigResult<()> {
        nebula_log::debug!("NoOpWatcher: stop_watching called (no-op)");
        Ok(())
    }

    fn is_watching(&self) -> bool {
        false
    }
}
