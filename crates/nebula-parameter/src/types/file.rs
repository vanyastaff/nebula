//! File parameter type for file uploads

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Represents a file reference with metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileReference {
    /// File path (can be relative or absolute)
    pub path: PathBuf,

    /// Original filename
    pub name: String,

    /// File size in bytes
    pub size: Option<u64>,

    /// MIME type
    pub mime_type: Option<String>,

    /// Whether this is a temporary file that should be cleaned up
    pub is_temporary: bool,
}

impl From<FileReference> for nebula_value::Value {
    fn from(file_ref: FileReference) -> Self {
        use crate::ValueRefExt;
        let mut obj = serde_json::Map::new();
        obj.insert(
            "path".to_string(),
            nebula_value::Value::text(file_ref.path.to_string_lossy().to_string()).to_json(),
        );
        obj.insert(
            "name".to_string(),
            nebula_value::Value::text(file_ref.name).to_json(),
        );
        if let Some(size) = file_ref.size {
            obj.insert(
                "size".to_string(),
                nebula_value::Value::integer(size as i64).to_json(),
            );
        }
        if let Some(mime_type) = file_ref.mime_type {
            obj.insert(
                "mime_type".to_string(),
                nebula_value::Value::text(mime_type).to_json(),
            );
        }
        obj.insert(
            "is_temporary".to_string(),
            nebula_value::Value::boolean(file_ref.is_temporary).to_json(),
        );

        use crate::JsonValueExt;
        serde_json::Value::Object(obj)
            .to_nebula_value()
            .unwrap_or(nebula_value::Value::Null)
    }
}

impl FileReference {
    /// Create a new file reference
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            size: None,
            mime_type: None,
            is_temporary: false,
        }
    }

    /// Set file size
    #[must_use = "builder methods must be chained or built"]
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Set MIME type
    #[must_use = "builder methods must be chained or built"]
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Mark as temporary file
    #[must_use = "builder methods must be chained or built"]
    pub fn as_temporary(mut self) -> Self {
        self.is_temporary = true;
        self
    }
}

/// Parameter for file uploads
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = FileParameter::builder()
///     .key("document")
///     .name("Document")
///     .description("Upload a document")
///     .required(true)
///     .options(
///         FileParameterOptions::builder()
///             .accepted_formats(vec![".pdf", ".docx", "application/pdf"])
///             .max_size(10 * 1024 * 1024) // 10 MB
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<FileReference>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<FileParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for file parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileParameterOptions {
    /// Accepted file formats (MIME types or extensions like ".pdf", "image/*")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_formats: Option<Vec<String>>,

    /// Maximum file size in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,

    /// Minimum file size in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u64>,

    /// Allow multiple file selection (creates array of `FileReference`)
    #[serde(default)]
    pub multiple: bool,

    /// Upload directory restriction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_directory: Option<String>,

    /// Whether to validate file content (not just extension)
    #[serde(default)]
    pub validate_content: bool,
}

// =============================================================================
// FileParameter Builder
// =============================================================================

/// Builder for `FileParameter`
#[derive(Debug, Default)]
pub struct FileParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<FileReference>,
    options: Option<FileParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl FileParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> FileParameterBuilder {
        FileParameterBuilder::new()
    }
}

impl FileParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            default: None,
            options: None,
            display: None,
            validation: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: FileReference) -> Self {
        self.default = Some(default);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: FileParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `FileParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<FileParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description)
            .required(self.required)
            .build()?;

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(FileParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// FileParameterOptions Builder
// =============================================================================

/// Builder for `FileParameterOptions`
#[derive(Debug, Default)]
pub struct FileParameterOptionsBuilder {
    accepted_formats: Option<Vec<String>>,
    max_size: Option<u64>,
    min_size: Option<u64>,
    multiple: bool,
    upload_directory: Option<String>,
    validate_content: bool,
}

impl FileParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> FileParameterOptionsBuilder {
        FileParameterOptionsBuilder::default()
    }
}

impl FileParameterOptionsBuilder {
    /// Set accepted file formats
    #[must_use]
    pub fn accepted_formats(
        mut self,
        formats: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.accepted_formats = Some(formats.into_iter().map(Into::into).collect());
        self
    }

    /// Set maximum file size in bytes
    #[must_use]
    pub fn max_size(mut self, max_size: u64) -> Self {
        self.max_size = Some(max_size);
        self
    }

    /// Set minimum file size in bytes
    #[must_use]
    pub fn min_size(mut self, min_size: u64) -> Self {
        self.min_size = Some(min_size);
        self
    }

    /// Set whether to allow multiple files
    #[must_use]
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Set upload directory restriction
    #[must_use]
    pub fn upload_directory(mut self, upload_directory: impl Into<String>) -> Self {
        self.upload_directory = Some(upload_directory.into());
        self
    }

    /// Set whether to validate file content
    #[must_use]
    pub fn validate_content(mut self, validate_content: bool) -> Self {
        self.validate_content = validate_content;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> FileParameterOptions {
        FileParameterOptions {
            accepted_formats: self.accepted_formats,
            max_size: self.max_size,
            min_size: self.min_size,
            multiple: self.multiple,
            upload_directory: self.upload_directory,
            validate_content: self.validate_content,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for FileParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::File
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for FileParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileParameter({})", self.metadata.name)
    }
}

impl Validatable for FileParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Object)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        let obj = match value {
            Value::Object(o) => o,
            Value::Null => return Ok(()), // Null is allowed for optional
            _ => return Ok(()),           // Type error already handled above
        };

        // Required check
        if self.is_empty(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Extract file reference data from object
        let path_value = obj
            .get("path")
            .ok_or_else(|| ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "File object missing 'path' field".to_string(),
            })?;

        let path_str = match path_value {
            Value::Text(t) => t.as_str(),
            _ => {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: "File 'path' field must be text".to_string(),
                });
            }
        };

        // Check if path contains expression (allow expressions)
        if path_str.starts_with("{{") && path_str.ends_with("}}") {
            return Ok(());
        }

        // Validate file constraints
        if let Some(options) = &self.options {
            // Check file size constraints
            if let Some(size_value) = obj.get("size")
                && let Value::Integer(num) = size_value
                && let Ok(size) = u64::try_from(num.value())
            {
                if let Some(max_size) = options.max_size
                    && size > max_size
                {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("File size {size} bytes exceeds maximum {max_size} bytes"),
                    });
                }
                if let Some(min_size) = options.min_size
                    && size < min_size
                {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("File size {size} bytes is below minimum {min_size} bytes"),
                    });
                }
            }

            // Check accepted formats
            if let Some(accepted_formats) = &options.accepted_formats
                && !accepted_formats.is_empty()
            {
                let mime_type = obj.get("mime_type").and_then(|v| {
                    if let Value::Text(t) = v {
                        Some(t.as_str())
                    } else {
                        None
                    }
                });

                let path = PathBuf::from(path_str);
                let extension = path.extension().and_then(|e| e.to_str());

                let is_format_accepted =
                    self.check_file_format(mime_type, extension, accepted_formats);

                if !is_format_accepted {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "File format not accepted. Accepted formats: {}",
                            accepted_formats.join(", ")
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        // Files are never considered "empty" if they have a valid object structure
        !matches!(value, Value::Object(_))
    }
}

impl Displayable for FileParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl FileParameter {
    /// Check if file format matches accepted formats
    fn check_file_format(
        &self,
        mime_type: Option<&str>,
        extension: Option<&str>,
        accepted_formats: &[String],
    ) -> bool {
        for format in accepted_formats {
            // Check MIME type match
            if let Some(mime) = mime_type
                && self.mime_type_matches(mime, format)
            {
                return true;
            }

            // Check extension match
            if format.starts_with('.')
                && let Some(ext) = extension
                && format[1..].eq_ignore_ascii_case(ext)
            {
                return true;
            }
        }
        false
    }

    /// Check if MIME type matches a format pattern
    fn mime_type_matches(&self, mime_type: &str, format: &str) -> bool {
        if format == mime_type {
            return true;
        }

        // Handle wildcard patterns like "image/*"
        if let Some(base_type) = format.strip_suffix("/*") {
            return mime_type.starts_with(base_type);
        }

        false
    }

    /// Get the file name from a value
    #[must_use]
    pub fn get_file_name(value: &Value) -> Option<String> {
        if let Value::Object(obj) = value {
            obj.get("name").and_then(|v| {
                if let Value::Text(t) = v {
                    Some(t.to_string())
                } else {
                    None
                }
            })
        } else {
            None
        }
    }

    /// Get the file path from a value
    #[must_use]
    pub fn get_file_path(value: &Value) -> Option<PathBuf> {
        if let Value::Object(obj) = value {
            obj.get("path").and_then(|v| {
                if let Value::Text(t) = v {
                    Some(PathBuf::from(t.as_str()))
                } else {
                    None
                }
            })
        } else {
            None
        }
    }

    /// Get the file size from a value
    #[must_use]
    pub fn get_file_size(value: &Value) -> Option<u64> {
        if let Value::Object(obj) = value {
            obj.get("size").and_then(|v| {
                if let Value::Integer(n) = v {
                    u64::try_from(n.value()).ok()
                } else {
                    None
                }
            })
        } else {
            None
        }
    }

    /// Get the MIME type from a value
    #[must_use]
    pub fn get_mime_type(value: &Value) -> Option<String> {
        if let Value::Object(obj) = value {
            obj.get("mime_type").and_then(|v| {
                if let Value::Text(t) = v {
                    Some(t.to_string())
                } else {
                    None
                }
            })
        } else {
            None
        }
    }

    /// Check if multiple files are allowed
    #[must_use]
    pub fn allows_multiple(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.multiple)
    }

    /// Get accepted file formats
    #[must_use]
    pub fn get_accepted_formats(&self) -> Option<&Vec<String>> {
        self.options
            .as_ref()
            .and_then(|opts| opts.accepted_formats.as_ref())
    }

    /// Get maximum file size
    #[must_use]
    pub fn get_max_size(&self) -> Option<u64> {
        self.options.as_ref().and_then(|opts| opts.max_size)
    }

    /// Format file size for display
    #[must_use]
    pub fn format_file_size(size_bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        let mut size = size_bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", size_bytes, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_parameter_builder() {
        let param = FileParameter::builder()
            .key("document")
            .name("Document")
            .description("Upload a document")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "document");
        assert_eq!(param.metadata.name, "Document");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_file_parameter_with_options() {
        let param = FileParameter::builder()
            .key("image")
            .name("Image")
            .options(
                FileParameterOptions::builder()
                    .accepted_formats([".png", ".jpg", "image/*"])
                    .max_size(5 * 1024 * 1024)
                    .multiple(true)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.accepted_formats.as_ref().unwrap().len(), 3);
        assert_eq!(opts.max_size, Some(5 * 1024 * 1024));
        assert!(opts.multiple);
    }

    #[test]
    fn test_file_parameter_missing_key() {
        let result = FileParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_file_reference() {
        let file_ref = FileReference::new("/path/to/file.pdf", "file.pdf")
            .with_size(1024)
            .with_mime_type("application/pdf")
            .as_temporary();

        assert_eq!(file_ref.path, PathBuf::from("/path/to/file.pdf"));
        assert_eq!(file_ref.name, "file.pdf");
        assert_eq!(file_ref.size, Some(1024));
        assert_eq!(file_ref.mime_type, Some("application/pdf".to_string()));
        assert!(file_ref.is_temporary);
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(FileParameter::format_file_size(500), "500 B");
        assert_eq!(FileParameter::format_file_size(1024), "1.0 KB");
        assert_eq!(FileParameter::format_file_size(1536), "1.5 KB");
        assert_eq!(FileParameter::format_file_size(1048576), "1.0 MB");
    }
}
