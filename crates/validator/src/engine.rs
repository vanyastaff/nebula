//! Validation engine for declarative rules.
//!
//! Provides [`validate_rules`] — a single function to validate a JSON value
//! against a slice of [`Rule`]s with configurable execution mode.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::{Rule, ExecutionMode, validate_rules};
//! use serde_json::json;
//!
//! let rules = vec![
//!     Rule::MinLength { min: 3, message: None },
//!     Rule::MaxLength { max: 20, message: None },
//! ];
//!
//! assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly).is_ok());
//! assert!(validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).is_err());
//! ```

use crate::foundation::ValidationError;
use crate::rule::Rule;

/// Controls which rules are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ExecutionMode {
    /// Execute only context-free, non-deferred rules.
    ///
    /// This is the default and the mode used at schema-validation time.
    #[default]
    StaticOnly,

    /// Execute only deferred rules (requires runtime context).
    Deferred,

    /// Execute all rules in deterministic order.
    Full,
}

/// Validates a JSON value against a slice of rules.
///
/// # Arguments
///
/// - `value` — the JSON value to validate
/// - `rules` — the rules to apply
/// - `mode` — which rule categories to execute
///
/// # Returns
///
/// `Ok(())` if all applicable rules pass, or `Err` with collected errors.
pub fn validate_rules(
    value: &serde_json::Value,
    rules: &[Rule],
    mode: ExecutionMode,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    for rule in rules {
        let should_run = match mode {
            ExecutionMode::StaticOnly => !rule.is_deferred(),
            ExecutionMode::Deferred => rule.is_deferred(),
            ExecutionMode::Full => true,
        };

        if !should_run {
            continue;
        }

        if let Err(e) = rule.validate_value(value) {
            errors.push(e);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn static_only_skips_deferred() {
        let rules = vec![
            Rule::MinLength {
                min: 3,
                message: None,
            },
            Rule::Custom {
                expression: "should_skip".into(),
                message: None,
            },
        ];
        assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly).is_ok());
    }

    #[test]
    fn static_only_catches_errors() {
        let rules = vec![Rule::MinLength {
            min: 5,
            message: None,
        }];
        let errs = validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].code.as_ref(), "min_length");
    }

    #[test]
    fn full_mode_runs_all() {
        let rules = vec![
            Rule::MinLength {
                min: 3,
                message: None,
            },
            Rule::UniqueBy {
                key: "id".into(),
                message: None,
            },
        ];
        // Deferred rules return Ok by default in validate_value
        assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::Full).is_ok());
    }

    #[test]
    fn collects_multiple_errors() {
        let rules = vec![
            Rule::MinLength {
                min: 10,
                message: None,
            },
            Rule::Pattern {
                pattern: "^[0-9]+$".into(),
                message: None,
            },
        ];
        let errs = validate_rules(&json!("abc"), &rules, ExecutionMode::StaticOnly).unwrap_err();
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn empty_rules_passes() {
        assert!(validate_rules(&json!("anything"), &[], ExecutionMode::Full).is_ok());
    }
}
