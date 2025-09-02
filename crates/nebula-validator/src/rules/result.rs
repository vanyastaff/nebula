//! Rule execution results

use super::RuleId;
use serde::{Serialize, Deserialize};
use std::fmt::{self, Display};

/// Result of rule execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleResult {
    /// Rule ID
    pub rule_id: RuleId,
    /// Outcome of the rule
    pub outcome: RuleOutcome,
    /// Violations if any
    pub violations: Vec<RuleViolation>,
    /// Execution time
    pub execution_time: Option<std::time::Duration>,
    /// Additional metadata
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

impl RuleResult {
    /// Create a passing result
    pub fn pass(rule_id: RuleId) -> Self {
        Self {
            rule_id,
            outcome: RuleOutcome::Pass,
            violations: Vec::new(),
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    /// Create a failing result
    pub fn fail(rule_id: RuleId, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.clone(),
            outcome: RuleOutcome::Fail,
            violations: vec![RuleViolation {
                rule_id,
                message: message.into(),
                severity: super::ConstraintSeverity::Error,
                fix_suggestion: None,
                field_path: None,
            }],
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    /// Create a warning result
    pub fn warning(rule_id: RuleId, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.clone(),
            outcome: RuleOutcome::Warning,
            violations: vec![RuleViolation {
                rule_id,
                message: message.into(),
                severity: super::ConstraintSeverity::Warning,
                fix_suggestion: None,
                field_path: None,
            }],
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    /// Create an info result
    pub fn info(rule_id: RuleId, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.clone(),
            outcome: RuleOutcome::Info,
            violations: vec![RuleViolation {
                rule_id,
                message: message.into(),
                severity: super::ConstraintSeverity::Info,
                fix_suggestion: None,
                field_path: None,
            }],
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    /// Create a critical failure
    pub fn critical(rule_id: RuleId, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.clone(),
            outcome: RuleOutcome::Critical,
            violations: vec![RuleViolation {
                rule_id,
                message: message.into(),
                severity: super::ConstraintSeverity::Critical,
                fix_suggestion: None,
                field_path: None,
            }],
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    /// Create a skipped result
    pub fn skip(rule_id: RuleId, reason: impl Into<String>) -> Self {
        let mut result = Self {
            rule_id,
            outcome: RuleOutcome::Skip,
            violations: Vec::new(),
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        };
        result.metadata.insert(
            "skip_reason".to_string(),
            serde_json::Value::String(reason.into())
        );
        result
    }
    
    /// Create an error result
    pub fn error(rule_id: RuleId, error: impl Into<String>) -> Self {
        let mut result = Self {
            rule_id,
            outcome: RuleOutcome::Error,
            violations: Vec::new(),
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        };
        result.metadata.insert(
            "error".to_string(),
            serde_json::Value::String(error.into())
        );
        result
    }
    
    /// Create from outcome
    pub fn from_outcome(rule_id: RuleId, outcome: RuleOutcome) -> Self {
        Self {
            rule_id,
            outcome,
            violations: Vec::new(),
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        }
    }
    
    /// Add a fix suggestion
    pub fn with_fix(mut self, suggestion: impl Into<String>) -> Self {
        if let Some(violation) = self.violations.first_mut() {
            violation.fix_suggestion = Some(suggestion.into());
        }
        self
    }
    
    /// Add field path
    pub fn with_field(mut self, path: impl Into<String>) -> Self {
        if let Some(violation) = self.violations.first_mut() {
            violation.field_path = Some(path.into());
        }
        self
    }
    
    /// Set execution time
    pub fn with_execution_time(mut self, duration: std::time::Duration) -> Self {
        self.execution_time = Some(duration);
        self
    }
    
    /// Check if the rule passed
    pub fn passed(&self) -> bool {
        matches!(self.outcome, RuleOutcome::Pass)
    }
    
    /// Check if the rule failed
    pub fn failed(&self) -> bool {
        matches!(self.outcome, RuleOutcome::Fail | RuleOutcome::Critical)
    }
    
    /// Check if there are violations
    pub fn has_violations(&self) -> bool {
        !self.violations.is_empty()
    }
    
    /// Combine multiple results
    pub fn combine(results: Vec<RuleResult>) -> Self {
        if results.is_empty() {
            return Self::pass(RuleId::new("combined"));
        }
        
        let mut combined = Self {
            rule_id: RuleId::new("combined"),
            outcome: RuleOutcome::Pass,
            violations: Vec::new(),
            execution_time: None,
            metadata: std::collections::HashMap::new(),
        };
        
        // Collect all violations
        for result in &results {
            combined.violations.extend(result.violations.clone());
        }
        
        // Determine overall outcome
        if results.iter().any(|r| matches!(r.outcome, RuleOutcome::Critical)) {
            combined.outcome = RuleOutcome::Critical;
        } else if results.iter().any(|r| matches!(r.outcome, RuleOutcome::Fail)) {
            combined.outcome = RuleOutcome::Fail;
        } else if results.iter().any(|r| matches!(r.outcome, RuleOutcome::Error)) {
            combined.outcome = RuleOutcome::Error;
        } else if results.iter().any(|r| matches!(r.outcome, RuleOutcome::Warning)) {
            combined.outcome = RuleOutcome::Warning;
        } else if results.iter().any(|r| matches!(r.outcome, RuleOutcome::Info)) {
            combined.outcome = RuleOutcome::Info;
        }
        
        // Sum execution times
        let total_time: Option<std::time::Duration> = results
            .iter()
            .filter_map(|r| r.execution_time)
            .reduce(|a, b| a + b);
        combined.execution_time = total_time;
        
        combined
    }
}

/// Outcome of rule execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleOutcome {
    /// Rule passed
    Pass,
    /// Rule failed
    Fail,
    /// Rule generated a warning
    Warning,
    /// Rule generated info
    Info,
    /// Critical failure
    Critical,
    /// Rule was skipped
    Skip,
    /// Error during execution
    Error,
}

impl RuleOutcome {
    /// Check if this is a passing outcome
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
    
    /// Check if this is a failure
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Fail | Self::Critical)
    }
}

impl Display for RuleOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::Fail => write!(f, "FAIL"),
            Self::Warning => write!(f, "WARNING"),
            Self::Info => write!(f, "INFO"),
            Self::Critical => write!(f, "CRITICAL"),
            Self::Skip => write!(f, "SKIP"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

/// Rule violation details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleViolation {
    /// Rule that was violated
    pub rule_id: RuleId,
    /// Violation message
    pub message: String,
    /// Severity of the violation
    pub severity: super::ConstraintSeverity,
    /// Suggestion for fixing
    pub fix_suggestion: Option<String>,
    /// Field path where violation occurred
    pub field_path: Option<String>,
}

/// Report of rule execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleReport {
    /// Total rules executed
    pub total_rules: usize,
    /// Rules passed
    pub passed: usize,
    /// Rules failed
    pub failed: usize,
    /// Rules with warnings
    pub warnings: usize,
    /// Rules skipped
    pub skipped: usize,
    /// Total execution time
    pub total_time: Option<std::time::Duration>,
    /// All violations
    pub violations: Vec<RuleViolation>,
    /// Results by rule
    pub results: Vec<RuleResult>,
}

impl RuleReport {
    /// Create from results
    pub fn from_results(results: Vec<RuleResult>) -> Self {
        let mut report = Self {
            total_rules: results.len(),
            passed: 0,
            failed: 0,
            warnings: 0,
            skipped: 0,
            total_time: None,
            violations: Vec::new(),
            results: results.clone(),
        };
        
        for result in &results {
            match result.outcome {
                RuleOutcome::Pass => report.passed += 1,
                RuleOutcome::Fail | RuleOutcome::Critical => report.failed += 1,
                RuleOutcome::Warning => report.warnings += 1,
                RuleOutcome::Skip => report.skipped += 1,
                _ => {},
            }
            
            report.violations.extend(result.violations.clone());
        }
        
        report.total_time = results
            .iter()
            .filter_map(|r| r.execution_time)
            .reduce(|a, b| a + b);
        
        report
    }
    
    /// Check if all rules passed
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }
    
    /// Get pass rate
    pub fn pass_rate(&self) -> f64 {
        if self.total_rules == 0 {
            0.0
        } else {
            self.passed as f64 / self.total_rules as f64
        }
    }
}

use super::ConstraintSeverity;