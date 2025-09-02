//! Validation error types

use std::fmt::{self, Display};
use serde::{Serialize, Deserialize};
use thiserror::Error;

/// Validation error with details
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct ValidationError {
    /// Error code for categorization
    code: ErrorCode,
    /// Human-readable error message
    message: String,
    /// Field path where the error occurred
    field_path: Option<String>,
    /// Severity of the error
    severity: ErrorSeverity,
    /// Additional error details
    details: Option<ErrorDetails>,
    /// Error context
    context: ErrorContext,
}

impl ValidationError {
    /// Create a new validation error
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field_path: None,
            severity: ErrorSeverity::Error,
            details: None,
            context: ErrorContext::default(),
        }
    }
    
    /// Create a warning
    pub fn warning(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field_path: None,
            severity: ErrorSeverity::Warning,
            details: None,
            context: ErrorContext::default(),
        }
    }
    
    /// Create an info message
    pub fn info(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field_path: None,
            severity: ErrorSeverity::Info,
            details: None,
            context: ErrorContext::default(),
        }
    }
    
    /// Set the field path
    pub fn with_field_path(mut self, path: impl Into<String>) -> Self {
        self.field_path = Some(path.into());
        self
    }
    
    /// Set the severity
    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }
    
    /// Add error details
    pub fn with_details(mut self, details: ErrorDetails) -> Self {
        self.details = Some(details);
        self
    }
    
    /// Add context
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.add(key.into(), value.into());
        self
    }
    
    /// Get the error code
    pub fn code(&self) -> &ErrorCode {
        &self.code
    }
    
    /// Get the message
    pub fn message(&self) -> &str {
        &self.message
    }
    
    /// Get the field path
    pub fn field_path(&self) -> Option<&str> {
        self.field_path.as_deref()
    }
    
    /// Get the severity
    pub fn severity(&self) -> &ErrorSeverity {
        &self.severity
    }
    
    /// Check if this is an error (not warning or info)
    pub fn is_error(&self) -> bool {
        matches!(self.severity, ErrorSeverity::Error)
    }
}

/// Error codes for categorization
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    // Type errors
    TypeMismatch,
    InvalidType,
    ConversionError,
    
    // Value errors
    Required,
    Forbidden,
    Invalid,
    OutOfRange,
    TooShort,
    TooLong,
    TooSmall,
    TooLarge,
    
    // Format errors
    InvalidFormat,
    PatternMismatch,
    InvalidEmail,
    InvalidUrl,
    InvalidUuid,
    InvalidDate,
    
    // Logical errors
    ConditionFailed,
    PredicateFailed,
    XorValidationFailed,
    
    // Cross-field errors
    DependencyMissing,
    ConflictingFields,
    InconsistentData,
    
    // System errors
    Timeout,
    RateLimitExceeded,
    InternalError,
    ExternalServiceError,
    
    // Proof errors
    InvalidProof,
    ProofExpired,
    SignatureFailed,
    
    // Recovery errors
    RecoveryFailed,
    TransformationFailed,
    
    // Custom errors
    Custom(String),
}

impl ErrorCode {
    /// Create a custom error code
    pub fn custom(code: impl Into<String>) -> Self {
        ErrorCode::Custom(code.into())
    }
}

impl Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::Custom(code) => write!(f, "{}", code),
            _ => write!(f, "{:?}", self),
        }
    }
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// Informational message
    Info,
    /// Warning that doesn't prevent validation
    Warning,
    /// Error that causes validation to fail
    Error,
}

impl Display for ErrorSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorSeverity::Info => write!(f, "INFO"),
            ErrorSeverity::Warning => write!(f, "WARNING"),
            ErrorSeverity::Error => write!(f, "ERROR"),
        }
    }
}

/// Additional error details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetails {
    /// Expected value
    pub expected: Option<serde_json::Value>,
    /// Actual value
    pub actual: Option<serde_json::Value>,
    /// Suggestion for fixing the error
    pub suggestion: Option<String>,
    /// Help URL for more information
    pub help_url: Option<String>,
    /// Error stack trace (for debugging)
    pub stack_trace: Option<Vec<String>>,
}

impl ErrorDetails {
    /// Create new error details
    pub fn new() -> Self {
        Self {
            expected: None,
            actual: None,
            suggestion: None,
            help_url: None,
            stack_trace: None,
        }
    }
    
    /// Set expected value
    pub fn expected(mut self, value: serde_json::Value) -> Self {
        self.expected = Some(value);
        self
    }
    
    /// Set actual value
    pub fn actual(mut self, value: serde_json::Value) -> Self {
        self.actual = Some(value);
        self
    }
    
    /// Set suggestion
    pub fn suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
    
    /// Set help URL
    pub fn help_url(mut self, url: impl Into<String>) -> Self {
        self.help_url = Some(url.into());
        self
    }
}

impl Default for ErrorDetails {
    fn default() -> Self {
        Self::new()
    }
}

/// Error context for additional information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Context key-value pairs
    context: std::collections::HashMap<String, String>,
}

impl ErrorContext {
    /// Create new context
    pub fn new() -> Self {
        Self {
            context: std::collections::HashMap::new(),
        }
    }
    
    /// Add context item
    pub fn add(&mut self, key: String, value: String) {
        self.context.insert(key, value);
    }
    
    /// Get context value
    pub fn get(&self, key: &str) -> Option<&str> {
        self.context.get(key).map(|s| s.as_str())
    }
    
    /// Check if context contains key
    pub fn contains(&self, key: &str) -> bool {
        self.context.contains_key(key)
    }
}