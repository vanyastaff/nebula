//! System error types
//!
//! These errors are typically caused by infrastructure issues, network problems,
//! resource constraints, or other system-level problems. Many are retryable.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::core::traits::{ErrorCode, RetryableError};

/// System-level error variants
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum SystemError {
    /// Operation timed out
    #[error("Operation timed out: {operation} after {duration:?}")]
    Timeout {
        operation: String,
        duration: Duration,
    },

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {limit} requests per {period:?}")]
    RateLimitExceeded { limit: u32, period: Duration },

    /// Resource exhausted
    #[error("Resource exhausted: {resource}")]
    ResourceExhausted { resource: String },

    /// Network error
    #[error("Network error: {message}")]
    Network { message: String },

    /// Database error
    #[error("Database error: {message}")]
    Database { message: String },

    /// External service error
    #[error("External service error: {service} - {message}")]
    ExternalService { service: String, message: String },

    /// File system error
    #[error("File system error: {operation} - {message}")]
    FileSystem { operation: String, message: String },

    /// Memory error
    #[error("Memory error: {message}")]
    Memory { message: String },

    /// Disk space error
    #[error("Disk space error: {message}")]
    DiskSpace { message: String },

    /// Connection error
    #[error("Connection error: {target} - {reason}")]
    Connection { target: String, reason: String },

    /// SSL/TLS error
    #[error("SSL/TLS error: {message}")]
    Ssl { message: String },

    /// DNS resolution error
    #[error("DNS resolution error: {hostname} - {reason}")]
    DnsResolution { hostname: String, reason: String },
}

impl RetryableError for SystemError {
    fn is_retryable(&self) -> bool {
        match self {
            SystemError::Timeout { .. } => true,
            SystemError::RateLimitExceeded { .. } => true,
            SystemError::ResourceExhausted { .. } => true, // Resources might become available
            SystemError::Network { .. } => true,
            SystemError::Database { .. } => true, // Database might recover
            SystemError::ExternalService { .. } => true,
            SystemError::FileSystem { .. } => false, // File system errors usually need manual intervention
            SystemError::Memory { .. } => false,     // Memory errors are usually fatal
            SystemError::DiskSpace { .. } => false,  // Disk space needs manual cleanup
            SystemError::Connection { .. } => true,
            SystemError::Ssl { .. } => false, // SSL errors usually indicate configuration issues
            SystemError::DnsResolution { .. } => true, // DNS might recover
        }
    }

    fn retry_delay(&self) -> Option<Duration> {
        match self {
            SystemError::Timeout { duration, .. } => Some(*duration / 2), // Retry with half the timeout
            SystemError::RateLimitExceeded { period, .. } => Some(*period / 4), // Wait quarter of the period
            SystemError::ResourceExhausted { .. } => Some(Duration::from_secs(5)),
            SystemError::Network { .. } => Some(Duration::from_secs(2)),
            SystemError::Database { .. } => Some(Duration::from_secs(3)),
            SystemError::ExternalService { .. } => Some(Duration::from_secs(5)),
            SystemError::Connection { .. } => Some(Duration::from_secs(2)),
            SystemError::DnsResolution { .. } => Some(Duration::from_secs(1)),
            _ => None,
        }
    }
}

impl ErrorCode for SystemError {
    fn error_code(&self) -> &str {
        match self {
            SystemError::Timeout { .. } => "TIMEOUT_ERROR",
            SystemError::RateLimitExceeded { .. } => "RATE_LIMIT_ERROR",
            SystemError::ResourceExhausted { .. } => "RESOURCE_EXHAUSTED_ERROR",
            SystemError::Network { .. } => "NETWORK_ERROR",
            SystemError::Database { .. } => "DATABASE_ERROR",
            SystemError::ExternalService { .. } => "EXTERNAL_SERVICE_ERROR",
            SystemError::FileSystem { .. } => "FILE_SYSTEM_ERROR",
            SystemError::Memory { .. } => "MEMORY_ERROR",
            SystemError::DiskSpace { .. } => "DISK_SPACE_ERROR",
            SystemError::Connection { .. } => "CONNECTION_ERROR",
            SystemError::Ssl { .. } => "SSL_ERROR",
            SystemError::DnsResolution { .. } => "DNS_RESOLUTION_ERROR",
        }
    }

    fn error_category(&self) -> &str {
        "SYSTEM"
    }
}

impl SystemError {
    /// Create a timeout error
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> Self {
        Self::Timeout {
            operation: operation.into(),
            duration,
        }
    }

    /// Create a rate limit exceeded error
    pub fn rate_limit_exceeded(limit: u32, period: Duration) -> Self {
        Self::RateLimitExceeded { limit, period }
    }

    /// Create a resource exhausted error
    pub fn resource_exhausted(resource: impl Into<String>) -> Self {
        Self::ResourceExhausted {
            resource: resource.into(),
        }
    }

    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::Network {
            message: message.into(),
        }
    }

    /// Create a database error
    pub fn database(message: impl Into<String>) -> Self {
        Self::Database {
            message: message.into(),
        }
    }

    /// Create an external service error
    pub fn external_service(service: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ExternalService {
            service: service.into(),
            message: message.into(),
        }
    }

    /// Create a file system error
    pub fn file_system(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self::FileSystem {
            operation: operation.into(),
            message: message.into(),
        }
    }

    /// Create a memory error
    pub fn memory(message: impl Into<String>) -> Self {
        Self::Memory {
            message: message.into(),
        }
    }

    /// Create a disk space error
    pub fn disk_space(message: impl Into<String>) -> Self {
        Self::DiskSpace {
            message: message.into(),
        }
    }

    /// Create a connection error
    pub fn connection(target: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Connection {
            target: target.into(),
            reason: reason.into(),
        }
    }

    /// Create an SSL error
    pub fn ssl(message: impl Into<String>) -> Self {
        Self::Ssl {
            message: message.into(),
        }
    }

    /// Create a DNS resolution error
    pub fn dns_resolution(hostname: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::DnsResolution {
            hostname: hostname.into(),
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_error_creation() {
        let timeout_error = SystemError::timeout("API call", Duration::from_secs(30));
        assert_eq!(timeout_error.error_code(), "TIMEOUT_ERROR");
        assert!(timeout_error.is_retryable());

        let rate_limit_error = SystemError::rate_limit_exceeded(100, Duration::from_secs(60));
        assert_eq!(rate_limit_error.error_code(), "RATE_LIMIT_ERROR");
        assert!(rate_limit_error.is_retryable());

        let memory_error = SystemError::memory("Out of memory");
        assert_eq!(memory_error.error_code(), "MEMORY_ERROR");
        assert!(!memory_error.is_retryable());
    }

    #[test]
    fn test_system_error_display() {
        let timeout_error = SystemError::timeout("database query", Duration::from_secs(30));
        assert_eq!(
            timeout_error.to_string(),
            "Operation timed out: database query after 30s"
        );

        let network_error = SystemError::network("Connection refused");
        assert_eq!(
            network_error.to_string(),
            "Network error: Connection refused"
        );

        let dns_error = SystemError::dns_resolution("example.com", "NXDOMAIN");
        assert_eq!(
            dns_error.to_string(),
            "DNS resolution error: example.com - NXDOMAIN"
        );
    }

    #[test]
    fn test_retry_behavior() {
        let timeout_error = SystemError::timeout("API call", Duration::from_secs(30));
        assert!(timeout_error.is_retryable());
        assert_eq!(timeout_error.retry_delay(), Some(Duration::from_secs(15)));

        let memory_error = SystemError::memory("Out of memory");
        assert!(!memory_error.is_retryable());
        assert_eq!(memory_error.retry_delay(), None);

        let network_error = SystemError::network("Connection reset");
        assert!(network_error.is_retryable());
        assert_eq!(network_error.retry_delay(), Some(Duration::from_secs(2)));
    }

    #[test]
    fn test_rate_limit_retry_delay() {
        let rate_limit_error = SystemError::rate_limit_exceeded(100, Duration::from_secs(60));
        assert!(rate_limit_error.is_retryable());
        assert_eq!(
            rate_limit_error.retry_delay(),
            Some(Duration::from_secs(15))
        ); // Quarter of period
    }
}
