//! Metadata types for validators and validations

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use super::ValidatorId;

/// Metadata for a validator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorMetadata {
    /// Unique identifier
    pub id: ValidatorId,
    /// Human-readable name
    pub name: String,
    /// Description of what the validator does
    pub description: Option<String>,
    /// Category for organization
    pub category: ValidatorCategory,
    /// Version of the validator
    pub version: String,
    /// Tags for searching and filtering
    pub tags: Vec<String>,
    /// Author information
    pub author: Option<String>,
    /// When the validator was created
    pub created_at: DateTime<Utc>,
    /// When the validator was last updated
    pub updated_at: DateTime<Utc>,
    /// Whether the validator is deprecated
    pub deprecated: bool,
    /// Custom metadata
    pub custom: HashMap<String, serde_json::Value>,
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
            id: ValidatorId::new(id),
            name: name.into(),
            description: None,
            category,
            version: "1.0.0".to_string(),
            tags: Vec::new(),
            author: None,
            created_at: now,
            updated_at: now,
            deprecated: false,
            custom: HashMap::new(),
        }
    }
    
    /// Set description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
    
    /// Set version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
    
    /// Add tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
    
    /// Add a single tag
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
    
    /// Set author
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
    
    /// Mark as deprecated
    pub fn deprecated(mut self) -> Self {
        self.deprecated = true;
        self
    }
}

/// Validator categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValidatorCategory {
    /// Basic validators (type checks, null checks)
    Basic,
    /// String validators
    String,
    /// Numeric validators
    Numeric,
    /// Boolean validators
    Boolean,
    /// Array/collection validators
    Array,
    /// Object validators
    Object,
    /// Date/time validators
    DateTime,
    /// Format validators (email, URL, etc.)
    Format,
    /// Logical combinators (AND, OR, NOT)
    Logical,
    /// Conditional validators (When, If)
    Conditional,
    /// Cross-field validators
    CrossField,
    /// Custom validators
    Custom,
    /// Performance validators (memoized, cached)
    Performance,
    /// Security validators
    Security,
    /// Transform validators
    Transform,
    /// Type validators
    Type,
}

impl ValidatorCategory {
    /// Get all categories
    pub fn all() -> Vec<Self> {
        vec![
            Self::Basic,
            Self::String,
            Self::Numeric,
            Self::Boolean,
            Self::Array,
            Self::Object,
            Self::DateTime,
            Self::Format,
            Self::Logical,
            Self::Conditional,
            Self::CrossField,
            Self::Custom,
            Self::Performance,
            Self::Security,
            Self::Transform,
            Self::Type,
        ]
    }
}

/// Metadata for a validation execution
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationMetadata {
    /// When the validation started
    pub started_at: Option<DateTime<Utc>>,
    /// When the validation completed
    pub completed_at: Option<DateTime<Utc>>,
    /// Duration of the validation
    pub duration_ms: Option<u64>,
    /// Number of validators executed
    pub validators_executed: usize,
    /// Number of rules evaluated
    pub rules_evaluated: usize,
    /// Performance metrics
    pub performance: PerformanceMetrics,
    /// Validation statistics
    pub stats: ValidationStats,
}

impl ValidationMetadata {
    /// Start timing the validation
    pub fn start(&mut self) {
        self.started_at = Some(Utc::now());
    }
    
    /// Complete the validation
    pub fn complete(&mut self) {
        let now = Utc::now();
        self.completed_at = Some(now);
        
        if let Some(started) = self.started_at {
            let duration = now.signed_duration_since(started);
            self.duration_ms = Some(duration.num_milliseconds() as u64);
        }
    }
    
    /// Record a validator execution
    pub fn record_validator(&mut self) {
        self.validators_executed += 1;
    }
    
    /// Record a rule evaluation
    pub fn record_rule(&mut self) {
        self.rules_evaluated += 1;
    }
}

/// Performance metrics for validation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// CPU time used (microseconds)
    pub cpu_time_us: Option<u64>,
    /// Memory allocated (bytes)
    pub memory_bytes: Option<u64>,
    /// Cache hits
    pub cache_hits: u64,
    /// Cache misses
    pub cache_misses: u64,
    /// Number of async operations
    pub async_operations: u64,
    /// Number of I/O operations
    pub io_operations: u64,
}

impl PerformanceMetrics {
    /// Calculate cache hit rate
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
}

/// Validation statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationStats {
    /// Total validations performed
    pub total_validations: u64,
    /// Successful validations
    pub successful_validations: u64,
    /// Failed validations
    pub failed_validations: u64,
    /// Warnings generated
    pub warnings_count: u64,
    /// Info messages generated
    pub info_count: u64,
}

impl ValidationStats {
    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_validations == 0 {
            0.0
        } else {
            self.successful_validations as f64 / self.total_validations as f64
        }
    }
    
    /// Record a successful validation
    pub fn record_success(&mut self) {
        self.total_validations += 1;
        self.successful_validations += 1;
    }
    
    /// Record a failed validation
    pub fn record_failure(&mut self) {
        self.total_validations += 1;
        self.failed_validations += 1;
    }
}

/// Builder for ValidatorMetadata
pub struct MetadataBuilder {
    metadata: ValidatorMetadata,
}

impl MetadataBuilder {
    /// Create a new builder
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ValidatorMetadata::new(
                id,
                name,
                ValidatorCategory::Custom,
            ),
        }
    }
    
    /// Set category
    pub fn category(mut self, category: ValidatorCategory) -> Self {
        self.metadata.category = category;
        self
    }
    
    /// Set description
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.metadata.description = Some(description.into());
        self
    }
    
    /// Add tags
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.metadata.tags = tags;
        self
    }
    
    /// Build the metadata
    pub fn build(self) -> ValidatorMetadata {
        self.metadata
    }
}