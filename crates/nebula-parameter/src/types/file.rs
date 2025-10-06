use bon::Builder;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, ParameterValue, Validatable,
};

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
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            size: None,
            mime_type: None,
            is_temporary: false,
        }
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    pub fn as_temporary(mut self) -> Self {
        self.is_temporary = true;
        self
    }
}

/// Parameter for file uploads
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct FileParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<FileReference>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<FileReference>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<FileParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct FileParameterOptions {
    /// Accepted file formats (MIME types or extensions like ".pdf", "image/*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_formats: Option<Vec<String>>,

    /// Maximum file size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,

    /// Minimum file size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u64>,

    /// Allow multiple file selection (creates array of FileReference)
    #[serde(default)]
    pub multiple: bool,

    /// Upload directory restriction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_directory: Option<String>,

    /// Whether to validate file content (not just extension)
    #[serde(default)]
    pub validate_content: bool,
}

impl Parameter for FileParameter {
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

impl HasValue for FileParameter {
    type Value = FileReference;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }

    fn to_expression(&self) -> Option<ParameterValue> {
        self.value.as_ref().map(|file_ref| {
            // Convert FileReference to a simple string representation (path)
            ParameterValue::Value(nebula_value::Value::text(
                file_ref.path.to_string_lossy().to_string(),
            ))
        })
    }

    fn from_expression(&mut self, value: impl Into<ParameterValue>) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Text(s)) => {
                // Simple path-based file reference
                let file_ref = FileReference::new(s.as_str(), s.to_string());
                if self.is_valid_file(&file_ref)? {
                    self.value = Some(file_ref);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: "File path is not valid".to_string(),
                    })
                }
            }
            ParameterValue::Expression(expr) => {
                // Create a file reference with the expression as path
                let file_ref = FileReference::new(&expr, expr.clone());
                self.value = Some(file_ref);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for file parameter".to_string(),
            }),
        }
    }
}

impl Validatable for FileParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, _value: &Self::Value) -> bool {
        // Files are never considered "empty" since they represent a file reference
        false
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
    /// Validate if a file reference meets the parameter constraints
    fn is_valid_file(&self, file_ref: &FileReference) -> Result<bool, ParameterError> {
        // Check if path contains expression
        let path_str = file_ref.path.to_string_lossy();
        if path_str.starts_with("{{") && path_str.ends_with("}}") {
            return Ok(true); // Allow expressions
        }

        if let Some(options) = &self.options {
            // Check file size constraints
            if let Some(size) = file_ref.size {
                if let Some(max_size) = options.max_size {
                    if size > max_size {
                        return Err(ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!(
                                "File size {} bytes exceeds maximum {} bytes",
                                size, max_size
                            ),
                        });
                    }
                }
                if let Some(min_size) = options.min_size {
                    if size < min_size {
                        return Err(ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!(
                                "File size {} bytes is below minimum {} bytes",
                                size, min_size
                            ),
                        });
                    }
                }
            }

            // Check accepted formats
            if let Some(accepted_formats) = &options.accepted_formats {
                if !accepted_formats.is_empty() {
                    let is_format_accepted = self.check_file_format(file_ref, accepted_formats);
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
        }

        Ok(true)
    }

    /// Check if file format matches accepted formats
    fn check_file_format(&self, file_ref: &FileReference, accepted_formats: &[String]) -> bool {
        for format in accepted_formats {
            // Check MIME type match
            if let Some(mime_type) = &file_ref.mime_type {
                if self.mime_type_matches(mime_type, format) {
                    return true;
                }
            }

            // Check extension match
            if format.starts_with('.') {
                if let Some(extension) = file_ref.path.extension() {
                    if format[1..].eq_ignore_ascii_case(&extension.to_string_lossy()) {
                        return true;
                    }
                }
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
        if format.ends_with("/*") {
            let base_type = &format[..format.len() - 2];
            return mime_type.starts_with(base_type);
        }

        false
    }

    /// Get the file name from current value
    pub fn get_file_name(&self) -> Option<&String> {
        self.value.as_ref().map(|f| &f.name)
    }

    /// Get the file path from current value
    pub fn get_file_path(&self) -> Option<&PathBuf> {
        self.value.as_ref().map(|f| &f.path)
    }

    /// Get the file size from current value
    pub fn get_file_size(&self) -> Option<u64> {
        self.value.as_ref().and_then(|f| f.size)
    }

    /// Get the MIME type from current value
    pub fn get_mime_type(&self) -> Option<&String> {
        self.value.as_ref().and_then(|f| f.mime_type.as_ref())
    }

    /// Check if multiple files are allowed
    pub fn allows_multiple(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.multiple)
            .unwrap_or(false)
    }

    /// Get accepted file formats
    pub fn get_accepted_formats(&self) -> Option<&Vec<String>> {
        self.options
            .as_ref()
            .and_then(|opts| opts.accepted_formats.as_ref())
    }

    /// Get maximum file size
    pub fn get_max_size(&self) -> Option<u64> {
        self.options.as_ref().and_then(|opts| opts.max_size)
    }

    /// Format file size for display
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
