//! Configuration source definitions

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration source type
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfigSource {
    /// Environment variables
    Env,

    /// Environment variables with prefix
    EnvWithPrefix(String),

    /// Configuration file
    File(PathBuf),

    /// Configuration file with format auto-detection
    FileAuto(PathBuf),

    /// Configuration directory (load all files)
    Directory(PathBuf),

    /// Remote configuration (HTTP/HTTPS)
    Remote(String),

    /// Database configuration
    Database {
        url: String,
        table: String,
        key: String,
    },

    /// Key-value store
    KeyValue {
        url: String,
        bucket: String
    },

    /// Default values
    Default,

    /// Command line arguments
    CommandLine,

    /// Inline configuration
    Inline(String),
}

impl ConfigSource {
    /// Check if this source is file-based
    pub fn is_file_based(&self) -> bool {
        matches!(
            self,
            ConfigSource::File(_) | ConfigSource::FileAuto(_) | ConfigSource::Directory(_)
        )
    }

    /// Check if this source is environment-based
    pub fn is_env_based(&self) -> bool {
        matches!(self, ConfigSource::Env | ConfigSource::EnvWithPrefix(_))
    }

    /// Check if this source is remote
    pub fn is_remote(&self) -> bool {
        matches!(self, ConfigSource::Remote(_))
    }

    /// Check if this source is database-based
    pub fn is_database(&self) -> bool {
        matches!(self, ConfigSource::Database { .. })
    }

    /// Check if this source is key-value based
    pub fn is_key_value(&self) -> bool {
        matches!(self, ConfigSource::KeyValue { .. })
    }

    /// Check if this source is optional (can fail without error)
    pub fn is_optional(&self) -> bool {
        matches!(
            self,
            ConfigSource::Env | ConfigSource::EnvWithPrefix(_) | ConfigSource::Default
        )
    }

    /// Get the priority of this source (lower = higher priority)
    pub fn priority(&self) -> u8 {
        match self {
            ConfigSource::Default => 100,
            ConfigSource::File(_) | ConfigSource::FileAuto(_) => 50,
            ConfigSource::Directory(_) => 40,
            ConfigSource::Env | ConfigSource::EnvWithPrefix(_) => 30,
            ConfigSource::CommandLine => 20,
            ConfigSource::Remote(_) => 10,
            ConfigSource::Database { .. } => 5,
            ConfigSource::KeyValue { .. } => 5,
            ConfigSource::Inline(_) => 1,
        }
    }

    /// Get the source name for display
    pub fn name(&self) -> &'static str {
        match self {
            ConfigSource::Env => "environment",
            ConfigSource::EnvWithPrefix(_) => "environment (prefixed)",
            ConfigSource::File(_) => "file",
            ConfigSource::FileAuto(_) => "file (auto-detect)",
            ConfigSource::Directory(_) => "directory",
            ConfigSource::Remote(_) => "remote",
            ConfigSource::Database { .. } => "database",
            ConfigSource::KeyValue { .. } => "key-value store",
            ConfigSource::Default => "default",
            ConfigSource::CommandLine => "command line",
            ConfigSource::Inline(_) => "inline",
        }
    }
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigSource::Env => write!(f, "environment variables"),
            ConfigSource::EnvWithPrefix(prefix) => {
                write!(f, "environment variables (prefix: {})", prefix)
            }
            ConfigSource::File(path) => write!(f, "file: {}", path.display()),
            ConfigSource::FileAuto(path) => write!(f, "file (auto): {}", path.display()),
            ConfigSource::Directory(path) => write!(f, "directory: {}", path.display()),
            ConfigSource::Remote(url) => write!(f, "remote: {}", url),
            ConfigSource::Database { url, table, key } => {
                write!(f, "database: {} (table: {}, key: {})", url, table, key)
            }
            ConfigSource::KeyValue { url, bucket } => {
                write!(f, "key-value store: {} (bucket: {})", url, bucket)
            }
            ConfigSource::Default => write!(f, "default values"),
            ConfigSource::CommandLine => write!(f, "command line arguments"),
            ConfigSource::Inline(data) => {
                let preview = if data.len() > 50 {
                    format!("{}...", &data[..50])
                } else {
                    data.clone()
                };
                write!(f, "inline: {}", preview)
            }
        }
    }
}

/// Configuration source metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMetadata {
    /// Source identifier
    pub source: ConfigSource,

    /// Last modified timestamp
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,

    /// Source version/ETag
    pub version: Option<String>,

    /// Source checksum
    pub checksum: Option<String>,

    /// Source size in bytes
    pub size: Option<u64>,

    /// Source format
    pub format: Option<ConfigFormat>,

    /// Source encoding
    pub encoding: Option<String>,

    /// Source compression
    pub compression: Option<String>,

    /// Source encryption
    pub encryption: Option<String>,

    /// Additional metadata
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl SourceMetadata {
    /// Create new source metadata
    pub fn new(source: ConfigSource) -> Self {
        Self {
            source,
            last_modified: None,
            version: None,
            checksum: None,
            size: None,
            format: None,
            encoding: None,
            compression: None,
            encryption: None,
            extra: std::collections::HashMap::new(),
        }
    }

    /// Set last modified timestamp
    pub fn with_last_modified(mut self, timestamp: chrono::DateTime<chrono::Utc>) -> Self {
        self.last_modified = Some(timestamp);
        self
    }

    /// Set version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set checksum
    pub fn with_checksum(mut self, checksum: impl Into<String>) -> Self {
        self.checksum = Some(checksum.into());
        self
    }

    /// Set size
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Set format
    pub fn with_format(mut self, format: ConfigFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Set encoding
    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = Some(encoding.into());
        self
    }

    /// Set compression
    pub fn with_compression(mut self, compression: impl Into<String>) -> Self {
        self.compression = Some(compression.into());
        self
    }

    /// Set encryption
    pub fn with_encryption(mut self, encryption: impl Into<String>) -> Self {
        self.encryption = Some(encryption.into());
        self
    }

    /// Add extra metadata
    pub fn with_extra(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }
}

/// Configuration format
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfigFormat {
    /// JSON format
    Json,

    /// TOML format
    Toml,

    /// YAML format
    Yaml,

    /// INI format
    Ini,

    /// HCL format (HashiCorp Configuration Language)
    Hcl,

    /// Properties format
    Properties,

    /// Environment format
    Env,

    /// Unknown format
    Unknown(String),
}

impl ConfigFormat {
    /// Get file extension for this format
    pub fn extension(&self) -> &str {
        match self {
            ConfigFormat::Json => "json",
            ConfigFormat::Toml => "toml",
            ConfigFormat::Yaml => "yml",
            ConfigFormat::Ini => "ini",
            ConfigFormat::Hcl => "hcl",
            ConfigFormat::Properties => "properties",
            ConfigFormat::Env => "env",
            ConfigFormat::Unknown(ext) => ext,
        }
    }

    /// Get MIME type for this format
    pub fn mime_type(&self) -> &str {
        match self {
            ConfigFormat::Json => "application/json",
            ConfigFormat::Toml => "application/toml",
            ConfigFormat::Yaml => "application/x-yaml",
            ConfigFormat::Ini => "text/plain",
            ConfigFormat::Hcl => "application/hcl",
            ConfigFormat::Properties => "text/plain",
            ConfigFormat::Env => "text/plain",
            ConfigFormat::Unknown(_) => "application/octet-stream",
        }
    }

    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "json" => ConfigFormat::Json,
            "toml" => ConfigFormat::Toml,
            "yml" | "yaml" => ConfigFormat::Yaml,
            "ini" | "cfg" => ConfigFormat::Ini,
            "hcl" | "tf" => ConfigFormat::Hcl,
            "properties" | "props" => ConfigFormat::Properties,
            "env" => ConfigFormat::Env,
            _ => ConfigFormat::Unknown(ext.to_string()),
        }
    }

    /// Detect format from file path
    pub fn from_path(path: &std::path::Path) -> Self {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(Self::from_extension)
            .unwrap_or(ConfigFormat::Unknown("no_extension".to_string()))
    }
}

impl std::fmt::Display for ConfigFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigFormat::Json => write!(f, "JSON"),
            ConfigFormat::Toml => write!(f, "TOML"),
            ConfigFormat::Yaml => write!(f, "YAML"),
            ConfigFormat::Ini => write!(f, "INI"),
            ConfigFormat::Hcl => write!(f, "HCL"),
            ConfigFormat::Properties => write!(f, "Properties"),
            ConfigFormat::Env => write!(f, "Environment"),
            ConfigFormat::Unknown(s) => write!(f, "Unknown ({})", s),
        }
    }
}