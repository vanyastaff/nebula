//! Closed backend identity for bounded persistence telemetry.

/// Owning persistence adapter, never a connection string or deployment identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageBackendKind {
    /// Internal reference/conformance adapter.
    InMemory,
    /// SQLite deployment adapter.
    Sqlite,
    /// PostgreSQL deployment adapter.
    Postgres,
}

impl StorageBackendKind {
    /// Closed metric/span label; tenant and execution identities are excluded.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InMemory => "in_memory",
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}
