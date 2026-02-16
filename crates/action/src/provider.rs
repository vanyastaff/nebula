//! Dependency-injection port traits for actions.
//!
//! These traits decouple actions from concrete runtime services (credential
//! storage, logging, metrics) so actions can be tested and executed in
//! different environments without modification.

use std::fmt;

use async_trait::async_trait;

use crate::error::ActionError;

/// A string that redacts its contents in Debug and Display.
///
/// Used for credential values to prevent accidental logging.
#[derive(Clone)]
pub struct SecureString {
    inner: String,
}

impl SecureString {
    /// Create a new secure string.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    /// Access the underlying value.
    pub fn expose(&self) -> &str {
        &self.inner
    }
}

impl fmt::Debug for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecureString(***)")
    }
}

impl fmt::Display for SecureString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Port trait for providing credentials to actions.
///
/// Implemented by the runtime to inject credential resolution into actions
/// without coupling them to the credential storage backend.
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Retrieve a credential value by key.
    async fn get(&self, key: &str) -> Result<SecureString, ActionError>;
}

/// Port trait for action-level logging.
///
/// Actions use this to emit structured log messages that are captured
/// by the runtime's logging infrastructure.
pub trait ActionLogger: Send + Sync {
    /// Log a debug message.
    fn debug(&self, message: &str);
    /// Log an info message.
    fn info(&self, message: &str);
    /// Log a warning.
    fn warn(&self, message: &str);
    /// Log an error.
    fn error(&self, message: &str);
}

/// Port trait for action-level metrics.
///
/// Actions use this to emit custom metrics (counters, histograms)
/// that are collected by the runtime's metrics infrastructure.
pub trait ActionMetrics: Send + Sync {
    /// Increment a counter by 1.
    fn counter(&self, name: &str, value: u64);
    /// Record a histogram observation.
    fn histogram(&self, name: &str, value: f64);
}

/// Port trait for providing resources to actions.
///
/// Implemented by the runtime to inject resource access (database connections,
/// HTTP clients, caches, etc.) into actions without coupling them to the
/// resource management backend.
///
/// Resources are identified by a string key (matching `Resource::id()` in
/// `nebula-resource`). The returned value is type-erased — the action is
/// responsible for downcasting to the expected instance type.
#[async_trait]
pub trait ResourceProvider: Send + Sync {
    /// Acquire a resource instance by key.
    ///
    /// The returned `Box<dyn Any + Send>` should be downcast to the concrete
    /// resource instance type. The resource is released when the box is dropped.
    async fn acquire(&self, key: &str) -> Result<Box<dyn std::any::Any + Send>, ActionError>;
}
