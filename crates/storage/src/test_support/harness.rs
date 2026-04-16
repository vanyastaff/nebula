//! Test database harness.
//!
//! Creates ephemeral in-memory SQLite configurations for isolated tests.

use crate::pool::PoolConfig;

/// Configuration for an ephemeral test database (SQLite in-memory).
///
/// Each call creates a fresh `PoolConfig` pointing to `:memory:`,
/// ensuring complete test isolation.
pub fn test_pool_config() -> PoolConfig {
    PoolConfig::sqlite_memory()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_sqlite_memory_config() {
        let config = test_pool_config();
        assert_eq!(config.url, "sqlite::memory:");
        assert!(config.run_migrations);
    }
}
