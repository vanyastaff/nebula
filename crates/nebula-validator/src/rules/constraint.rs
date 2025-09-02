//! Constraint implementation for rules

use super::{Rule, RuleId, RuleMetadata, RulePriority, RuleContext, RuleResult, RuleOutcome};
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use serde_json::Value;

/// Severity levels for constraints
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConstraintSeverity {
    /// Informational - doesn't affect validation
    Info,
    /// Warning - validation passes but with warnings
    Warning,
    /// Error - validation fails
    Error,
    /// Critical - validation fails immediately
    Critical,
}

impl ConstraintSeverity {
    /// Check if this severity causes failure
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Error | Self::Critical)
    }
    
    /// Check if this is critical
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::Critical)
    }
}

/// A constraint is a specialized rule for enforcing business logic
#[derive(Debug)]
pub struct Constraint {
    id: RuleId,
    metadata: RuleMetadata,
    priority: RulePriority,
    severity: ConstraintSeverity,
    predicate: Box<dyn Fn(&Value, &RuleContext) -> bool + Send + Sync>,
    message: String,
    fix_suggestion: Option<String>,
}

impl Constraint {
    /// Create a new constraint
    pub fn new(
        id: impl Into<String>,
        predicate: impl Fn(&Value, &RuleContext) -> bool + Send + Sync + 'static,
        message: impl Into<String>,
    ) -> Self {
        let id = RuleId::new(id);
        Self {
            metadata: RuleMetadata::new(id.as_str()),
            id,
            priority: RulePriority::NORMAL,
            severity: ConstraintSeverity::Error,
            predicate: Box::new(predicate),
            message: message.into(),
            fix_suggestion: None,
        }
    }
    
    /// Create a builder
    pub fn builder(id: impl Into<String>) -> ConstraintBuilder {
        ConstraintBuilder::new(id)
    }
    
    /// Set severity
    pub fn with_severity(mut self, severity: ConstraintSeverity) -> Self {
        self.severity = severity;
        self
    }
    
    /// Set priority
    pub fn with_priority(mut self, priority: RulePriority) -> Self {
        self.priority = priority;
        self
    }
    
    /// Set fix suggestion
    pub fn with_fix(mut self, suggestion: impl Into<String>) -> Self {
        self.fix_suggestion = Some(suggestion.into());
        self
    }
}

#[async_trait]
impl Rule for Constraint {
    fn id(&self) -> &RuleId {
        &self.id
    }
    
    fn priority(&self) -> RulePriority {
        self.priority
    }
    
    fn metadata(&self) -> &RuleMetadata {
        &self.metadata
    }
    
    async fn apply(&self, value: &Value, context: &RuleContext) -> RuleResult {
        let satisfied = (self.predicate)(value, context);
        
        if satisfied {
            RuleResult::pass(self.id.clone())
        } else {
            match self.severity {
                ConstraintSeverity::Info => {
                    RuleResult::info(self.id.clone(), &self.message)
                },
                ConstraintSeverity::Warning => {
                    RuleResult::warning(self.id.clone(), &self.message)
                },
                ConstraintSeverity::Error => {
                    let mut result = RuleResult::fail(self.id.clone(), &self.message);
                    if let Some(ref suggestion) = self.fix_suggestion {
                        result = result.with_fix(suggestion.clone());
                    }
                    result
                },
                ConstraintSeverity::Critical => {
                    RuleResult::critical(self.id.clone(), &self.message)
                },
            }
        }
    }
}

/// Builder for constraints
pub struct ConstraintBuilder {
    id: RuleId,
    metadata: RuleMetadata,
    priority: RulePriority,
    severity: ConstraintSeverity,
    predicate: Option<Box<dyn Fn(&Value, &RuleContext) -> bool + Send + Sync>>,
    message: Option<String>,
    fix_suggestion: Option<String>,
}

impl ConstraintBuilder {
    /// Create a new builder
    pub fn new(id: impl Into<String>) -> Self {
        let id = RuleId::new(id);
        Self {
            metadata: RuleMetadata::new(id.as_str()),
            id,
            priority: RulePriority::NORMAL,
            severity: ConstraintSeverity::Error,
            predicate: None,
            message: None,
            fix_suggestion: None,
        }
    }
    
    /// Set the predicate
    pub fn predicate<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&Value, &RuleContext) -> bool + Send + Sync + 'static,
    {
        self.predicate = Some(Box::new(predicate));
        self
    }
    
    /// Set the message
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
    
    /// Set severity
    pub fn severity(mut self, severity: ConstraintSeverity) -> Self {
        self.severity = severity;
        self
    }
    
    /// Set priority
    pub fn priority(mut self, priority: RulePriority) -> Self {
        self.priority = priority;
        self
    }
    
    /// Set fix suggestion
    pub fn fix(mut self, suggestion: impl Into<String>) -> Self {
        self.fix_suggestion = Some(suggestion.into());
        self
    }
    
    /// Set metadata
    pub fn metadata(mut self, metadata: RuleMetadata) -> Self {
        self.metadata = metadata;
        self
    }
    
    /// Build the constraint
    pub fn build(self) -> Result<Constraint, BuildError> {
        let predicate = self.predicate
            .ok_or(BuildError::MissingPredicate)?;
        let message = self.message
            .ok_or(BuildError::MissingMessage)?;
        
        Ok(Constraint {
            id: self.id,
            metadata: self.metadata,
            priority: self.priority,
            severity: self.severity,
            predicate,
            message,
            fix_suggestion: self.fix_suggestion,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("Missing predicate")]
    MissingPredicate,
    
    #[error("Missing message")]
    MissingMessage,
}

/// Common constraint predicates
pub mod predicates {
    use serde_json::Value;
    use super::RuleContext;
    
    /// Field exists predicate
    pub fn field_exists(field: &str) -> impl Fn(&Value, &RuleContext) -> bool + Send + Sync + Clone {
        let field = field.to_string();
        move |value: &Value, _: &RuleContext| {
            value.get(&field).is_some()
        }
    }
    
    /// Field equals predicate
    pub fn field_equals(field: &str, expected: Value) -> impl Fn(&Value, &RuleContext) -> bool + Send + Sync + Clone {
        let field = field.to_string();
        move |value: &Value, _: &RuleContext| {
            value.get(&field) == Some(&expected)
        }
    }
    
    /// Range predicate for numbers
    pub fn in_range(field: &str, min: f64, max: f64) -> impl Fn(&Value, &RuleContext) -> bool + Send + Sync + Clone {
        let field = field.to_string();
        move |value: &Value, _: &RuleContext| {
            value.get(&field)
                .and_then(|v| v.as_f64())
                .map(|n| n >= min && n <= max)
                .unwrap_or(false)
        }
    }
    
    /// String length predicate
    pub fn string_length(field: &str, min: usize, max: usize) -> impl Fn(&Value, &RuleContext) -> bool + Send + Sync + Clone {
        let field = field.to_string();
        move |value: &Value, _: &RuleContext| {
            value.get(&field)
                .and_then(|v| v.as_str())
                .map(|s| s.len() >= min && s.len() <= max)
                .unwrap_or(false)
        }
    }
    
    /// Pattern match predicate
    pub fn matches_pattern(field: &str, pattern: regex::Regex) -> impl Fn(&Value, &RuleContext) -> bool + Send + Sync {
        let field = field.to_string();
        move |value: &Value, _: &RuleContext| {
            value.get(&field)
                .and_then(|v| v.as_str())
                .map(|s| pattern.is_match(s))
                .unwrap_or(false)
        }
    }
}