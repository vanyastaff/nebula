//! Core types for the Nebula validation framework
//! 
//! This module contains all the fundamental types used throughout the validation system,
//! including metadata, configuration, and result types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

// ==================== Core Types ====================

/// Unique identifier for a validator instance
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValidatorId(pub String);

impl ValidatorId {
    /// Create a new validator ID
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    
    /// Get the ID as a string reference
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validation result with detailed information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult<T> {
    /// Whether validation passed
    pub is_valid: bool,
    /// The validated value if successful
    pub value: Option<T>,
    /// Collection of validation errors
    pub errors: Vec<ValidationError>,
    /// Validation metadata
    pub metadata: ValidationMetadata,
}

impl<T> ValidationResult<T> {
    /// Create a successful validation result
    pub fn success(value: T) -> Self {
        Self {
            is_valid: true,
            value: Some(value),
            errors: Vec::new(),
            metadata: ValidationMetadata::default(),
        }
    }
    
    /// Create a failed validation result
    pub fn failure(errors: Vec<ValidationError>) -> Self {
        Self {
            is_valid: false,
            value: None,
            errors,
            metadata: ValidationMetadata::default(),
        }
    }
    
    /// Check if validation passed
    pub fn is_success(&self) -> bool {
        self.is_valid
    }
    
    /// Check if validation failed
    pub fn is_failure(&self) -> bool {
        !self.is_valid
    }
    
    /// Get the first error if validation failed
    pub fn first_error(&self) -> Option<&ValidationError> {
        self.errors.first()
    }
    
    /// Convert to a Result
    pub fn into_result(self) -> Result<T, Vec<ValidationError>> {
        if self.is_valid {
            self.value.ok_or_else(|| {
                vec![ValidationError::new(
                    ErrorCode::InternalError,
                    "Validation succeeded but no value returned"
                )]
            })
        } else {
            Err(self.errors)
        }
    }
}

// ==================== Metadata Types ====================

/// Validator metadata for observability and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorMetadata {
    /// Unique identifier for the validator
    pub id: ValidatorId,
    /// Human-readable name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Category classification
    pub category: ValidatorCategory,
    /// Tags for grouping and filtering
    pub tags: Vec<String>,
    /// Version information
    pub version: String,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp
    pub modified_at: DateTime<Utc>,
}

impl ValidatorMetadata {
    /// Create new metadata
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        category: ValidatorCategory,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: ValidatorId(id.into()),
            name: name.into(),
            description: None,
            category,
            tags: Vec::new(),
            version: "1.0.0".to_string(),
            created_at: now,
            modified_at: now,
        }
    }
    
    /// Add a description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    
    /// Add tags
    pub fn with_tags(mut self, tags: impl Into<Vec<String>>) -> Self {
        self.tags = tags.into();
        self
    }
    
    /// Set version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
}

/// Validation metadata for tracking execution details
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidationMetadata {
    /// Execution timestamp
    pub executed_at: DateTime<Utc>,
    /// Execution duration in nanoseconds
    pub duration_ns: u64,
    /// Number of validation rules applied
    pub rules_applied: usize,
    /// Cache hit/miss information
    pub cache_info: CacheInfo,
    /// Performance metrics
    pub performance: PerformanceMetrics,
}

impl ValidationMetadata {
    /// Create new metadata with current timestamp
    pub fn new() -> Self {
        Self {
            executed_at: Utc::now(),
            duration_ns: 0,
            rules_applied: 0,
            cache_info: CacheInfo::default(),
            performance: PerformanceMetrics::default(),
        }
    }
    
    /// Set execution duration
    pub fn with_duration(mut self, duration_ns: u64) -> Self {
        self.duration_ns = duration_ns;
        self
    }
    
    /// Set rules applied count
    pub fn with_rules_applied(mut self, count: usize) -> Self {
        self.rules_applied = count;
        self
    }
}

/// Cache information for validation results
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheInfo {
    /// Whether result was retrieved from cache
    pub cache_hit: bool,
    /// Cache key used
    pub cache_key: Option<String>,
    /// Cache TTL in seconds
    pub ttl_seconds: Option<u64>,
}

/// Performance metrics for validation operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceMetrics {
    /// Memory usage in bytes
    pub memory_bytes: u64,
    /// CPU time in nanoseconds
    pub cpu_time_ns: u64,
    /// I/O operations count
    pub io_operations: u64,
}

// ==================== Category and Complexity Types ====================

/// Validator category classification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValidatorCategory {
    /// Basic validation operations
    Basic,
    /// Logical combinations
    Logical,
    /// Cross-field validation
    CrossField,
    /// Conditional validation
    Conditional,
    /// Array validation
    Array,
    /// Object validation
    Object,
    /// Collection validation
    Collection,
    /// Format validation
    Format,
    /// Custom validation logic
    Custom,
    /// Composite validators
    Composite,
    /// Pipeline validators
    Pipeline,
}

impl ValidatorCategory {
    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Logical => "Logical",
            Self::CrossField => "Cross-Field",
            Self::Conditional => "Conditional",
            Self::Array => "Array",
            Self::Object => "Object",
            Self::Collection => "Collection",
            Self::Format => "Format",
            Self::Custom => "Custom",
            Self::Composite => "Composite",
            Self::Pipeline => "Pipeline",
        }
    }
    
    /// Get description
    pub fn description(&self) -> &'static str {
        match self {
            Self::Basic => "Basic validation operations like null checks and type validation",
            Self::Logical => "Logical combinations of other validators",
            Self::CrossField => "Validation across multiple fields",
            Self::Conditional => "Conditional validation based on field values",
            Self::Array => "Array-specific validation operations",
            Self::Object => "Object and nested structure validation",
            Self::Collection => "General collection validation",
            Self::Format => "Format and pattern validation",
            Self::Custom => "Custom validation logic",
            Self::Composite => "Combined validation operations",
            Self::Pipeline => "Sequential validation pipeline",
        }
    }
}

/// Validation complexity levels for optimization
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ValidationComplexity {
    /// Simple validation (O(1) or O(n))
    Simple = 1,
    /// Moderate complexity (O(n log n))
    Moderate = 2,
    /// Complex validation (O(n²) or multiple passes)
    Complex = 3,
    /// Very complex (O(n³) or exponential)
    VeryComplex = 4,
}

impl ValidationComplexity {
    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Simple => "Simple",
            Self::Moderate => "Moderate",
            Self::Complex => "Complex",
            Self::VeryComplex => "Very Complex",
        }
    }
    
    /// Get estimated time complexity
    pub fn time_complexity(&self) -> &'static str {
        match self {
            Self::Simple => "O(1) - O(n)",
            Self::Moderate => "O(n log n)",
            Self::Complex => "O(n²)",
            Self::VeryComplex => "O(n³) or higher",
        }
    }
}

// ==================== Configuration Types ====================

/// Validation configuration options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Whether to enable caching
    pub enable_caching: bool,
    /// Cache TTL in seconds
    pub cache_ttl_seconds: u64,
    /// Maximum validation depth for nested structures
    pub max_depth: usize,
    /// Whether to collect detailed metadata
    pub collect_metadata: bool,
    /// Performance budget in milliseconds
    pub performance_budget_ms: u64,
    /// Custom configuration options
    pub custom_options: HashMap<String, serde_json::Value>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            enable_caching: true,
            cache_ttl_seconds: 300, // 5 minutes
            max_depth: 10,
            collect_metadata: true,
            performance_budget_ms: 1000, // 1 second
            custom_options: HashMap::new(),
        }
    }
}

impl ValidationConfig {
    /// Create new configuration
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Disable caching
    pub fn without_caching(mut self) -> Self {
        self.enable_caching = false;
        self
    }
    
    /// Set cache TTL
    pub fn with_cache_ttl(mut self, ttl_seconds: u64) -> Self {
        self.cache_ttl_seconds = ttl_seconds;
        self
    }
    
    /// Set maximum depth
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }
    
    /// Set performance budget
    pub fn with_performance_budget(mut self, budget_ms: u64) -> Self {
        self.performance_budget_ms = budget_ms;
        self
    }
    
    /// Add custom option
    pub fn with_custom_option(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.custom_options.insert(key.into(), value);
        self
    }
}

// ==================== Error Types ====================

/// Validation error with detailed information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Error code
    pub code: ErrorCode,
    /// Human-readable error message
    pub message: String,
    /// Field path where error occurred
    pub field_path: Option<String>,
    /// Actual value that failed validation
    pub actual_value: Option<serde_json::Value>,
    /// Expected value or constraint
    pub expected_value: Option<serde_json::Value>,
    /// Rule name that failed
    pub rule_name: Option<String>,
    /// Suggestion for fixing the error
    pub suggestion: Option<String>,
    /// Error severity level
    pub severity: ErrorSeverity,
    /// Timestamp when error occurred
    pub timestamp: DateTime<Utc>,
    /// Additional context information
    pub context: HashMap<String, serde_json::Value>,
}

impl ValidationError {
    /// Create new validation error
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field_path: None,
            actual_value: None,
            expected_value: None,
            rule_name: None,
            suggestion: None,
            severity: ErrorSeverity::Error,
            timestamp: Utc::now(),
            context: HashMap::new(),
        }
    }
    
    /// Set field path
    pub fn with_field_path(mut self, path: impl Into<String>) -> Self {
        self.field_path = Some(path.into());
        self
    }
    
    /// Set actual value
    pub fn with_actual_value(mut self, value: serde_json::Value) -> Self {
        self.actual_value = Some(value);
        self
    }
    
    /// Set expected value
    pub fn with_expected_value(mut self, value: serde_json::Value) -> Self {
        self.expected_value = Some(value);
        self
    }
    
    /// Set rule name
    pub fn with_rule_name(mut self, name: impl Into<String>) -> Self {
        self.rule_name = Some(name.into());
        self
    }
    
    /// Set suggestion
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
    
    /// Set severity
    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }
    
    /// Add context information
    pub fn with_context(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.context.insert(key.into(), value);
        self
    }
}

/// Error codes for different validation failures
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    /// Type mismatch error
    TypeMismatch,
    /// Value out of range
    ValueOutOfRange,
    /// Required field missing
    RequiredFieldMissing,
    /// Invalid format
    InvalidFormat,
    /// Cross-field validation failed
    CrossFieldValidationFailed,
    /// Conditional validation failed
    ConditionalValidationFailed,
    /// Collection validation failed
    CollectionValidationFailed,
    /// Custom validation error
    Custom(String),
    /// Internal error
    InternalError,
    /// Timeout error
    Timeout,
    /// Resource limit exceeded
    ResourceLimitExceeded,
}

/// Error severity levels
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// Information level
    Info = 0,
    /// Warning level
    Warning = 1,
    /// Error level
    Error = 2,
    /// Critical error level
    Critical = 3,
}

impl ErrorSeverity {
    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Critical => "Critical",
        }
    }
    
    /// Check if error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Info | Self::Warning)
    }
}

// ==================== Re-exports ====================

pub use ValidationResult as Result;
pub use ValidationError as Error;
pub use ValidationConfig as Config;
pub use ValidatorMetadata as Metadata;
