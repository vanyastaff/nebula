//! Validation engine for declarative rules.
//!
//! Provides [`validate_rules`] — a single function to validate a JSON value
//! against a slice of [`Rule`]s with configurable [`ExecutionMode`].
//!
//! # Execution Modes
//!
//! | Mode | Runs | Skips |
//! |------|------|-------|
//! | [`StaticOnly`](ExecutionMode::StaticOnly) | Value rules, predicates, combinators | Deferred (`Custom`, `UniqueBy`) |
//! | [`Deferred`](ExecutionMode::Deferred) | Deferred rules only | Everything else |
//! | [`Full`](ExecutionMode::Full) | All value + deferred rules | Predicates (use [`Rule::evaluate`] directly) |
//!
//! **Important:** Predicate rules (`Eq`, `Ne`, `Gt`, etc.) always return
//! `Ok(())` from `validate_value` regardless of mode. To evaluate predicates,
//! call [`Rule::evaluate`] directly with a context map.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::{ExecutionMode, Rule, validate_rules};
//! use serde_json::json;
//!
//! let rules = vec![
//!     Rule::MinLength {
//!         min: 3,
//!         message: None,
//!     },
//!     Rule::MaxLength {
//!         max: 20,
//!         message: None,
//!     },
//! ];
//!
//! assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly).is_ok());
//! assert!(validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).is_err());
//! ```

use crate::{foundation::ValidationErrors, rule::Rule};

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

    /// Execute all value + deferred rules in deterministic order.
    ///
    /// Note: predicate rules still return `Ok(())` from `validate_value` —
    /// they require `Rule::evaluate()` with a context map.
    Full,
}

/// Validates a JSON value against a slice of rules.
///
/// Iterates through all rules, skipping those not applicable to the given
/// [`ExecutionMode`], and collects all errors (non-short-circuiting).
///
/// # Arguments
///
/// - `value` — the JSON value to validate
/// - `rules` — the rules to apply
/// - `mode` — which rule categories to execute
///
/// # Returns
///
/// `Ok(())` if all applicable rules pass, or `Err(ValidationErrors)`
/// with all collected validation failures.
pub fn validate_rules(
    value: &serde_json::Value,
    rules: &[Rule],
    mode: ExecutionMode,
) -> Result<(), ValidationErrors> {
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
        Err(errors.into_iter().collect())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
        assert_eq!(errs.errors()[0].code.as_ref(), "min_length");
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

    #[test]
    fn deferred_mode_skips_static_rules() {
        let rules = vec![
            Rule::MinLength {
                min: 100,
                message: None,
            },
            Rule::UniqueBy {
                key: "id".into(),
                message: None,
            },
        ];
        // Deferred mode skips MinLength, UniqueBy returns Ok
        assert!(validate_rules(&json!("short"), &rules, ExecutionMode::Deferred).is_ok());
    }

    #[test]
    fn deferred_mode_runs_deferred_rules() {
        let rules = vec![Rule::UniqueBy {
            key: "id".into(),
            message: None,
        }];
        // UniqueBy is deferred and returns Ok by default
        assert!(validate_rules(&json!([1, 2]), &rules, ExecutionMode::Deferred).is_ok());
    }

    #[test]
    fn static_only_skips_predicates() {
        let rules = vec![Rule::Eq {
            field: "x".into(),
            value: json!(1),
        }];
        // Predicates return Ok in validate_value
        assert!(validate_rules(&json!("whatever"), &rules, ExecutionMode::StaticOnly).is_ok());
    }

    #[test]
    fn full_mode_collects_all_errors() {
        let rules = vec![
            Rule::MinLength {
                min: 10,
                message: None,
            },
            Rule::MaxLength {
                max: 2,
                message: None,
            },
            Rule::Pattern {
                pattern: "^[0-9]+$".into(),
                message: None,
            },
        ];
        // "abc" fails all three
        let errs = validate_rules(&json!("abc"), &rules, ExecutionMode::Full).unwrap_err();
        assert_eq!(errs.len(), 3);
    }

    #[test]
    fn validate_rules_with_combinator() {
        let rules = vec![Rule::All {
            rules: vec![
                Rule::MinLength {
                    min: 3,
                    message: None,
                },
                Rule::MaxLength {
                    max: 10,
                    message: None,
                },
            ],
        }];
        assert!(validate_rules(&json!("hello"), &rules, ExecutionMode::StaticOnly).is_ok());
        assert!(validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).is_err());
    }

    #[test]
    fn default_execution_mode_is_static_only() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::StaticOnly);
    }
}
