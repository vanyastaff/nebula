use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

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

    #[must_use = "builder methods must be chained or built"]
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn as_temporary(mut self) -> Self {
        self.is_temporary = true;
        self
    }
}

/// Parameter for file uploads
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct FileParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<FileReference>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<FileParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
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

    /// Allow multiple file selection (creates array of `FileReference`)
    #[builder(default)]
    #[serde(default)]
    pub multiple: bool,

    /// Upload directory restriction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_directory: Option<String>,

    /// Whether to validate file content (not just extension)
    #[builder(default)]
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

impl Validatable for FileParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check - expect an object representing FileReference
        let obj = match value {
            Value::Object(o) => o,
            _ => {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Expected object value for file, got {}", value.kind()),
                });
            }
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
