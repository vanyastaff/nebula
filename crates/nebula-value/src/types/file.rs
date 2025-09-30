use std::collections::BTreeMap;
use std::fmt;
use std::hash::Hash;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{BinaryValue, ValueError, ValueResult};

/// File value with flattened structure - no nested location object
#[derive(Debug, Clone, PartialEq, Hash, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum FileValue {
    /// Data fully loaded in memory
    InMemory {
        data: BinaryValue,

        #[cfg_attr(feature = "serde", serde(flatten))]
        metadata: FileMetadata,
    },

    /// Data stored in remote storage
    Remote {
        storage_key: String,
        storage_type: StorageType,
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        credentials_ref: Option<String>,
        #[cfg_attr(feature = "serde", serde(flatten))]
        metadata: FileMetadata,
    },

    /// Data available via URL
    Url {
        url: String,
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        headers: Option<BTreeMap<String, String>>,
        #[cfg_attr(feature = "serde", serde(default = "default_true"))]
        follow_redirects: bool,
        #[cfg_attr(feature = "serde", serde(flatten))]
        metadata: FileMetadata,
    },

    /// File requires generation
    Generated {
        generator_id: String,
        #[cfg(feature = "json")]
        parameters: serde_json::Value,
        #[cfg(not(feature = "json"))]
        parameters: String, // Fallback to string if no JSON support
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        cache_key: Option<String>,
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        #[cfg_attr(feature = "serde", serde(with = "optional_duration_seconds"))]
        timeout_seconds: Option<u64>,
        #[cfg_attr(feature = "serde", serde(flatten))]
        metadata: FileMetadata,
    },

    /// Reference to a temporary file
    Temporary {
        path: String,
        #[cfg_attr(feature = "serde", serde(default))]
        cleanup_on_drop: bool,
        #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
        owner_id: Option<String>,
        #[cfg_attr(feature = "serde", serde(flatten))]
        metadata: FileMetadata,
    },

    /// Stream-based access
    Stream {
        stream_id: String,
        #[cfg_attr(feature = "serde", serde(default = "default_chunk_size"))]
        chunk_size: usize,
        #[cfg_attr(feature = "serde", serde(default))]
        seekable: bool,
        #[cfg_attr(feature = "serde", serde(flatten))]
        metadata: FileMetadata,
    },
}

/// Storage backend types
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum StorageType {
    S3,
    LocalFileSystem,
    AzureBlob,
    GoogleCloud,
    Custom(String),
}

/// File metadata (now flattened into each variant)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FileMetadata {
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub filename: Option<String>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub mime_type: Option<String>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub size: Option<usize>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub created_at: Option<DateTime<Utc>>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub modified_at: Option<DateTime<Utc>>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub checksum: Option<String>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub encoding: Option<String>,

    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub format_version: Option<String>,

    #[cfg_attr(feature = "serde", serde(default))]
    pub is_sensitive: bool,

    #[cfg(feature = "json")]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "BTreeMap::is_empty"))]
    pub custom: BTreeMap<String, serde_json::Value>,

    #[cfg(not(feature = "json"))]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "HashMap::is_empty"))]
    pub custom: BTreeMap<String, String>, // Fallback to string values
}

impl Hash for FileMetadata {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.filename.hash(state);
        self.mime_type.hash(state);
        self.size.hash(state);
        self.created_at.hash(state);
        self.modified_at.hash(state);
        self.checksum.hash(state);
        self.encoding.hash(state);
        self.format_version.hash(state);
        self.is_sensitive.hash(state);
        // Custom metadata is not hashed for simplicity
    }
}

#[cfg(feature = "serde")]
mod optional_duration_seconds {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(duration: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        duration.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where D: Deserializer<'de> {
        Option::<u64>::deserialize(deserializer)
    }
}

// Helper functions
#[cfg(feature = "serde")]
fn default_true() -> bool {
    true
}

#[cfg(feature = "serde")]
fn default_chunk_size() -> usize {
    1_048_576 // 1MB
}

impl FileValue {
    // === Constructor methods ===

    /// Creates a new in-memory file
    pub fn from_bytes(data: Vec<u8>, filename: Option<String>) -> Self {
        let binary_data = BinaryValue::from(data);
        let size = binary_data.len();
        Self::InMemory {
            data: binary_data,
            metadata: FileMetadata {
                filename,
                size: Some(size),
                created_at: Some(Utc::now()),
                ..Default::default()
            },
        }
    }

    /// Creates a new in-memory file from BinaryValue
    pub fn from_binary(data: BinaryValue, filename: Option<String>) -> Self {
        let size = data.len();
        Self::InMemory {
            data,
            metadata: FileMetadata {
                filename,
                size: Some(size),
                created_at: Some(Utc::now()),
                ..Default::default()
            },
        }
    }

    /// Creates a remote file
    pub fn from_remote(
        storage_key: String,
        storage_type: StorageType,
        metadata: FileMetadata,
    ) -> Self {
        Self::Remote { storage_key, storage_type, credentials_ref: None, metadata }
    }

    /// Creates a URL file
    pub fn from_url(url: String, metadata: FileMetadata) -> Self {
        Self::Url { url, headers: None, follow_redirects: true, metadata }
    }

    /// Creates a generated file
    #[cfg(feature = "json")]
    pub fn from_generator(
        generator_id: String,
        parameters: serde_json::Value,
        metadata: FileMetadata,
    ) -> Self {
        Self::Generated {
            generator_id,
            parameters,
            cache_key: None,
            timeout_seconds: None,
            metadata,
        }
    }

    /// Creates a generated file (without JSON support)
    #[cfg(not(feature = "json"))]
    pub fn from_generator(
        generator_id: String,
        parameters: String,
        metadata: FileMetadata,
    ) -> Self {
        Self::Generated {
            generator_id,
            parameters,
            cache_key: None,
            timeout_seconds: None,
            metadata,
        }
    }

    /// Creates a temporary file
    pub fn from_temp_path(path: PathBuf, cleanup_on_drop: bool) -> Self {
        let size = std::fs::metadata(&path).ok().map(|m| m.len() as usize);
        let filename = path.file_name().and_then(|n| n.to_str()).map(String::from);

        Self::Temporary {
            path: path.to_string_lossy().into_owned(),
            cleanup_on_drop,
            owner_id: None,
            metadata: FileMetadata {
                filename,
                size,
                created_at: Some(Utc::now()),
                ..Default::default()
            },
        }
    }

    // === Access methods ===

    /// Gets the metadata for this file
    pub fn metadata(&self) -> &FileMetadata {
        match self {
            FileValue::InMemory { metadata, .. } => metadata,
            FileValue::Remote { metadata, .. } => metadata,
            FileValue::Url { metadata, .. } => metadata,
            FileValue::Generated { metadata, .. } => metadata,
            FileValue::Temporary { metadata, .. } => metadata,
            FileValue::Stream { metadata, .. } => metadata,
        }
    }

    /// Gets mutable metadata for this file
    pub fn metadata_mut(&mut self) -> &mut FileMetadata {
        match self {
            FileValue::InMemory { metadata, .. } => metadata,
            FileValue::Remote { metadata, .. } => metadata,
            FileValue::Url { metadata, .. } => metadata,
            FileValue::Generated { metadata, .. } => metadata,
            FileValue::Temporary { metadata, .. } => metadata,
            FileValue::Stream { metadata, .. } => metadata,
        }
    }

    /// Gets the filename if available
    pub fn filename(&self) -> Option<&str> {
        self.metadata().filename.as_deref()
    }

    /// Gets the MIME type if known
    pub fn mime_type(&self) -> Option<&str> {
        self.metadata().mime_type.as_deref()
    }

    /// Gets the file size if known
    pub fn size(&self) -> Option<usize> {
        match self {
            FileValue::InMemory { data, .. } => Some(data.len()),
            _ => self.metadata().size,
        }
    }

    /// Gets the binary data for InMemory files
    pub fn binary_data(&self) -> Option<&BinaryValue> {
        match self {
            FileValue::InMemory { data, .. } => Some(data),
            _ => None,
        }
    }

    /// Gets mutable binary data for InMemory files
    pub fn binary_data_mut(&mut self) -> Option<&mut BinaryValue> {
        match self {
            FileValue::InMemory { data, .. } => Some(data),
            _ => None,
        }
    }

    /// Reads the file content as BinaryValue (for InMemory files)
    pub fn read_binary(&self) -> ValueResult<BinaryValue> {
        match self {
            FileValue::InMemory { data, .. } => Ok(data.clone()),
            _ => Err(ValueError::custom(
                "Can only read binary data from InMemory files synchronously",
            )),
        }
    }

    /// Reads the file content as Vec<u8> (for InMemory files)
    pub fn read_bytes(&self) -> ValueResult<Vec<u8>> {
        match self {
            FileValue::InMemory { data, .. } => Ok(data.clone().into_bytes()),
            _ => Err(ValueError::custom("Can only read bytes from InMemory files synchronously")),
        }
    }

    /// Reads the file content as UTF-8 string (for InMemory files)
    pub fn read_string(&self) -> ValueResult<String> {
        match self {
            FileValue::InMemory { data, .. } => data.to_utf8(),
            _ => Err(ValueError::custom("Can only read string from InMemory files synchronously")),
        }
    }

    /// Checks if the file is immediately available (no I/O needed)
    pub fn is_immediately_available(&self) -> bool {
        matches!(self, FileValue::InMemory { .. })
    }

    /// Checks if the file requires network access
    pub fn requires_network(&self) -> bool {
        matches!(self, FileValue::Url { .. } | FileValue::Remote { .. })
    }

    /// Checks if the file needs to be generated
    pub fn requires_generation(&self) -> bool {
        matches!(self, FileValue::Generated { .. })
    }

    /// Checks if the file is stored locally
    pub fn is_local(&self) -> bool {
        matches!(self, FileValue::InMemory { .. } | FileValue::Temporary { .. })
    }

    /// Checks if this is likely an image file
    pub fn is_image_file(&self) -> bool {
        // First check MIME type from metadata
        if let Some(mime) = self.mime_type() {
            if mime.starts_with("image/") {
                return true;
            }
        }

        // For InMemory files, also check magic bytes
        if let Some(detected_type) = self.detect_file_type() {
            matches!(detected_type, "jpeg" | "png" | "gif" | "bmp" | "webp")
        } else {
            false
        }
    }

    /// Enhanced text file detection using both MIME type and content analysis
    pub fn is_text_file(&self) -> bool {
        // Check MIME type first
        if let Some(mime) = self.mime_type() {
            if mime.starts_with("text/") || mime.contains("json") || mime.contains("xml") {
                return true;
            }
        }

        // For InMemory files, check if content is valid UTF-8 with low entropy
        if let FileValue::InMemory { data, .. } = self {
            // Try to decode as UTF-8 and check if entropy is reasonable for text
            if data.to_utf8().is_ok() {
                let entropy = data.entropy();
                // Text typically has lower entropy than binary data
                return entropy < 6.0 && entropy > 0.0;
            }
        }

        false
    }

    /// Detects the file type based on magic bytes (for InMemory files)
    pub fn detect_file_type(&self) -> Option<&'static str> {
        match self {
            FileValue::InMemory { data, .. } => data.detect_file_type(),
            _ => None,
        }
    }

    /// Checks if the file appears to be compressed (for InMemory files)
    pub fn appears_compressed(&self) -> bool {
        match self {
            FileValue::InMemory { data, .. } => data.appears_compressed(),
            _ => false,
        }
    }

    /// Gets entropy/randomness measure (for InMemory files)
    pub fn entropy(&self) -> Option<f64> {
        match self {
            FileValue::InMemory { data, .. } => Some(data.entropy()),
            _ => None,
        }
    }

    /// Gets byte statistics (for InMemory files)
    pub fn byte_statistics(&self) -> Option<crate::types::binary::ByteStatistics> {
        match self {
            FileValue::InMemory { data, .. } => Some(data.byte_statistics()),
            _ => None,
        }
    }

    // === Modification methods ===

    /// Sets the filename
    pub fn set_filename(&mut self, filename: Option<String>) {
        self.metadata_mut().filename = filename;
    }

    /// Sets the MIME type
    pub fn set_mime_type(&mut self, mime_type: Option<String>) {
        self.metadata_mut().mime_type = mime_type;
    }

    /// Marks the file as sensitive
    pub fn mark_sensitive(&mut self) {
        self.metadata_mut().is_sensitive = true;
    }

    /// Sets a custom metadata field
    #[cfg(feature = "json")]
    pub fn set_custom_metadata(&mut self, key: String, value: serde_json::Value) {
        self.metadata_mut().custom.insert(key, value);
    }

    /// Sets a custom metadata field (string fallback)
    #[cfg(not(feature = "json"))]
    pub fn set_custom_metadata(&mut self, key: String, value: String) {
        self.metadata_mut().custom.insert(key, value);
    }

    /// Updates the generator cache key (only for Generated files)
    pub fn set_cache_key(&mut self, cache_key: Option<String>) -> ValueResult<()> {
        match self {
            FileValue::Generated { cache_key: current_key, .. } => {
                *current_key = cache_key;
                Ok(())
            },
            _ => Err(ValueError::custom("Cache key can only be set on Generated files")),
        }
    }

    /// Sets generation timeout (only for Generated files)
    pub fn set_timeout(&mut self, timeout: Option<Duration>) -> ValueResult<()> {
        match self {
            FileValue::Generated { timeout_seconds, .. } => {
                *timeout_seconds = timeout.map(|d| d.as_secs());
                Ok(())
            },
            _ => Err(ValueError::custom("Timeout can only be set on Generated files")),
        }
    }

    // === Type checking methods ===

    /// Returns the file type as a string
    pub fn file_type(&self) -> &'static str {
        match self {
            FileValue::InMemory { .. } => "in_memory",
            FileValue::Remote { .. } => "remote",
            FileValue::Url { .. } => "url",
            FileValue::Generated { .. } => "generated",
            FileValue::Temporary { .. } => "temporary",
            FileValue::Stream { .. } => "stream",
        }
    }

    /// Gets the storage type for remote files
    pub fn storage_type(&self) -> Option<&StorageType> {
        match self {
            FileValue::Remote { storage_type, .. } => Some(storage_type),
            _ => None,
        }
    }

    /// Gets the generator ID for generated files
    pub fn generator_id(&self) -> Option<&str> {
        match self {
            FileValue::Generated { generator_id, .. } => Some(generator_id),
            _ => None,
        }
    }

    /// Gets the URL for URL files
    pub fn url(&self) -> Option<&str> {
        match self {
            FileValue::Url { url, .. } => Some(url),
            _ => None,
        }
    }
}

impl fmt::Display for FileValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size_info = self.size().map(|s| format!(" ({s}B)")).unwrap_or_default();

        let filename_info = self.filename().map(|f| format!(" '{f}'")).unwrap_or_default();

        write!(f, "[{} file{}{}]", self.file_type(), filename_info, size_info)
    }
}

// === From implementations ===

impl From<Vec<u8>> for FileValue {
    fn from(data: Vec<u8>) -> Self {
        Self::from_bytes(data, None)
    }
}

impl From<BinaryValue> for FileValue {
    fn from(data: BinaryValue) -> Self {
        Self::from_binary(data, None)
    }
}

impl From<&[u8]> for FileValue {
    fn from(data: &[u8]) -> Self {
        Self::from_bytes(data.to_vec(), None)
    }
}

impl From<String> for FileValue {
    fn from(content: String) -> Self {
        Self::from_bytes(content.into_bytes(), None)
    }
}

impl From<&str> for FileValue {
    fn from(content: &str) -> Self {
        Self::from_bytes(content.as_bytes().to_vec(), None)
    }
}

// === JSON conversion (feature-gated) ===

#[cfg(feature = "json")]
impl From<FileValue> for serde_json::Value {
    fn from(file: FileValue) -> Self {
        serde_json::to_value(file).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(feature = "json")]
impl TryFrom<serde_json::Value> for FileValue {
    type Error = ValueError;

    fn try_from(value: serde_json::Value) -> ValueResult<Self> {
        serde_json::from_value(value)
            .map_err(|e| ValueError::custom(format!("JSON to FileValue conversion error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_creation() {
        let file = FileValue::from_bytes(b"Hello World".to_vec(), Some("hello.txt".to_string()));

        assert_eq!(file.filename(), Some("hello.txt"));
        assert_eq!(file.size(), Some(11));
        assert!(file.is_immediately_available());
        assert!(!file.requires_network());
        assert!(file.is_local());
    }

    #[test]
    fn test_binary_operations() {
        let data = b"Hello World";
        let file = FileValue::from_bytes(data.to_vec(), Some("hello.txt".to_string()));

        let binary = file.read_binary().unwrap();
        assert_eq!(binary.as_bytes(), data);

        let bytes = file.read_bytes().unwrap();
        assert_eq!(bytes, data);

        let string = file.read_string().unwrap();
        assert_eq!(string, "Hello World");
    }

    #[test]
    fn test_file_type_detection() {
        // Test with JPEG magic bytes
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let file = FileValue::from_bytes(jpeg_data, Some("image.jpg".to_string()));

        assert_eq!(file.detect_file_type(), Some("jpeg"));
        assert!(file.is_image_file());
        assert!(!file.is_text_file());
    }

    #[test]
    fn test_text_detection() {
        let text_data = "This is a text file with normal content.";
        let file =
            FileValue::from_bytes(text_data.as_bytes().to_vec(), Some("text.txt".to_string()));

        assert!(file.is_text_file());
        assert!(!file.is_image_file());

        if let Some(entropy) = file.entropy() {
            assert!(entropy < 6.0); // Text should have lower entropy
        }
    }

    #[cfg(all(feature = "serde", feature = "json"))]
    #[test]
    fn test_serialization() {
        let file = FileValue::from_bytes(b"Hello World".to_vec(), Some("hello.txt".to_string()));

        let json = serde_json::to_value(&file).unwrap();
        assert_eq!(json["type"], "in_memory");
        assert_eq!(json["filename"], "hello.txt");

        let deserialized: FileValue = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.filename(), Some("hello.txt"));
        assert_eq!(deserialized.size(), Some(11));
    }

    #[cfg(feature = "json")]
    #[test]
    fn test_generator_parameters() {
        let params = serde_json::json!({
            "template": "report",
            "data": {"value": 42}
        });

        let file = FileValue::from_generator(
            "pdf_generator".to_string(),
            params.clone(),
            FileMetadata::default(),
        );

        if let FileValue::Generated { parameters, .. } = file {
            assert_eq!(parameters["template"], "report");
            assert_eq!(parameters["data"]["value"], 42);
        } else {
            panic!("Expected Generated variant");
        }
    }

    #[test]
    fn test_custom_metadata() {
        let mut file = FileValue::from_bytes(b"test".to_vec(), None);

        #[cfg(feature = "json")]
        {
            file.set_custom_metadata(
                "version".to_string(),
                serde_json::Value::String("1.0".to_string()),
            );
            assert_eq!(
                file.metadata().custom.get("version"),
                Some(&serde_json::Value::String("1.0".to_string()))
            );
        }

        #[cfg(not(feature = "json"))]
        {
            file.set_custom_metadata("version".to_string(), "1.0".to_string());
            assert_eq!(file.metadata().custom.get("version"), Some(&"1.0".to_string()));
        }
    }
}
