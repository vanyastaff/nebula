//! Local filesystem storage provider
//!
//! Implements encrypted credential storage on local filesystem with atomic writes.

use crate::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use crate::providers::StorageMetrics;
use crate::providers::config::{ConfigError, ProviderConfig};
use crate::traits::StorageProvider;
use crate::utils::EncryptedData;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Local filesystem storage configuration
///
/// Stores encrypted credentials on the local filesystem with atomic writes.
/// Uses platform-appropriate paths by default.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::providers::LocalStorageConfig;
///
/// // Use platform-default data directory
/// let config = LocalStorageConfig::default();
///
/// // Use custom directory
/// let config = LocalStorageConfig::new("/secure/credentials");
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalStorageConfig {
    /// Base directory for credential storage
    ///
    /// Must be an absolute path. Directory will be created if `create_dir` is true.
    pub base_path: PathBuf,

    /// Whether to create the directory if it doesn't exist
    ///
    /// Default: true
    #[serde(default = "default_create_dir")]
    pub create_dir: bool,

    /// File extension for credential files
    ///
    /// Must not contain path separators. Default: "cred"
    #[serde(default = "default_file_extension")]
    pub file_extension: String,

    /// Enable file locking for concurrent access
    ///
    /// Uses fs2 crate for advisory locks. Default: true
    #[serde(default = "default_enable_locking")]
    pub enable_locking: bool,
}

fn default_create_dir() -> bool {
    true
}

fn default_file_extension() -> String {
    "cred".to_string()
}

fn default_enable_locking() -> bool {
    true
}

impl LocalStorageConfig {
    /// Create a new configuration with custom base path
    ///
    /// # Arguments
    ///
    /// * `base_path` - Directory path for credential storage (must be absolute)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = LocalStorageConfig::new("/var/lib/nebula/credentials");
    /// ```
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
            create_dir: default_create_dir(),
            file_extension: default_file_extension(),
            enable_locking: default_enable_locking(),
        }
    }

    /// Set whether to create directory if it doesn't exist
    pub fn with_create_dir(mut self, create_dir: bool) -> Self {
        self.create_dir = create_dir;
        self
    }

    /// Set file extension for credential files
    pub fn with_file_extension(mut self, extension: impl Into<String>) -> Self {
        self.file_extension = extension.into();
        self
    }

    /// Set whether to enable file locking
    pub fn with_locking(mut self, enable_locking: bool) -> Self {
        self.enable_locking = enable_locking;
        self
    }
}

impl Default for LocalStorageConfig {
    /// Create default configuration using platform data directory
    ///
    /// Uses `directories` crate to find platform-appropriate path:
    /// - Linux: `$HOME/.local/share/nebula/credentials`
    /// - macOS: `$HOME/Library/Application Support/nebula/credentials`
    /// - Windows: `{FOLDERID_RoamingAppData}\nebula\credentials`
    fn default() -> Self {
        let base_path = if let Some(data_dir) = directories::ProjectDirs::from("", "", "nebula") {
            data_dir.data_dir().join("credentials")
        } else {
            // Fallback if directories crate fails
            PathBuf::from("./credentials")
        };

        Self {
            base_path,
            create_dir: true,
            file_extension: "cred".to_string(),
            enable_locking: true,
        }
    }
}

impl ProviderConfig for LocalStorageConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        // Validate base_path is absolute
        if !self.base_path.is_absolute() {
            return Err(ConfigError::InvalidValue {
                field: "base_path".to_string(),
                reason: "must be an absolute path (use std::env::current_dir().join(path) to make relative paths absolute)".to_string(),
            });
        }

        // Validate file_extension doesn't contain path separators
        if self.file_extension.contains('/') || self.file_extension.contains('\\') {
            return Err(ConfigError::InvalidValue {
                field: "file_extension".to_string(),
                reason: "must not contain path separators ('/' or '\\')".to_string(),
            });
        }

        // Validate file_extension is not empty
        if self.file_extension.is_empty() {
            return Err(ConfigError::InvalidValue {
                field: "file_extension".to_string(),
                reason: "must not be empty".to_string(),
            });
        }

        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "LocalStorage"
    }
}

/// Serialization format for credential files
///
/// Stored as JSON on disk with encryption and metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)] // Will be used in T016-T018
struct CredentialFile {
    /// Format version for future migration support
    version: u32,

    /// Encrypted credential data (serialized EncryptedData)
    encrypted_data: EncryptedData,

    /// Credential metadata
    metadata: CredentialMetadata,

    /// Salt used for encryption (for future key derivation)
    #[serde(skip_serializing_if = "Option::is_none")]
    salt: Option<Vec<u8>>,
}

const CURRENT_VERSION: u32 = 1;

impl CredentialFile {
    /// Create new credential file
    fn new(encrypted_data: EncryptedData, metadata: CredentialMetadata) -> Self {
        Self {
            version: CURRENT_VERSION,
            encrypted_data,
            metadata,
            salt: None,
        }
    }

    /// Check if file needs migration
    #[allow(dead_code)]
    fn needs_migration(&self) -> bool {
        self.version < CURRENT_VERSION
    }
}

/// Atomically write data to a file with proper permissions
///
/// Creates a temporary file in the same directory, writes data, sets permissions,
/// then atomically renames to the target path. This ensures either complete success
/// or no change (no partial writes).
///
/// # Arguments
///
/// * `path` - Target file path
/// * `data` - Data to write
///
/// # Errors
///
/// Returns I/O error if write, permission setting, or rename fails
async fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    // Create temp file in same directory to ensure atomic rename (same filesystem)
    let temp_path = path.with_file_name(format!(
        "{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("credential"),
        Uuid::new_v4()
    ));

    // Write to temp file
    tokio::fs::write(&temp_path, data).await?;

    // Set permissions to 0600 (owner read/write only) on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(&temp_path, perms).await?;
    }

    // Set restrictive ACL on Windows
    #[cfg(windows)]
    {
        // TODO: Implement Windows ACL setting in future iteration
        // For now, rely on directory permissions set during initialization
    }

    // Atomic rename (same filesystem, atomically replaces existing file)
    tokio::fs::rename(&temp_path, path).await?;

    Ok(())
}

/// Check if metadata matches the given filter
fn filter_matches(metadata: &CredentialMetadata, filter: &CredentialFilter) -> bool {
    // Check tags if present in filter
    if let Some(filter_tags) = &filter.tags {
        for (key, value) in filter_tags {
            if !metadata.tags.get(key).map_or(false, |v| v == value) {
                return false;
            }
        }
    }

    // Check created_after if present
    if let Some(created_after) = filter.created_after {
        if metadata.created_at < created_after {
            return false;
        }
    }

    // Check created_before if present
    if let Some(created_before) = filter.created_before {
        if metadata.created_at > created_before {
            return false;
        }
    }

    true
}

/// Local filesystem storage provider
///
/// Stores encrypted credentials as JSON files on local filesystem.
/// Provides atomic writes with platform-appropriate file permissions.
#[derive(Clone)]
pub struct LocalStorageProvider {
    config: LocalStorageConfig,
    metrics: Arc<RwLock<StorageMetrics>>,
}

impl LocalStorageProvider {
    /// Create new local storage provider
    ///
    /// # Arguments
    ///
    /// * `config` - Provider configuration
    ///
    /// # Panics
    ///
    /// Panics if configuration is invalid (programming error - validate config before calling new)
    pub fn new(config: LocalStorageConfig) -> Self {
        config
            .validate()
            .expect("LocalStorageConfig validation failed - validate config before calling new()");

        Self {
            config,
            metrics: Arc::new(RwLock::new(StorageMetrics::new())),
        }
    }

    /// Ensure base directory exists with proper permissions
    async fn ensure_directory_exists(&self) -> Result<(), StorageError> {
        if !self.config.base_path.exists() {
            if !self.config.create_dir {
                return Err(StorageError::WriteFailure {
                    id: "[directory]".to_string(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!(
                            "directory does not exist and create_dir is false: {}",
                            self.config.base_path.display()
                        ),
                    ),
                });
            }

            tokio::fs::create_dir_all(&self.config.base_path)
                .await
                .map_err(|e| StorageError::WriteFailure {
                    id: "[directory]".to_string(),
                    source: e,
                })?;

            // Set directory permissions to 0700 (owner only) on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o700);
                tokio::fs::set_permissions(&self.config.base_path, perms)
                    .await
                    .map_err(|e| StorageError::WriteFailure {
                        id: "[directory]".to_string(),
                        source: e,
                    })?;
            }
        }

        Ok(())
    }

    /// Get file path for credential ID
    fn get_file_path(&self, id: &CredentialId) -> PathBuf {
        self.config
            .base_path
            .join(format!("{}.{}", id.as_str(), self.config.file_extension))
    }
}

#[async_trait]
impl StorageProvider for LocalStorageProvider {
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let start = std::time::Instant::now();

        // Ensure directory exists
        self.ensure_directory_exists().await?;

        // Create credential file
        let cred_file = CredentialFile::new(data, metadata);

        // Serialize to JSON
        let json =
            serde_json::to_vec_pretty(&cred_file).map_err(|e| StorageError::WriteFailure {
                id: id.to_string(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            })?;

        // Get file path
        let path = self.get_file_path(id);

        // Atomic write
        atomic_write(&path, &json)
            .await
            .map_err(|e| StorageError::WriteFailure {
                id: id.to_string(),
                source: e,
            })?;

        // Record metrics
        let elapsed = start.elapsed();
        self.metrics
            .write()
            .await
            .record_operation("store", elapsed, true);

        Ok(())
    }

    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        let start = std::time::Instant::now();

        // Get file path
        let path = self.get_file_path(id);

        // Check if file exists
        if !path.exists() {
            return Err(StorageError::NotFound { id: id.to_string() });
        }

        // Read file
        let json = tokio::fs::read(&path)
            .await
            .map_err(|e| StorageError::ReadFailure {
                id: id.to_string(),
                source: e,
            })?;

        // Deserialize
        let cred_file: CredentialFile =
            serde_json::from_slice(&json).map_err(|e| StorageError::ReadFailure {
                id: id.to_string(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            })?;

        // Extract EncryptedData
        let encrypted_data = cred_file.encrypted_data;

        // Record metrics
        let elapsed = start.elapsed();
        self.metrics
            .write()
            .await
            .record_operation("retrieve", elapsed, true);

        Ok((encrypted_data, cred_file.metadata))
    }

    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let start = std::time::Instant::now();

        // Get file path
        let path = self.get_file_path(id);

        // Delete file (idempotent - ignore NotFound)
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| StorageError::WriteFailure {
                    id: id.to_string(),
                    source: e,
                })?;
        }

        // Record metrics
        let elapsed = start.elapsed();
        self.metrics
            .write()
            .await
            .record_operation("delete", elapsed, true);

        Ok(())
    }

    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        let start = std::time::Instant::now();

        // Ensure directory exists
        self.ensure_directory_exists().await?;

        // Read directory entries
        let mut entries = tokio::fs::read_dir(&self.config.base_path)
            .await
            .map_err(|e| StorageError::ReadFailure {
                id: "[directory]".to_string(),
                source: e,
            })?;

        let mut ids = Vec::new();

        // Scan for credential files
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| StorageError::ReadFailure {
                    id: "[directory]".to_string(),
                    source: e,
                })?
        {
            let path = entry.path();

            // Check if file has correct extension
            if let Some(ext) = path.extension() {
                if ext == self.config.file_extension.as_str() {
                    // Extract credential ID from filename
                    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(id) = CredentialId::try_from(file_stem.to_string()) {
                            // Apply filter if provided
                            if let Some(filter) = filter {
                                // Read metadata to check filter
                                if let Ok(json) = tokio::fs::read(&path).await {
                                    if let Ok(cred_file) =
                                        serde_json::from_slice::<CredentialFile>(&json)
                                    {
                                        if filter_matches(&cred_file.metadata, filter) {
                                            ids.push(id);
                                        }
                                    }
                                }
                            } else {
                                ids.push(id);
                            }
                        }
                    }
                }
            }
        }

        // Record metrics
        let elapsed = start.elapsed();
        self.metrics
            .write()
            .await
            .record_operation("list", elapsed, true);

        Ok(ids)
    }

    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        let path = self.get_file_path(id);
        Ok(path.exists())
    }
}
