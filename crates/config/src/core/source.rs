//! Configuration source definitions

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration source type
#[non_exhaustive]
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

    /// Default values
    Default,
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
            ConfigSource::Default => "default",
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
            ConfigSource::Default => write!(f, "default values"),
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
    #[must_use = "builder methods must be chained or built"]
    pub fn with_last_modified(mut self, timestamp: chrono::DateTime<chrono::Utc>) -> Self {
        self.last_modified = Some(timestamp);
        self
    }

    /// Set version
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set checksum
    #[must_use = "builder methods must be chained or built"]
    pub fn with_checksum(mut self, checksum: impl Into<String>) -> Self {
        self.checksum = Some(checksum.into());
        self
    }

    /// Set size
    #[must_use = "builder methods must be chained or built"]
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Set format
    #[must_use = "builder methods must be chained or built"]
    pub fn with_format(mut self, format: ConfigFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Set encoding
    #[must_use = "builder methods must be chained or built"]
    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = Some(encoding.into());
        self
    }

    /// Set compression
    #[must_use = "builder methods must be chained or built"]
    pub fn with_compression(mut self, compression: impl Into<String>) -> Self {
        self.compression = Some(compression.into());
        self
    }

    /// Set encryption
    #[must_use = "builder methods must be chained or built"]
    pub fn with_encryption(mut self, encryption: impl Into<String>) -> Self {
        self.encryption = Some(encryption.into());
        self
    }

    /// Add extra metadata
    #[must_use = "builder methods must be chained or built"]
    pub fn with_extra(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }
}

/// Configuration format
#[non_exhaustive]
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
            "toml" => {
                #[cfg(feature = "toml")]
                {
                    ConfigFormat::Toml
                }
                #[cfg(not(feature = "toml"))]
                {
                    ConfigFormat::Unknown(ext.to_string())
                }
            }
            "yaml" | "yml" => {
                #[cfg(feature = "yaml")]
                {
                    ConfigFormat::Yaml
                }
                #[cfg(not(feature = "yaml"))]
                {
                    ConfigFormat::Unknown(ext.to_string())
                }
            }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_config_source_type_checks() {
        let file = ConfigSource::File(PathBuf::from("config.json"));
        assert!(file.is_file_based());
        assert!(!file.is_env_based());

        assert!(ConfigSource::FileAuto(PathBuf::from("f")).is_file_based());
        assert!(ConfigSource::Directory(PathBuf::from("d")).is_file_based());

        assert!(ConfigSource::Env.is_env_based());
        assert!(ConfigSource::EnvWithPrefix("APP".into()).is_env_based());
        assert!(!ConfigSource::Env.is_file_based());
    }

    #[test]
    fn test_config_source_optional_and_priority() {
        assert!(ConfigSource::Env.is_optional());
        assert!(ConfigSource::EnvWithPrefix("X".into()).is_optional());
        assert!(ConfigSource::Default.is_optional());
        assert!(!ConfigSource::File(PathBuf::from("f")).is_optional());

        assert_eq!(ConfigSource::Default.priority(), 100);
        assert_eq!(ConfigSource::File(PathBuf::from("f")).priority(), 50);
        assert_eq!(ConfigSource::Env.priority(), 30);
        assert_eq!(ConfigSource::Directory(PathBuf::from("d")).priority(), 40);
    }

    #[test]
    fn test_config_source_name_and_display() {
        assert_eq!(ConfigSource::Env.name(), "environment");
        assert_eq!(ConfigSource::Default.name(), "default");
        assert_eq!(ConfigSource::File(PathBuf::from("f.json")).name(), "file");

        let display = format!("{}", ConfigSource::Env);
        assert_eq!(display, "environment variables");

        let display = format!("{}", ConfigSource::EnvWithPrefix("APP".into()));
        assert!(display.contains("APP"));

        let display = format!("{}", ConfigSource::File(PathBuf::from("config.json")));
        assert!(display.contains("config.json"));
    }

    #[test]
    fn test_config_format_extension_mime_from() {
        assert_eq!(ConfigFormat::Json.extension(), "json");
        assert_eq!(ConfigFormat::Toml.extension(), "toml");
        assert_eq!(ConfigFormat::Yaml.extension(), "yml");
        assert_eq!(ConfigFormat::Ini.extension(), "ini");

        assert_eq!(ConfigFormat::Json.mime_type(), "application/json");
        assert_eq!(ConfigFormat::Yaml.mime_type(), "application/x-yaml");
        assert_eq!(ConfigFormat::Ini.mime_type(), "text/plain");

        assert!(matches!(
            ConfigFormat::from_extension("json"),
            ConfigFormat::Unknown(_)
        ));
        #[cfg(feature = "yaml")]
        assert_eq!(ConfigFormat::from_extension("yml"), ConfigFormat::Yaml);
        #[cfg(feature = "yaml")]
        assert_eq!(ConfigFormat::from_extension("yaml"), ConfigFormat::Yaml);
        #[cfg(not(feature = "yaml"))]
        assert!(matches!(
            ConfigFormat::from_extension("yml"),
            ConfigFormat::Unknown(_)
        ));
        assert!(matches!(
            ConfigFormat::from_extension("ini"),
            ConfigFormat::Unknown(_)
        ));
        assert!(matches!(
            ConfigFormat::from_extension("xyz"),
            ConfigFormat::Unknown(_)
        ));

        assert_eq!(
            ConfigFormat::from_path(Path::new("config.toml")),
            ConfigFormat::Toml
        );
        assert!(matches!(
            ConfigFormat::from_path(Path::new("noext")),
            ConfigFormat::Unknown(_)
        ));
    }

    #[test]
    fn test_source_metadata_builder() {
        let meta = SourceMetadata::new(ConfigSource::Default)
            .with_version("1.0")
            .with_checksum("abc123")
            .with_size(1024)
            .with_format(ConfigFormat::Json)
            .with_encoding("utf-8")
            .with_compression("gzip")
            .with_encryption("aes-256")
            .with_extra("custom", serde_json::json!("value"));

        assert_eq!(meta.source, ConfigSource::Default);
        assert_eq!(meta.version.as_deref(), Some("1.0"));
        assert_eq!(meta.checksum.as_deref(), Some("abc123"));
        assert_eq!(meta.size, Some(1024));
        assert_eq!(meta.format, Some(ConfigFormat::Json));
        assert_eq!(meta.encoding.as_deref(), Some("utf-8"));
        assert_eq!(meta.compression.as_deref(), Some("gzip"));
        assert_eq!(meta.encryption.as_deref(), Some("aes-256"));
        assert_eq!(meta.extra["custom"], serde_json::json!("value"));
    }
}
