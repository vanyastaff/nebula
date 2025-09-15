//! Configuration error types
use nebula_error::Error;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration error type
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ConfigError {
    /// Configuration file not found
    #[error("Configuration file not found: {path}")]
    FileNotFound { path: PathBuf },

    /// Configuration file read error
    #[error("Failed to read configuration file {path}: {message}")]
    FileReadError { path: PathBuf, message: String },

    /// Configuration file parse error
    #[error("Failed to parse configuration file {path}: {message}")]
    ParseError { path: PathBuf, message: String },

    /// Configuration validation error
    #[error("Configuration validation failed: {message}")]
    ValidationError {
        message: String,
        field: Option<String>,
    },

    /// Configuration source error
    #[error("Configuration source error: {message}")]
    SourceError { message: String, origin: String },

    /// Environment variable not found
    #[error("Environment variable not found: {name}")]
    EnvVarNotFound { name: String },

    /// Environment variable parse error
    #[error("Failed to parse environment variable {name}: {value}")]
    EnvVarParseError { name: String, value: String },

    /// Configuration reload error
    #[error("Failed to reload configuration: {message}")]
    ReloadError { message: String },

    /// Configuration watch error
    #[error("Configuration watch error: {message}")]
    WatchError { message: String },

    /// Configuration merge error
    #[error("Failed to merge configurations: {message}")]
    MergeError { message: String },

    /// Configuration type error
    #[error("Configuration type error: {message}")]
    TypeError {
        message: String,
        expected: String,
        actual: String,
    },

    /// Configuration path error
    #[error("Configuration path error: {message}")]
    PathError { message: String, path: String },

    /// Configuration format not supported
    #[error("Configuration format not supported: {format}")]
    FormatNotSupported { format: String },

    /// Configuration encryption error
    #[error("Configuration encryption error: {message}")]
    EncryptionError { message: String },

    /// Configuration decryption error
    #[error("Configuration decryption error: {message}")]
    DecryptionError { message: String },
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
}

/// Result type for configuration operations
pub type ConfigResult<T> = Result<T, ConfigError>;

// Implement From for common error types
impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::file_read_error(PathBuf::from("unknown"), err.to_string())
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(err: serde_json::Error) -> Self {
        ConfigError::parse_error(PathBuf::from("unknown"), format!("JSON error: {}", err))
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> Self {
        ConfigError::parse_error(PathBuf::from("unknown"), format!("TOML error: {}", err))
    }
}

impl From<yaml_rust::ScanError> for ConfigError {
    fn from(err: yaml_rust::ScanError) -> Self {
        ConfigError::parse_error(PathBuf::from("unknown"), format!("YAML error: {}", err))
    }
}

impl From<notify::Error> for ConfigError {
    fn from(err: notify::Error) -> Self {
        ConfigError::watch_error(err.to_string())
    }
}
