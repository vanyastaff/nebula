//! Validation context and state management
//! 
//! This module provides types for managing validation context, state,
//! and execution metadata.

use serde_json::Value;
use std::collections::HashMap;
use crate::types::{ValidationResult, ValidationConfig, ValidationMetadata};

// ==================== Validation Context ====================

/// Basic validation context
#[derive(Debug, Clone)]
pub struct ValidationContext {
    /// Field path being validated
    pub field_path: Option<String>,
    /// Parent object if available
    pub parent: Option<Value>,
    /// Validation mode
    pub mode: ValidationMode,
    /// Additional context data
    pub data: HashMap<String, Value>,
}

impl ValidationContext {
    /// Create new validation context
    pub fn new() -> Self {
        Self {
            field_path: None,
            parent: None,
            mode: ValidationMode::Strict,
            data: HashMap::new(),
        }
    }
    
    /// Set field path
    pub fn with_field_path(mut self, path: impl Into<String>) -> Self {
        self.field_path = Some(path.into());
        self
    }
    
    /// Set parent object
    pub fn with_parent(mut self, parent: Value) -> Self {
        self.parent = Some(parent);
        self
    }
    
    /// Set validation mode
    pub fn with_mode(mut self, mode: ValidationMode) -> Self {
        self.mode = mode;
        self
    }
    
    /// Add context data
    pub fn with_data(mut self, key: impl Into<String>, value: Value) -> Self {
        self.data.insert(key.into(), value);
        self
    }
    
    /// Get context data
    pub fn get_data(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }
}

/// Full validation context with all available information
#[derive(Debug, Clone)]
pub struct FullValidationContext {
    /// Basic context
    pub basic: ValidationContext,
    /// All field values in the current object
    pub field_values: HashMap<String, Value>,
    /// Validation configuration
    pub config: ValidationConfig,
    /// Execution metadata
    pub metadata: ValidationMetadata,
}

impl FullValidationContext {
    /// Create new full context
    pub fn new() -> Self {
        Self {
            basic: ValidationContext::new(),
            field_values: HashMap::new(),
            config: ValidationConfig::default(),
            metadata: ValidationMetadata::new(),
        }
    }
    
    /// Set basic context
    pub fn with_basic_context(mut self, context: ValidationContext) -> Self {
        self.basic = context;
        self
    }
    
    /// Set field values
    pub fn with_field_values(mut self, values: HashMap<String, Value>) -> Self {
        self.field_values = values;
        self
    }
    
    /// Set configuration
    pub fn with_config(mut self, config: ValidationConfig) -> Self {
        self.config = config;
        self
    }
    
    /// Set metadata
    pub fn with_metadata(mut self, metadata: ValidationMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Validation mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationMode {
    /// Strict validation - all rules must pass
    Strict,
    /// Lenient validation - some rules can fail
    Lenient,
    /// Warning-only validation - failures become warnings
    WarningOnly,
}

// ==================== Validation State ====================

/// Validation state for state-aware validators
#[derive(Debug, Clone)]
pub struct ValidationState {
    /// Current status
    pub status: ValidationStatus,
    /// Last validation result
    pub last_result: Option<ValidationResult<()>>,
    /// Validation count
    pub validation_count: u64,
    /// Last validation timestamp
    pub last_validation: Option<chrono::DateTime<chrono::Utc>>,
}

impl ValidationState {
    /// Create new validation state
    pub fn new() -> Self {
        Self {
            status: ValidationStatus::Ready,
            last_result: None,
            validation_count: 0,
            last_validation: None,
        }
    }
    
    /// Mark validation as started
    pub fn mark_started(&mut self) {
        self.status = ValidationStatus::Validating;
        self.last_validation = Some(chrono::Utc::now());
    }
    
    /// Mark validation as completed
    pub fn mark_completed(&mut self, result: ValidationResult<()>) {
        self.status = if result.is_success() {
            ValidationStatus::Completed
        } else {
            ValidationStatus::Error
        };
        self.last_result = Some(result);
        self.validation_count += 1;
    }
    
    /// Reset state
    pub fn reset(&mut self) {
        self.status = ValidationStatus::Ready;
        self.last_result = None;
        self.validation_count = 0;
        self.last_validation = None;
    }
}

/// Validation status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationStatus {
    /// Validator is ready
    Ready,
    /// Validator is currently validating
    Validating,
    /// Validator has completed validation
    Completed,
    /// Validator has encountered an error
    Error,
}

/// State statistics
#[derive(Debug, Clone)]
pub struct StateStats {
    /// Total validations performed
    pub total_validations: u64,
    /// Successful validations
    pub successful_validations: u64,
    /// Failed validations
    pub failed_validations: u64,
    /// Average validation time
    pub average_validation_time: std::time::Duration,
}

impl StateStats {
    /// Create new stats
    pub fn new() -> Self {
        Self {
            total_validations: 0,
            successful_validations: 0,
            failed_validations: 0,
            average_validation_time: std::time::Duration::ZERO,
        }
    }
    
    /// Record a validation
    pub fn record_validation(&mut self, success: bool, duration: std::time::Duration) {
        self.total_validations += 1;
        
        if success {
            self.successful_validations += 1;
        } else {
            self.failed_validations += 1;
        }
        
        // Update average time
        let total_time = self.average_validation_time * (self.total_validations - 1) as u32 + duration;
        self.average_validation_time = total_time / self.total_validations as u32;
    }
}

// ==================== Validation Strategy ====================

/// Validation strategy for composite validators
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationStrategy {
    /// All validators must pass
    All,
    /// At least one validator must pass
    Any,
    /// Exactly one validator must pass
    One,
    /// Validators must pass in sequence
    Sequence,
    /// Validators must pass in parallel
    Parallel,
}

impl ValidationStrategy {
    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Any => "Any",
            Self::One => "One",
            Self::Sequence => "Sequence",
            Self::Parallel => "Parallel",
        }
    }
    
    /// Get description
    pub fn description(&self) -> &'static str {
        match self {
            Self::All => "All validators must pass for success",
            Self::Any => "At least one validator must pass for success",
            Self::One => "Exactly one validator must pass for success",
            Self::Sequence => "Validators must pass in sequence order",
            Self::Parallel => "Validators can run in parallel",
        }
    }
}

// ==================== Re-exports ====================

pub use ValidationContext as Context;
pub use FullValidationContext as FullContext;
pub use ValidationMode as Mode;
pub use ValidationState as State;
pub use ValidationStatus as Status;
pub use StateStats as Stats;
pub use ValidationStrategy as Strategy;
