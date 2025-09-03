//! Error types for memory management operations

use std::fmt;

/// Result type for memory operations
pub type Result<T> = std::result::Result<T, MemoryError>;

/// Memory operation errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    /// Out of memory
    OutOfMemory { requested: usize, available: Option<usize> },

    /// Invalid alignment
    InvalidAlignment { value: usize, required: usize },

    /// Invalid size
    InvalidSize { size: usize, reason: String },

    /// Pool exhausted
    PoolExhausted { pool_name: String, capacity: usize },

    /// Arena full
    ArenaFull { arena_size: usize, requested: usize },

    /// Cache miss
    CacheMiss { key: String },

    /// Budget exceeded
    BudgetExceeded { limit: usize, attempted: usize },

    /// Configuration error
    ConfigError { message: String },

    /// System error
    SystemError { message: String },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory { requested, available } => {
                if let Some(available) = available {
                    write!(
                        f,
                        "Out of memory: requested {} bytes, {} available",
                        requested, available
                    )
                } else {
                    write!(f, "Out of memory: requested {} bytes", requested)
                }
            },
            Self::InvalidAlignment { value, required } => {
                write!(f, "Invalid alignment: {} is not aligned to {}", value, required)
            },
            Self::InvalidSize { size, reason } => {
                write!(f, "Invalid size {}: {}", size, reason)
            },
            Self::PoolExhausted { pool_name, capacity } => {
                write!(f, "Pool '{}' exhausted (capacity: {})", pool_name, capacity)
            },
            Self::ArenaFull { arena_size, requested } => {
                write!(f, "Arena full: size {}, requested {}", arena_size, requested)
            },
            Self::CacheMiss { key } => {
                write!(f, "Cache miss for key '{}'", key)
            },
            Self::BudgetExceeded { limit, attempted } => {
                write!(f, "Memory budget exceeded: limit {}, attempted {}", limit, attempted)
            },
            Self::ConfigError { message } => {
                write!(f, "Configuration error: {message}")
            },
            Self::SystemError { message } => {
                write!(f, "System error: {message}")
            },
        }
    }
}

impl std::error::Error for MemoryError {}

impl MemoryError {
    /// Create an out of memory error
    pub fn out_of_memory(requested: usize) -> Self {
        Self::OutOfMemory { requested, available: None }
    }

    /// Create an out of memory error with available memory info
    pub fn out_of_memory_with_available(requested: usize, available: usize) -> Self {
        Self::OutOfMemory { requested, available: Some(available) }
    }

    /// Create an invalid alignment error
    pub fn invalid_alignment(value: usize, required: usize) -> Self {
        Self::InvalidAlignment { value, required }
    }

    /// Create an invalid size error
    pub fn invalid_size(size: usize, reason: impl Into<String>) -> Self {
        Self::InvalidSize { size, reason: reason.into() }
    }

    /// Create a pool exhausted error
    pub fn pool_exhausted(pool_name: impl Into<String>, capacity: usize) -> Self {
        Self::PoolExhausted { pool_name: pool_name.into(), capacity }
    }

    /// Create an arena full error
    pub fn arena_full(arena_size: usize, requested: usize) -> Self {
        Self::ArenaFull { arena_size, requested }
    }

    /// Create a cache miss error
    pub fn cache_miss(key: impl Into<String>) -> Self {
        Self::CacheMiss { key: key.into() }
    }

    /// Create a budget exceeded error
    pub fn budget_exceeded(limit: usize, attempted: usize) -> Self {
        Self::BudgetExceeded { limit, attempted }
    }

    /// Create a configuration error
    pub fn config_error(message: impl Into<String>) -> Self {
        Self::ConfigError { message: message.into() }
    }

    /// Create a system error
    pub fn system_error(message: impl Into<String>) -> Self {
        Self::SystemError { message: message.into() }
    }
}

impl From<nebula_system::SystemError> for MemoryError {
    fn from(err: nebula_system::SystemError) -> Self {
        MemoryError::SystemError { message: err.to_string() }
    }
}
