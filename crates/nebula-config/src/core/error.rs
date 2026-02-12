//! Configuration error types

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Configuration error type
#[non_exhaustive]
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

/// Error category for grouping errors
#[non_exhaustive]
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

// ConfigError can be converted to other error types as needed
// by implementing From traits in consuming crates

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_constructors_and_display() {
        let e = ConfigError::file_not_found("/tmp/missing.json");
        assert!(e.to_string().contains("missing.json"));

        let e = ConfigError::file_read_error("/tmp/f.json", "permission denied");
        assert!(e.to_string().contains("permission denied"));

        let e = ConfigError::parse_error("/tmp/f.json", "unexpected token");
        assert!(e.to_string().contains("unexpected token"));

        let e = ConfigError::validation_error("field too short", Some("name".into()));
        assert!(e.to_string().contains("field too short"));

        let e = ConfigError::source_error("connection refused", "redis");
        assert!(e.to_string().contains("connection refused"));

        let e = ConfigError::env_var_not_found("MY_VAR");
        assert!(e.to_string().contains("MY_VAR"));

        let e = ConfigError::env_var_parse_error("PORT", "abc");
        assert!(e.to_string().contains("PORT"));

        let e = ConfigError::reload_error("source unavailable");
        assert!(e.to_string().contains("source unavailable"));

        let e = ConfigError::watch_error("inotify failed");
        assert!(e.to_string().contains("inotify failed"));

        let e = ConfigError::merge_error("conflicting keys");
        assert!(e.to_string().contains("conflicting keys"));

        let e = ConfigError::type_error("expected string", "String", "Number");
        assert!(e.to_string().contains("expected string"));

        let e = ConfigError::path_error("key not found", "a.b.c");
        assert!(e.to_string().contains("key not found"));

        let e = ConfigError::format_not_supported("xml");
        assert!(e.to_string().contains("xml"));

        let e = ConfigError::encryption_error("key expired");
        assert!(e.to_string().contains("key expired"));

        let e = ConfigError::decryption_error("invalid key");
        assert!(e.to_string().contains("invalid key"));
    }

    #[test]
    fn test_error_is_recoverable() {
        assert!(ConfigError::file_not_found("/tmp/f").is_recoverable());
        assert!(ConfigError::env_var_not_found("VAR").is_recoverable());
        assert!(ConfigError::validation("bad").is_recoverable());

        assert!(!ConfigError::parse_error("/tmp/f", "bad").is_recoverable());
        assert!(!ConfigError::merge_error("conflict").is_recoverable());
        assert!(!ConfigError::encryption_error("fail").is_recoverable());
    }

    #[test]
    fn test_error_is_missing_source() {
        assert!(ConfigError::file_not_found("/tmp/f").is_missing_source());
        assert!(ConfigError::env_var_not_found("VAR").is_missing_source());

        assert!(!ConfigError::parse_error("/tmp/f", "bad").is_missing_source());
        assert!(!ConfigError::validation("bad").is_missing_source());
    }

    #[test]
    fn test_error_category() {
        assert_eq!(
            ConfigError::file_not_found("/f").category(),
            ErrorCategory::NotFound
        );
        assert_eq!(
            ConfigError::env_var_not_found("V").category(),
            ErrorCategory::NotFound
        );
        assert_eq!(
            ConfigError::file_read_error("/f", "e").category(),
            ErrorCategory::Io
        );
        assert_eq!(ConfigError::watch_error("e").category(), ErrorCategory::Io);
        assert_eq!(
            ConfigError::parse_error("/f", "e").category(),
            ErrorCategory::Parse
        );
        assert_eq!(
            ConfigError::env_var_parse_error("V", "x").category(),
            ErrorCategory::Parse
        );
        assert_eq!(
            ConfigError::format_not_supported("xml").category(),
            ErrorCategory::Parse
        );
        assert_eq!(
            ConfigError::validation("e").category(),
            ErrorCategory::Validation
        );
        assert_eq!(
            ConfigError::type_error("e", "a", "b").category(),
            ErrorCategory::Validation
        );
        assert_eq!(
            ConfigError::source_error("e", "o").category(),
            ErrorCategory::Operation
        );
        assert_eq!(
            ConfigError::reload_error("e").category(),
            ErrorCategory::Operation
        );
        assert_eq!(
            ConfigError::merge_error("e").category(),
            ErrorCategory::Operation
        );
        assert_eq!(
            ConfigError::path_error("e", "p").category(),
            ErrorCategory::Operation
        );
        assert_eq!(
            ConfigError::encryption_error("e").category(),
            ErrorCategory::Security
        );
        assert_eq!(
            ConfigError::decryption_error("e").category(),
            ErrorCategory::Security
        );
    }

    #[test]
    fn test_error_from_conversions() {
        // From<io::Error> - NotFound
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let cfg_err: ConfigError = io_err.into();
        assert!(matches!(cfg_err, ConfigError::FileNotFound { .. }));

        // From<io::Error> - PermissionDenied
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
        let cfg_err: ConfigError = io_err.into();
        assert!(matches!(cfg_err, ConfigError::FileReadError { .. }));

        // From<serde_json::Error>
        let json_err = serde_json::from_str::<serde_json::Value>("{bad}").unwrap_err();
        let cfg_err: ConfigError = json_err.into();
        assert!(matches!(cfg_err, ConfigError::ParseError { .. }));
    }

    #[test]
    fn test_error_not_found_and_internal() {
        let e = ConfigError::not_found("file", "/tmp/f.json");
        assert!(matches!(e, ConfigError::FileNotFound { .. }));

        let e = ConfigError::not_found("env", "MY_VAR");
        assert!(matches!(e, ConfigError::EnvVarNotFound { .. }));

        let e = ConfigError::not_found("redis", "key123");
        assert!(matches!(e, ConfigError::SourceError { .. }));

        let e = ConfigError::internal("something broke");
        assert!(matches!(e, ConfigError::SourceError { origin, .. } if origin == "internal"));
    }
}
