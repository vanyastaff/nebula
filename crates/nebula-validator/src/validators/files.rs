//! File and MIME type validation operations using the unified validator macro

use crate::{validator, validator_fn, ValueExt};

// ==================== FILE VALIDATORS ====================

validator! {
    /// Validator that checks if string matches allowed MIME types
    pub struct MimeType {
        allowed_types: Vec<String>
    }
    impl {
        fn check(value: &Value, allowed_types: &Vec<String>) -> bool {
            {
                if let Some(mime_str) = value.as_str() {
                    allowed_types.contains(&mime_str.to_string())
                } else {
                    false
                }
            }
        }
        fn error(allowed_types: &Vec<String>) -> String {
            { format!("MIME type not allowed. Allowed types: {:?}", allowed_types) }
        }
        const DESCRIPTION: &str = "String must be an allowed MIME type";
    }
}

validator! {
    /// Validator that checks if filename has allowed extensions
    pub struct FileExtension {
        allowed_extensions: Vec<String>
    }
    impl {
        fn check(value: &Value, allowed_extensions: &Vec<String>) -> bool {
            {
                if let Some(filename) = value.as_str() {
                    if let Some(ext_pos) = filename.rfind('.') {
                        let extension = &filename[ext_pos + 1..].to_lowercase();
                        allowed_extensions.iter().any(|allowed| {
                            allowed.to_lowercase() == *extension
                        })
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
        fn error(allowed_extensions: &Vec<String>) -> String {
            { format!("File extension not allowed. Allowed extensions: {:?}", allowed_extensions) }
        }
        const DESCRIPTION: &str = "Filename must have an allowed extension";
    }
}

validator! {
    /// Validator that checks file size in bytes
    pub struct FileSize {
        max_size: u64
    }
    impl {
        fn check(value: &Value, max_size: &u64) -> bool {
            {
                if let Some(size) = value.as_u64() {
                    size <= *max_size
                } else {
                    false
                }
            }
        }
        fn error(max_size: &u64) -> String {
            { format!("File size must not exceed {} bytes", max_size) }
        }
        const DESCRIPTION: &str = "File size must not exceed maximum";
    }
}

validator! {
    /// Validator that checks file size range
    pub struct FileSizeRange {
        min_size: u64,
        max_size: u64
    }
    impl {
        fn check(value: &Value, min_size: &u64, max_size: &u64) -> bool {
            {
                if let Some(size) = value.as_u64() {
                    size >= *min_size && size <= *max_size
                } else {
                    false
                }
            }
        }
        fn error(min_size: &u64, max_size: &u64) -> String {
            { format!("File size must be between {} and {} bytes", min_size, max_size) }
        }
        const DESCRIPTION: &str = "File size must be within specified range";
    }
}

validator! {
    /// Validator that checks if filename is valid (no dangerous characters)
    pub struct ValidFilename {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if let Some(filename) = value.as_str() {
                    // Check for dangerous characters
                    let dangerous_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
                    !filename.chars().any(|c| dangerous_chars.contains(&c)) &&
                    !filename.starts_with('.') &&
                    !filename.is_empty() &&
                    filename.len() <= 255
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "Filename contains invalid characters or is too long".to_string() }
        }
        const DESCRIPTION: &str = "Filename must be valid and safe";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn mime_type(allowed_types: Vec<String>) -> MimeType);
validator_fn!(pub fn file_extension(allowed_extensions: Vec<String>) -> FileExtension);
validator_fn!(pub fn file_size(max_size: u64) -> FileSize);
validator_fn!(pub fn file_size_range(min_size: u64, max_size: u64) -> FileSizeRange);
validator_fn!(pub fn valid_filename() -> ValidFilename);

// String-specific convenience functions with &str input
pub fn mime_types(allowed_types: Vec<&str>) -> MimeType {
    MimeType::new(allowed_types.into_iter().map(|s| s.to_string()).collect())
}

pub fn file_extensions(allowed_extensions: Vec<&str>) -> FileExtension {
    FileExtension::new(allowed_extensions.into_iter().map(|s| s.to_string()).collect())
}

// Common MIME type convenience functions
pub fn image_files() -> MimeType {
    mime_types(vec!["image/jpeg", "image/png", "image/gif", "image/webp"])
}

pub fn document_files() -> MimeType {
    mime_types(vec!["application/pdf", "application/msword", "text/plain"])
}

pub fn video_files() -> MimeType {
    mime_types(vec!["video/mp4", "video/mpeg", "video/quicktime"])
}

pub fn audio_files() -> MimeType {
    mime_types(vec!["audio/mpeg", "audio/wav", "audio/ogg"])
}

// Common file extension convenience functions
pub fn image_extensions() -> FileExtension {
    file_extensions(vec!["jpg", "jpeg", "png", "gif", "webp"])
}

pub fn document_extensions() -> FileExtension {
    file_extensions(vec!["pdf", "doc", "docx", "txt"])
}

pub fn video_extensions() -> FileExtension {
    file_extensions(vec!["mp4", "avi", "mov", "mkv"])
}

pub fn audio_extensions() -> FileExtension {
    file_extensions(vec!["mp3", "wav", "ogg", "flac"])
}

// File size convenience functions
pub fn max_file_size_mb(megabytes: u64) -> FileSize {
    FileSize::new(megabytes * 1024 * 1024)
}

pub fn max_file_size_kb(kilobytes: u64) -> FileSize {
    FileSize::new(kilobytes * 1024)
}