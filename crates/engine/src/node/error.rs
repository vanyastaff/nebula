use thiserror::Error;

use crate::ParameterError;
use crate::types::{Key, KeyParseError};

/// Error types for node operations
#[derive(Debug, Error)]
pub enum NodeError {
    /// Node identified by `key` was not found.
    #[error("Node with key '{0}' is not found")]
    NotFound(Key),

    /// A specified version was not found for the node.
    #[error("Version {0} not found for the node with the key '{1}'")]
    VersionNotFound(u32, Key),

    /// Invalid version format or content.
    #[error("Invalid version format: {0}")]
    InvalidVersion(String),

    /// Operation that requires a versioned node was attempted on a single node.
    #[error("Node with key '{0}' is not versioned")]
    NotVersioned(Key),

    /// Node with the specified key already exists in the registry.
    #[error("Node with key '{0}' already exists in the registry")]
    AlreadyExists(Key),

    /// No versions are available for the node.
    #[error("No versions are available for the node with the key '{0}'")]
    NoVersionsAvailable(Key),

    /// The key of the node doesn't match the key of NodeVersions.
    #[error("Key mismatch: node has key '{0}', but a collection has key '{1}'")]
    KeyMismatch(Key, Key),

    /// Error occurred during node loading.
    #[error("Failed to load node '{0}': {1}")]
    LoadError(String, String),

    /// Version already exists in the collection.
    #[error("Version {version} already exists for a node with the key '{key}'")]
    VersionAlreadyExists { version: u32, key: Key },

    /// Node validation failed.
    #[error("Validation failed for node '{key}': {reason}")]
    ValidationError { key: Key, reason: String },

    /// Node execution error.
    #[error("Execution error for node '{key}': {reason}")]
    ExecutionError { key: Key, reason: String },

    /// Node configuration error.
    #[error("Invalid configuration for node '{key}': {reason}")]
    InvalidConfiguration { key: Key, reason: String },

    /// Credential error for node.
    #[error("Credential error for node '{key}': {reason}")]
    CredentialError { key: Key, reason: String },

    /// Node parameters error.
    #[error("Parameter error for node '{key}': {error}")]
    ParameterError { key: Key, error: ParameterError },

    /// Missing required field in node.
    #[error("Missing required field '{field}' for a node with the key '{key}'")]
    MissingRequiredField { key: Key, field: String },

    /// Invalid format or content for a parameter key string.
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(#[from] KeyParseError),

    /// Build error during node construction.
    #[error("Build error for node: {0}")]
    BuildError(#[from] derive_builder::UninitializedFieldError),

    /// Internal error during node operation.
    #[error("Internal node error: {0}")]
    InternalError(String),

    /// I/O error encountered during node operation.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

impl PartialEq for NodeError {
    fn eq(&self, other: &Self) -> bool {
        use NodeError::*;
        match (self, other) {
            (NotFound(a), NotFound(b)) => a == b,

            (VersionNotFound(v1, k1), VersionNotFound(v2, k2)) => v1 == v2 && k1 == k2,

            (InvalidVersion(a), InvalidVersion(b)) => a == b,

            (NotVersioned(a), NotVersioned(b)) => a == b,

            (AlreadyExists(a), AlreadyExists(b)) => a == b,

            (NoVersionsAvailable(a), NoVersionsAvailable(b)) => a == b,

            (KeyMismatch(k1, k2), KeyMismatch(k3, k4)) => k1 == k3 && k2 == k4,

            (
                VersionAlreadyExists {
                    version: v1,
                    key: k1,
                },
                VersionAlreadyExists {
                    version: v2,
                    key: k2,
                },
            ) => v1 == v2 && k1 == k2,

            (
                ValidationError {
                    key: k1,
                    reason: r1,
                },
                ValidationError {
                    key: k2,
                    reason: r2,
                },
            ) => k1 == k2 && r1 == r2,

            (
                ExecutionError {
                    key: k1,
                    reason: r1,
                },
                ExecutionError {
                    key: k2,
                    reason: r2,
                },
            ) => k1 == k2 && r1 == r2,

            (
                InvalidConfiguration {
                    key: k1,
                    reason: r1,
                },
                InvalidConfiguration {
                    key: k2,
                    reason: r2,
                },
            ) => k1 == k2 && r1 == r2,

            (
                CredentialError {
                    key: k1,
                    reason: r1,
                },
                CredentialError {
                    key: k2,
                    reason: r2,
                },
            ) => k1 == k2 && r1 == r2,

            (ParameterError { key: k1, error: e1 }, ParameterError { key: k2, error: e2 }) => {
                k1 == k2 && e1 == e2
            }

            (
                MissingRequiredField { key: k1, field: f1 },
                MissingRequiredField { key: k2, field: f2 },
            ) => k1 == k2 && f1 == f2,

            (InvalidKeyFormat(e1), InvalidKeyFormat(e2)) => e1 == e2,

            (BuildError(e1), BuildError(e2)) => e1.field_name() == e2.field_name(),

            (InternalError(a), InternalError(b)) => a == b,

            (IoError(e1), IoError(e2)) => {
                e1.kind() == e2.kind() && e1.to_string() == e2.to_string()
            }

            _ => false,
        }
    }
}

impl Eq for NodeError {}
