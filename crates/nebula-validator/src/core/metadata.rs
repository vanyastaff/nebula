//! Validator metadata for introspection and optimization
//!
//! This module provides metadata types that validators can expose to enable:
//! - Runtime introspection
//! - Performance optimization
//! - Documentation generation
//! - UI generation

use std::time::Duration;

// ============================================================================
// VALIDATOR METADATA
// ============================================================================

/// Metadata about a validator for introspection and optimization.
///
/// This information can be used to:
/// - Generate documentation
/// - Optimize validation order (cheap validators first)
/// - Build user interfaces
/// - Cache validation results
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::core::{ValidatorMetadata, ValidationComplexity};
///
/// let metadata = ValidatorMetadata::builder()
///     .name("MinLength")
///     .description("Validates minimum string length")
///     .complexity(ValidationComplexity::Constant)
///     .cacheable(true)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct ValidatorMetadata {
    /// Human-readable name of the validator.
    pub name: String,

    /// Optional description of what the validator does.
    pub description: Option<String>,

    /// Computational complexity of the validation.
    pub complexity: ValidationComplexity,

    /// Whether validation results can be safely cached.
    pub cacheable: bool,

    /// Estimated average execution time.
    pub estimated_time: Option<Duration>,

    /// Tags for categorization.
    pub tags: Vec<String>,

    /// Version of the validator (for tracking changes).
    pub version: Option<String>,

    /// Additional custom metadata.
    pub custom: std::collections::HashMap<String, String>,
}

impl Default for ValidatorMetadata {
    fn default() -> Self {
        Self {
            name: "Unknown".to_string(),
            description: None,
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: Vec::new(),
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

impl ValidatorMetadata {
    /// Creates a new metadata builder.
    #[must_use]
    pub fn builder() -> ValidatorMetadataBuilder {
        ValidatorMetadataBuilder::default()
    }

    /// Creates simple metadata with just a name.
    pub fn simple(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Creates metadata with name and description.
    pub fn with_description(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
            ..Default::default()
        }
    }

    /// Adds a tag to the metadata.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Adds custom metadata.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_custom(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom.insert(key.into(), value.into());
        self
    }
}

// ============================================================================
// VALIDATION COMPLEXITY
// ============================================================================

/// Computational complexity classification for validators.
///
/// This helps optimize validation order by running cheap validators first.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::core::ValidationComplexity;
///
/// // O(1) - checking if a value is null
/// let complexity = ValidationComplexity::Constant;
///
/// // O(n) - checking string length
/// let complexity = ValidationComplexity::Linear;
///
/// // O(n²) or async operations - regex, database checks
/// let complexity = ValidationComplexity::Expensive;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ValidationComplexity {
    /// O(1) - constant time operations.
    ///
    /// Examples: null checks, type checks, simple comparisons
    #[default]
    Constant,

    /// O(n) - linear time operations.
    ///
    /// Examples: string length checks, array size checks
    Linear,

    /// O(n log n) - logarithmic operations.
    ///
    /// Examples: sorted array checks
    Logarithmic,

    /// O(n²) or worse, or involves I/O.
    ///
    /// Examples: regex matching, database lookups, API calls
    Expensive,
}

impl ValidationComplexity {
    /// Returns a numeric score for comparison (lower is cheaper).
    #[must_use]
    pub fn score(&self) -> u8 {
        match self {
            Self::Constant => 1,
            Self::Linear => 2,
            Self::Logarithmic => 3,
            Self::Expensive => 4,
        }
    }

    /// Returns true if this complexity is more expensive than another.
    #[must_use]
    pub fn is_more_expensive_than(&self, other: &Self) -> bool {
        self.score() > other.score()
    }
}

impl fmt::Display for ValidationComplexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Constant => write!(f, "O(1)"),
            Self::Linear => write!(f, "O(n)"),
            Self::Logarithmic => write!(f, "O(n log n)"),
            Self::Expensive => write!(f, "O(n²) or I/O"),
        }
    }
}

use std::fmt;

// ============================================================================
// METADATA BUILDER
// ============================================================================

/// Builder for creating validator metadata.
#[derive(Default)]
pub struct ValidatorMetadataBuilder {
    name: Option<String>,
    description: Option<String>,
    complexity: ValidationComplexity,
    cacheable: bool,
    estimated_time: Option<Duration>,
    tags: Vec<String>,
    version: Option<String>,
    custom: std::collections::HashMap<String, String>,
}

impl ValidatorMetadataBuilder {
    /// Sets the validator name.
    #[must_use = "builder methods must be chained or built"]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the description.
    #[must_use = "builder methods must be chained or built"]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the complexity.
    #[must_use = "builder methods must be chained or built"]
    pub fn complexity(mut self, complexity: ValidationComplexity) -> Self {
        self.complexity = complexity;
        self
    }

    /// Sets whether the validator is cacheable.
    #[must_use = "builder methods must be chained or built"]
    pub fn cacheable(mut self, cacheable: bool) -> Self {
        self.cacheable = cacheable;
        self
    }

    /// Sets the estimated execution time.
    #[must_use = "builder methods must be chained or built"]
    pub fn estimated_time(mut self, time: Duration) -> Self {
        self.estimated_time = Some(time);
        self
    }

    /// Adds a tag.
    #[must_use = "builder methods must be chained or built"]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Adds multiple tags.
    #[must_use = "builder methods must be chained or built"]
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags.extend(tags);
        self
    }

    /// Sets the version.
    #[must_use = "builder methods must be chained or built"]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Adds custom metadata.
    #[must_use = "builder methods must be chained or built"]
    pub fn custom(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom.insert(key.into(), value.into());
        self
    }

    /// Builds the metadata.
    #[must_use]
    pub fn build(self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: self.name.unwrap_or_else(|| "Unknown".to_string()),
            description: self.description,
            complexity: self.complexity,
            cacheable: self.cacheable,
            estimated_time: self.estimated_time,
            tags: self.tags,
            version: self.version,
            custom: self.custom,
        }
    }
}

// ============================================================================
// VALIDATOR REGISTRY METADATA
// ============================================================================

/// Metadata for a registered validator in the registry.
///
/// This extends `ValidatorMetadata` with registry-specific information.
#[derive(Debug, Clone)]
pub struct RegisteredValidatorMetadata {
    /// The base metadata.
    pub metadata: ValidatorMetadata,

    /// Unique identifier in the registry.
    pub id: String,

    /// When the validator was registered.
    pub registered_at: std::time::SystemTime,

    /// Number of times this validator has been used.
    pub usage_count: usize,

    /// Whether this validator is deprecated.
    pub deprecated: bool,

    /// Deprecation message if applicable.
    pub deprecation_message: Option<String>,
}

impl RegisteredValidatorMetadata {
    /// Creates new registry metadata.
    pub fn new(id: impl Into<String>, metadata: ValidatorMetadata) -> Self {
        Self {
            metadata,
            id: id.into(),
            registered_at: std::time::SystemTime::now(),
            usage_count: 0,
            deprecated: false,
            deprecation_message: None,
        }
    }

    /// Marks the validator as deprecated.
    #[must_use = "builder methods must be chained or built"]
    pub fn deprecate(mut self, message: impl Into<String>) -> Self {
        self.deprecated = true;
        self.deprecation_message = Some(message.into());
        self
    }

    /// Increments the usage count.
    pub fn increment_usage(&mut self) {
        self.usage_count += 1;
    }
}

// ============================================================================
// VALIDATOR STATISTICS
// ============================================================================

/// Runtime statistics for a validator.
///
/// Useful for performance monitoring and optimization.
#[derive(Debug, Clone, Default)]
pub struct ValidatorStatistics {
    /// Total number of validations performed.
    pub total_validations: u64,

    /// Number of successful validations.
    pub successful_validations: u64,

    /// Number of failed validations.
    pub failed_validations: u64,

    /// Total time spent in validation.
    pub total_time: Duration,

    /// Average validation time.
    pub average_time: Duration,

    /// Minimum validation time.
    pub min_time: Option<Duration>,

    /// Maximum validation time.
    pub max_time: Option<Duration>,

    /// Cache hit rate (if caching is enabled).
    pub cache_hit_rate: Option<f64>,
}

impl ValidatorStatistics {
    /// Creates new empty statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a validation result and its execution time.
    pub fn record(&mut self, success: bool, duration: Duration) {
        self.total_validations += 1;
        if success {
            self.successful_validations += 1;
        } else {
            self.failed_validations += 1;
        }

        self.total_time += duration;
        self.average_time = self.total_time / self.total_validations as u32;

        match self.min_time {
            None => self.min_time = Some(duration),
            Some(min) if duration < min => self.min_time = Some(duration),
            _ => {}
        }

        match self.max_time {
            None => self.max_time = Some(duration),
            Some(max) if duration > max => self.max_time = Some(duration),
            _ => {}
        }
    }

    /// Returns the success rate as a percentage.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total_validations == 0 {
            0.0
        } else {
            (self.successful_validations as f64 / self.total_validations as f64) * 100.0
        }
    }

    /// Returns the failure rate as a percentage.
    #[must_use]
    pub fn failure_rate(&self) -> f64 {
        100.0 - self.success_rate()
    }
}

impl fmt::Display for ValidatorStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Validator Statistics:")?;
        writeln!(f, "  Total validations: {}", self.total_validations)?;
        writeln!(
            f,
            "  Successful: {} ({:.2}%)",
            self.successful_validations,
            self.success_rate()
        )?;
        writeln!(
            f,
            "  Failed: {} ({:.2}%)",
            self.failed_validations,
            self.failure_rate()
        )?;
        writeln!(f, "  Average time: {:?}", self.average_time)?;
        if let Some(min) = self.min_time {
            writeln!(f, "  Min time: {min:?}")?;
        }
        if let Some(max) = self.max_time {
            writeln!(f, "  Max time: {max:?}")?;
        }
        if let Some(hit_rate) = self.cache_hit_rate {
            writeln!(f, "  Cache hit rate: {hit_rate:.2}%")?;
        }
        Ok(())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_builder() {
        let metadata = ValidatorMetadata::builder()
            .name("TestValidator")
            .description("A test validator")
            .complexity(ValidationComplexity::Linear)
            .cacheable(true)
            .tag("string")
            .tag("length")
            .build();

        assert_eq!(metadata.name, "TestValidator");
        assert_eq!(metadata.description, Some("A test validator".to_string()));
        assert_eq!(metadata.complexity, ValidationComplexity::Linear);
        assert!(metadata.cacheable);
        assert_eq!(metadata.tags.len(), 2);
    }

    #[test]
    fn test_complexity_ordering() {
        assert!(ValidationComplexity::Constant < ValidationComplexity::Linear);
        assert!(ValidationComplexity::Linear < ValidationComplexity::Expensive);
        assert!(
            ValidationComplexity::Expensive.is_more_expensive_than(&ValidationComplexity::Constant)
        );
    }

    #[test]
    fn test_statistics() {
        let mut stats = ValidatorStatistics::new();

        stats.record(true, Duration::from_millis(10));
        stats.record(true, Duration::from_millis(20));
        stats.record(false, Duration::from_millis(15));

        assert_eq!(stats.total_validations, 3);
        assert_eq!(stats.successful_validations, 2);
        assert_eq!(stats.failed_validations, 1);
        // Use approximate comparison for floating point
        assert!((stats.success_rate() - 200.0 / 3.0).abs() < 1e-10);
    }
}
