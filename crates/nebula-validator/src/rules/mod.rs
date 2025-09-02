//! Rule engine for complex validation logic
//! 
//! This module provides a flexible rule-based validation system that allows
//! for dynamic rule evaluation, constraints, and complex business logic.

mod engine;
mod rule;
mod constraint;
mod context;
mod executor;
mod result;

// Re-export all types
pub use engine::{RuleEngine, RuleEngineBuilder, EngineConfig};
pub use rule::{Rule, RuleMetadata, RulePriority, RuleId};
pub use constraint::{Constraint, ConstraintSeverity, ConstraintBuilder};
pub use context::{RuleContext, ContextValue, ContextBuilder};
pub use executor::{RuleExecutor, ExecutionStrategy, ExecutionMode};
pub use result::{RuleResult, RuleOutcome, RuleReport, RuleViolation};

// Prelude for convenient imports
pub mod prelude {
    pub use super::{
        RuleEngine, Rule, Constraint, RuleContext,
        RuleResult, RuleOutcome, ConstraintSeverity,
    };
}

/// Common trait for rule-like types
pub trait RuleLike: Send + Sync {
    /// Get rule identifier
    fn id(&self) -> &RuleId;
    
    /// Get rule priority
    fn priority(&self) -> RulePriority;
    
    /// Check if rule is enabled
    fn is_enabled(&self) -> bool {
        true
    }
    
    /// Get rule metadata
    fn metadata(&self) -> &RuleMetadata;
}