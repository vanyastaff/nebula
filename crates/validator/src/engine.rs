//! Validation engine for declarative rules.
//!
//! Provides [`validate_rules`] — a single function to validate a JSON value
//! against a slice of [`Rule`]s with configurable [`ExecutionMode`].
//!
//! # Execution Modes
//!
//! | Mode | Runs | Skips |
//! |------|------|-------|
//! | [`StaticOnly`](ExecutionMode::StaticOnly) | Value rules, combinators | Deferred (`Custom`, `UniqueBy`) |
//! | [`Deferred`](ExecutionMode::Deferred) | Deferred rules only | Everything else |
//! | [`Full`](ExecutionMode::Full) | All value + deferred rules | — |
//!
//! Predicates require a `PredicateContext` — call
//! `validate_rules_with_ctx` to thread one in; predicates dispatched via
//! [`validate_rules`] (no ctx) are treated as `Ok(())`.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::{ExecutionMode, Rule, validate_rules};
//! use serde_json::json;
//!
//! let rules = vec![Rule::min_length(3), Rule::max_length(20)];
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
    Full,
}

/// Validates a JSON value against a slice of rules.
///
/// Iterates through all rules, skipping those not applicable to the given
/// [`ExecutionMode`], and collects all errors (non-short-circuiting).
///
/// Predicates with no ctx short-circuit to `Ok(())`. Call
/// `validate_rules_with_ctx` when predicate evaluation is required.
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
    validate_rules_with_ctx(value, rules, None, mode)
}

/// Validates with an optional predicate context. Rules whose kind doesn't
/// match `mode` are skipped.
pub fn validate_rules_with_ctx(
    value: &serde_json::Value,
    rules: &[Rule],
    ctx: Option<&crate::rule::PredicateContext>,
    mode: ExecutionMode,
) -> Result<(), ValidationErrors> {
    // Fast path: empty rules slice — avoids all allocation and control flow.
    if rules.is_empty() {
        return Ok(());
    }

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

        if let Err(e) = rule.validate(value, ctx, mode) {
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
    use crate::rule::Predicate;

    #[test]
    fn static_only_skips_deferred() {
        let rules = vec![Rule::min_length(3), Rule::custom("should_skip")];
        assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::StaticOnly).is_ok());
    }

    #[test]
    fn static_only_catches_errors() {
        let rules = vec![Rule::min_length(5)];
        let errs = validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs.errors()[0].code.as_ref(), "min_length");
    }

    #[test]
    fn full_mode_runs_all() {
        let rules = vec![Rule::min_length(3), Rule::unique_by("id").unwrap()];
        // Deferred rules return Ok by default in validate (no ctx)
        assert!(validate_rules(&json!("alice"), &rules, ExecutionMode::Full).is_ok());
    }

    #[test]
    fn collects_multiple_errors() {
        let rules = vec![Rule::min_length(10), Rule::pattern("^[0-9]+$")];
        let errs = validate_rules(&json!("abc"), &rules, ExecutionMode::StaticOnly).unwrap_err();
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn empty_rules_passes() {
        assert!(validate_rules(&json!("anything"), &[], ExecutionMode::Full).is_ok());
    }

    #[test]
    fn deferred_mode_skips_static_rules() {
        let rules = vec![Rule::min_length(100), Rule::unique_by("id").unwrap()];
        // Deferred mode skips MinLength; UniqueBy returns Ok
        assert!(validate_rules(&json!("short"), &rules, ExecutionMode::Deferred).is_ok());
    }

    #[test]
    fn deferred_mode_runs_deferred_rules() {
        let rules = vec![Rule::unique_by("id").unwrap()];
        // UniqueBy is deferred and returns Ok by default
        assert!(validate_rules(&json!([1, 2]), &rules, ExecutionMode::Deferred).is_ok());
    }

    #[test]
    fn static_only_skips_predicates_without_ctx() {
        let rules = vec![Rule::predicate(Predicate::eq("x", json!(1)).unwrap())];
        // Predicates return Ok in validate when ctx is None
        assert!(validate_rules(&json!("whatever"), &rules, ExecutionMode::StaticOnly).is_ok());
    }

    #[test]
    fn full_mode_collects_all_errors() {
        let rules = vec![
            Rule::min_length(10),
            Rule::max_length(2),
            Rule::pattern("^[0-9]+$"),
        ];
        // "abc" fails all three
        let errs = validate_rules(&json!("abc"), &rules, ExecutionMode::Full).unwrap_err();
        assert_eq!(errs.len(), 3);
    }

    #[test]
    fn validate_rules_with_combinator() {
        let rules = vec![Rule::all([Rule::min_length(3), Rule::max_length(10)])];
        assert!(validate_rules(&json!("hello"), &rules, ExecutionMode::StaticOnly).is_ok());
        assert!(validate_rules(&json!("ab"), &rules, ExecutionMode::StaticOnly).is_err());
    }

    #[test]
    fn default_execution_mode_is_static_only() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::StaticOnly);
    }
}
