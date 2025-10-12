//! Configuration error types

use nebula_error::NebulaError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Configuration error type
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ConfigError {
    /// Configuration file not found
    #[error("Configuration file not found: {path}")]
    FileNotFound {
        /// Path to the configuration file
        path: PathBuf,
    },

    /// Configuration file read error
    #[error("Failed to read configuration file {path}: {message}")]
    FileReadError {
        /// Path to the configuration file
        path: PathBuf,
        /// Error message
        message: String,
    },

    /// Configuration file parse error
    #[error("Failed to parse configuration file {path}: {message}")]
    ParseError {
        /// Path to the configuration file
        path: PathBuf,
        /// Error message describing the parse failure
        message: String,
    },

    /// Configuration validation error
    #[error("Configuration validation failed: {message}")]
    ValidationError {
        /// Error message describing the validation failure
        message: String,
        /// Optional field name that failed validation
        field: Option<String>,
    },

    /// Configuration source error
    #[error("Configuration source error: {message}")]
    SourceError {
        /// Error message describing the source error
        message: String,
        /// Origin of the configuration source
        origin: String,
    },

    /// Environment variable not found
    #[error("Environment variable not found: {name}")]
    EnvVarNotFound {
        /// Name of the environment variable
        name: String,
    },

    /// Environment variable parse error
    #[error("Failed to parse environment variable {name}: {value}")]
    EnvVarParseError {
        /// Name of the environment variable
        name: String,
        /// Value that failed to parse
        value: String,
    },

    /// Configuration reload error
    #[error("Failed to reload configuration: {message}")]
    ReloadError {
        /// Error message describing the reload failure
        message: String,
    },

    /// Configuration watch error
    #[error("Configuration watch error: {message}")]
    WatchError {
        /// Error message describing the watch failure
        message: String,
    },

    /// Configuration merge error
    #[error("Failed to merge configurations: {message}")]
    MergeError {
        /// Error message describing the merge failure
        message: String,
    },

    /// Configuration type error
    #[error("Configuration type error: {message}")]
    TypeError {
        /// Error message describing the type mismatch
        message: String,
        /// Expected type
        expected: String,
        /// Actual type encountered
        actual: String,
    },

    /// Configuration path error
    #[error("Configuration path error: {message}")]
    PathError {
        /// Error message describing the path issue
        message: String,
        /// Path that caused the error
        path: String,
    },

    /// Configuration format not supported
    #[error("Configuration format not supported: {format}")]
    FormatNotSupported {
        /// Format that is not supported
        format: String,
    },

    /// Configuration encryption error
    #[error("Configuration encryption error: {message}")]
    EncryptionError {
        /// Error message describing the encryption failure
        message: String,
    },

    /// Configuration decryption error
    #[error("Configuration decryption error: {message}")]
    DecryptionError {
        /// Error message describing the decryption failure
        message: String,
    },
}

impl ConfigError {
    /// Create a file not found error
    pub fn file_not_found(path: impl Into<PathBuf>) -> Self {
        Self::FileNotFound { path: path.into() }
    }

    /// Create a file read error
    pub fn file_read_error(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::FileReadError {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create a parse error
    pub fn parse_error(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::ParseError {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create a validation error
    pub fn validation_error(message: impl Into<String>, field: Option<String>) -> Self {
        Self::ValidationError {
            message: message.into(),
            field,
        }
    }

    /// Create a source error
    pub fn source_error(message: impl Into<String>, origin: impl Into<String>) -> Self {
        Self::SourceError {
            message: message.into(),
            origin: origin.into(),
        }
    }

    /// Create an environment variable not found error
    pub fn env_var_not_found(name: impl Into<String>) -> Self {
        Self::EnvVarNotFound { name: name.into() }
    }

    /// Create an environment variable parse error
    pub fn env_var_parse_error(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::EnvVarParseError {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Create a reload error
    pub fn reload_error(message: impl Into<String>) -> Self {
        Self::ReloadError {
            message: message.into(),
        }
    }

    /// Create a watch error
    pub fn watch_error(message: impl Into<String>) -> Self {
        Self::WatchError {
            message: message.into(),
        }
    }

    /// Create a merge error
    pub fn merge_error(message: impl Into<String>) -> Self {
        Self::MergeError {
            message: message.into(),
        }
    }

    /// Create a type error
    pub fn type_error(
        message: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::TypeError {
            message: message.into(),
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create a path error
    pub fn path_error(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self::PathError {
            message: message.into(),
            path: path.into(),
        }
    }

    /// Create a format not supported error
    pub fn format_not_supported(format: impl Into<String>) -> Self {
        Self::FormatNotSupported {
            format: format.into(),
        }
    }

    /// Create an encryption error
    pub fn encryption_error(message: impl Into<String>) -> Self {
        Self::EncryptionError {
            message: message.into(),
        }
    }

    /// Create a decryption error
    pub fn decryption_error(message: impl Into<String>) -> Self {
        Self::DecryptionError {
            message: message.into(),
        }
    }

    /// Check if error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            ConfigError::FileNotFound { .. }
                | ConfigError::EnvVarNotFound { .. }
                | ConfigError::ValidationError { .. }
        )
    }

    /// Check if error is due to missing source
    pub fn is_missing_source(&self) -> bool {
        matches!(
            self,
            ConfigError::FileNotFound { .. } | ConfigError::EnvVarNotFound { .. }
        )
    }

    /// Get the error category
    pub fn category(&self) -> ErrorCategory {
        match self {
            ConfigError::FileNotFound { .. } | ConfigError::EnvVarNotFound { .. } => {
                ErrorCategory::NotFound
            }
            ConfigError::FileReadError { .. } | ConfigError::WatchError { .. } => ErrorCategory::Io,
            ConfigError::ParseError { .. }
            | ConfigError::EnvVarParseError { .. }
            | ConfigError::FormatNotSupported { .. } => ErrorCategory::Parse,
            ConfigError::ValidationError { .. } | ConfigError::TypeError { .. } => {
                ErrorCategory::Validation
            }
            ConfigError::SourceError { .. }
            | ConfigError::ReloadError { .. }
            | ConfigError::MergeError { .. }
            | ConfigError::PathError { .. } => ErrorCategory::Operation,
            ConfigError::EncryptionError { .. } | ConfigError::DecryptionError { .. } => {
                ErrorCategory::Security
            }
        }
    }
}

/// Error category for grouping errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Resource not found
    NotFound,
    /// I/O error
    Io,
    /// Parse error
    Parse,
    /// Validation error
    Validation,
    /// Operation error
    Operation,
    /// Security error
    Security,
}

// Implement From for common error types
impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        use std::io::ErrorKind;

        match err.kind() {
            ErrorKind::NotFound => ConfigError::file_not_found(PathBuf::from("unknown")),
            ErrorKind::PermissionDenied => ConfigError::file_read_error(
                PathBuf::from("unknown"),
                format!("Permission denied: {err}"),
            ),
            _ => ConfigError::file_read_error(PathBuf::from("unknown"), err.to_string()),
        }
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(err: serde_json::Error) -> Self {
        ConfigError::parse_error(PathBuf::from("json"), format!("JSON error: {err}"))
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> Self {
        ConfigError::parse_error(PathBuf::from("toml"), format!("TOML error: {err}"))
    }
}

impl From<yaml_rust2::ScanError> for ConfigError {
    fn from(err: yaml_rust2::ScanError) -> Self {
        ConfigError::parse_error(PathBuf::from("yaml"), format!("YAML error: {err:?}"))
    }
}

impl From<notify::Error> for ConfigError {
    fn from(err: notify::Error) -> Self {
        ConfigError::watch_error(err.to_string())
    }
}

// ==================== NebulaError Integration ====================

impl From<ConfigError> for NebulaError {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::FileNotFound { path } => {
                NebulaError::not_found("config_file", path.to_string_lossy())
            }
            ConfigError::FileReadError { path, message } => NebulaError::internal(format!(
                "Failed to read config file {}: {}",
                path.display(),
                message
            )),
            ConfigError::ParseError { path, message } => NebulaError::validation(format!(
                "Failed to parse config file {}: {}",
                path.display(),
                message
            )),
            ConfigError::ValidationError { message, field } => {
                let msg = match field {
                    Some(field) => {
                        format!("Configuration validation failed for field '{field}': {message}")
                    }
                    None => format!("Configuration validation failed: {message}"),
                };
                NebulaError::validation(msg)
            }
            ConfigError::SourceError { message, origin } => NebulaError::internal(format!(
                "Configuration source error from '{}': {}",
                origin, message
            )),
            ConfigError::EnvVarNotFound { name } => {
                NebulaError::not_found("environment_variable", name)
            }
            ConfigError::EnvVarParseError { name, value } => NebulaError::validation(format!(
                "Failed to parse environment variable {}: '{}'",
                name, value
            )),
            ConfigError::ReloadError { message } => {
                NebulaError::internal(format!("Configuration reload failed: {message}"))
            }
            ConfigError::WatchError { message } => {
                NebulaError::internal(format!("Configuration watch error: {message}"))
            }
            ConfigError::MergeError { message } => {
                NebulaError::internal(format!("Configuration merge failed: {message}"))
            }
            ConfigError::TypeError {
                message,
                expected,
                actual,
            } => NebulaError::validation(format!(
                "Type error: {} (expected {}, got {})",
                message, expected, actual
            )),
            ConfigError::PathError { message, path } => {
                NebulaError::validation(format!("Path error for '{path}': {message}"))
            }
            ConfigError::FormatNotSupported { format } => {
                NebulaError::validation(format!("Configuration format not supported: {format}"))
            }
            ConfigError::EncryptionError { message } => {
                NebulaError::internal(format!("Configuration encryption error: {message}"))
            }
            ConfigError::DecryptionError { message } => {
                NebulaError::internal(format!("Configuration decryption error: {message}"))
            }
        }
    }
}

impl From<NebulaError> for ConfigError {
    fn from(err: NebulaError) -> Self {
        if err.is_client_error() {
            ConfigError::validation_error(err.user_message().to_string(), None)
        } else {
            ConfigError::source_error(err.user_message().to_string(), "nebula_error")
        }
    }
}

/// Helper functions for creating ConfigErrors with better integration
impl ConfigError {
    /// Create a simple validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::ValidationError {
            message: message.into(),
            field: None,
        }
    }

    /// Create a validation error with field
    pub fn validation_with_field(message: impl Into<String>, field: impl Into<String>) -> Self {
        Self::ValidationError {
            message: message.into(),
            field: Some(field.into()),
        }
    }

    /// Create a not found error for a generic resource
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        let resource_type_str = resource_type.into();
        let resource_id_str = resource_id.into();
        match resource_type_str.as_str() {
            "file" => Self::file_not_found(PathBuf::from(resource_id_str)),
            "env" => Self::env_var_not_found(resource_id_str),
            _ => Self::source_error(format!("{resource_type_str} not found"), resource_id_str),
        }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::source_error(message, "internal")
    }
}
