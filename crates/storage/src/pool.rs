//! Database connection pool — dual-backend (Postgres + SQLite).
//!
//! # Design
//!
//! Two explicit backend variants instead of `sqlx::AnyPool`:
//! - Preserves compile-time SQL checking per dialect
//! - Each backend has its own migration directory
//! - Runtime backend is selected by URL scheme (`postgres://` vs `sqlite://`)

use crate::error::StorageError;

/// Supported storage backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// PostgreSQL — production multi-process.
    Postgres,
    /// SQLite — local-first, dev, tests.
    Sqlite,
}

impl Backend {
    /// Detect backend from a database URL.
    ///
    /// ```text
    /// postgres://...  → Postgres
    /// sqlite://...    → Sqlite
    /// sqlite::memory: → Sqlite
    /// :memory:        → Sqlite
    /// file:...        → Sqlite
    /// ```
    pub fn from_url(url: &str) -> Result<Self, StorageError> {
        let lower = url.to_lowercase();
        if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
            Ok(Self::Postgres)
        } else if lower.starts_with("sqlite:") || lower.starts_with("file:") || lower == ":memory:"
        {
            Ok(Self::Sqlite)
        } else {
            Err(StorageError::Configuration(format!(
                "cannot detect backend from URL: {url}"
            )))
        }
    }
}

/// Database connection pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Database URL (determines backend automatically).
    pub url: String,
    /// Maximum number of connections (default: 10 for PG, 1 for SQLite).
    pub max_connections: Option<u32>,
    /// Minimum number of idle connections (default: 1).
    pub min_connections: Option<u32>,
    /// Whether to run migrations on connect (default: true).
    pub run_migrations: bool,
}

impl PoolConfig {
    /// Create config from a URL with defaults.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: None,
            min_connections: None,
            run_migrations: true,
        }
    }

    /// SQLite in-memory for tests.
    pub fn sqlite_memory() -> Self {
        Self {
            url: "sqlite::memory:".into(),
            max_connections: Some(1),
            min_connections: Some(1),
            run_migrations: true,
        }
    }

    /// Set max connections.
    pub fn with_max_connections(mut self, n: u32) -> Self {
        self.max_connections = Some(n);
        self
    }

    /// Disable auto-migration on connect.
    pub fn without_migrations(mut self) -> Self {
        self.run_migrations = false;
        self
    }

    /// Detected backend.
    pub fn backend(&self) -> Result<Backend, StorageError> {
        Backend::from_url(&self.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_postgres() {
        assert_eq!(
            Backend::from_url("postgres://localhost/nebula").unwrap(),
            Backend::Postgres
        );
        assert_eq!(
            Backend::from_url("postgresql://localhost/nebula").unwrap(),
            Backend::Postgres
        );
    }

    #[test]
    fn detect_sqlite() {
        assert_eq!(
            Backend::from_url("sqlite::memory:").unwrap(),
            Backend::Sqlite
        );
        assert_eq!(
            Backend::from_url("sqlite:nebula.db").unwrap(),
            Backend::Sqlite
        );
        assert_eq!(Backend::from_url(":memory:").unwrap(), Backend::Sqlite);
        assert_eq!(
            Backend::from_url("file:nebula.db").unwrap(),
            Backend::Sqlite
        );
    }

    #[test]
    fn unknown_scheme_errors() {
        assert!(Backend::from_url("mysql://localhost").is_err());
    }
}
