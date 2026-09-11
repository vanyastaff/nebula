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
//! [`validate_rules`] without context report explicit deferral in `StaticOnly`
//! and an unavailable-evaluation error in `Full`.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::{DiagnosticDisclosure, ExecutionMode, Rule, validate_rules};
//! use serde_json::json;
//!
//! let rules = vec![Rule::min_length(3), Rule::max_length(20)];
//!
//! assert!(validate_rules(
//!     &json!("alice"),
//!     &rules,
//!     ExecutionMode::StaticOnly,
//!     DiagnosticDisclosure::IncludeValue,
//! ).is_ok());
//! assert!(validate_rules(
//!     &json!("ab"),
//!     &rules,
//!     ExecutionMode::StaticOnly,
//!     DiagnosticDisclosure::IncludeValue,
//! ).is_err());
//! ```

use crate::{
    foundation::{ValidationError, ValidationErrors},
    rule::Rule,
};

/// Why a rule was not evaluated during a partial pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeferredReason {
    /// A predicate needs unavailable context or still-pending context values.
    PredicateContext,
    /// The static pass excludes deferred rules.
    DeferredRule,
    /// The deferred pass excludes static rules.
    StaticRule,
}

/// The result of a successful evaluation pass, including remaining obligations.
///
/// A deferred pass is not proof that the rules are satisfied. Re-evaluate the
/// entire rule tree with all required context before issuing a complete proof.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[must_use = "deferred evaluation must be completed before issuing a proof"]
pub enum EvaluationOutcome {
    /// The rule tree is satisfied by the supplied value and context.
    Satisfied,
    /// The rule tree still depends on the listed unavailable checks.
    Deferred(Vec<DeferredReason>),
}

impl EvaluationOutcome {
    /// Requires complete satisfaction, rejecting a partial result.
    ///
    /// # Errors
    /// Returns an `Unavailable` diagnostic when any obligation remains.
    pub fn require_satisfied(self) -> Result<(), ValidationError> {
        match self {
            Self::Satisfied => Ok(()),
            Self::Deferred(_) => Err(ValidationError::unavailable(
                "rule evaluation is incomplete",
            )),
        }
    }

    pub(crate) fn from_deferred(reasons: Vec<DeferredReason>) -> Self {
        if reasons.is_empty() {
            Self::Satisfied
        } else {
            Self::Deferred(reasons)
        }
    }
}

/// Controls which rules are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ExecutionMode {
    /// Execute non-deferred rules, using predicate context when supplied.
    ///
    /// This is the default and the mode used at schema-validation time.
    #[default]
    StaticOnly,

    /// Execute only deferred rules (requires runtime context).
    Deferred,

    /// Require evaluation of all rule kinds, reporting unavailable evaluators.
    Full,
}

/// Controls whether validation diagnostics may retain the evaluated value.
///
/// Schema owners must choose [`OmitValue`](Self::OmitValue) for protected
/// fields. The policy is propagated through the complete rule tree, including
/// logical combinators and described rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticDisclosure {
    /// Include the evaluated value in diagnostic template parameters.
    IncludeValue,
    /// Never materialize the evaluated value into a diagnostic.
    OmitValue,
}

/// Validates a JSON value against a slice of rules.
///
/// Iterates through all rules, recording checks excluded by the given
/// [`ExecutionMode`] as deferred, and collects all errors.
///
/// Predicates without context defer in `StaticOnly` and fail in `Full`.
/// Call `validate_rules_with_ctx` to supply the context.
///
/// # Arguments
///
/// - `value` — the JSON value to validate
/// - `rules` — the rules to apply
/// - `mode` — which rule categories to execute
/// - `disclosure` — whether diagnostics may retain the evaluated value
///
/// # Returns
///
/// `Satisfied` when the conjunction is proven, `Deferred` when obligations
/// remain, or `Err(ValidationErrors)` with collected failures.
pub fn validate_rules(
    value: &serde_json::Value,
    rules: &[Rule],
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationErrors> {
    validate_rules_with_ctx(value, rules, None, mode, disclosure)
}

/// Validates with an optional predicate context. Mode exclusions are explicit
/// deferred outcomes and propagate through nested logical rules.
///
/// `None` means the whole context is unavailable. Mark unresolved expression
/// roots with [`PredicateContext::with_pending_paths`](crate::rule::PredicateContext::with_pending_paths).
/// Other absent paths retain genuine missing-value semantics.
#[tracing::instrument(level = "debug", skip(value, rules, ctx), fields(rule_count = rules.len(), has_context = ctx.is_some()))]
pub fn validate_rules_with_ctx(
    value: &serde_json::Value,
    rules: &[Rule],
    ctx: Option<&crate::rule::PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationErrors> {
    // Fast path: empty rules slice — avoids all allocation and control flow.
    if rules.is_empty() {
        return Ok(EvaluationOutcome::Satisfied);
    }

    let mut errors = Vec::new();
    let mut deferred = Vec::new();

    for rule in rules {
        match rule.validate(value, ctx, mode, disclosure) {
            Ok(EvaluationOutcome::Satisfied) => {},
            Ok(EvaluationOutcome::Deferred(reasons)) => deferred.extend(reasons),
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Ok(EvaluationOutcome::from_deferred(deferred))
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
        let rules = vec![Rule::min_length(3), Rule::custom("should_skip").unwrap()];
        assert_eq!(
            validate_rules(
                &json!("alice"),
                &rules,
                ExecutionMode::StaticOnly,
                DiagnosticDisclosure::IncludeValue,
            )
            .unwrap(),
            EvaluationOutcome::Deferred(vec![DeferredReason::DeferredRule])
        );
    }

    #[test]
    fn static_only_catches_errors() {
        let rules = vec![Rule::min_length(5)];
        let errs = validate_rules(
            &json!("ab"),
            &rules,
            ExecutionMode::StaticOnly,
            DiagnosticDisclosure::IncludeValue,
        )
        .unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs.errors()[0].code.as_ref(), "min_length");
    }

    #[test]
    fn full_mode_runs_all() {
        let rules = vec![Rule::min_length(3), Rule::unique_by("id").unwrap()];
        let errors = validate_rules(
            &json!("alice"),
            &rules,
            ExecutionMode::Full,
            DiagnosticDisclosure::IncludeValue,
        )
        .unwrap_err();
        assert_eq!(
            errors.errors()[0].kind(),
            crate::ValidationErrorKind::Unavailable
        );
    }

    #[test]
    fn collects_multiple_errors() {
        let rules = vec![Rule::min_length(10), Rule::pattern("^[0-9]+$").unwrap()];
        let errs = validate_rules(
            &json!("abc"),
            &rules,
            ExecutionMode::StaticOnly,
            DiagnosticDisclosure::IncludeValue,
        )
        .unwrap_err();
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn empty_rules_passes() {
        assert!(
            validate_rules(
                &json!("anything"),
                &[],
                ExecutionMode::Full,
                DiagnosticDisclosure::IncludeValue,
            )
            .is_ok()
        );
    }

    #[test]
    fn deferred_mode_skips_static_rules() {
        let rules = vec![Rule::min_length(100), Rule::unique_by("id").unwrap()];
        let errors = validate_rules(
            &json!("short"),
            &rules,
            ExecutionMode::Deferred,
            DiagnosticDisclosure::IncludeValue,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors.errors()[0].kind(),
            crate::ValidationErrorKind::Unavailable
        );
    }

    #[test]
    fn deferred_mode_runs_deferred_rules() {
        let rules = vec![Rule::unique_by("id").unwrap()];
        let errors = validate_rules(
            &json!([1, 2]),
            &rules,
            ExecutionMode::Deferred,
            DiagnosticDisclosure::IncludeValue,
        )
        .unwrap_err();
        assert_eq!(
            errors.errors()[0].kind(),
            crate::ValidationErrorKind::Unavailable
        );
    }

    #[test]
    fn static_only_skips_predicates_without_ctx() {
        let rules = vec![Rule::predicate(Predicate::eq("x", json!(1)).unwrap()).unwrap()];
        assert_eq!(
            validate_rules(
                &json!("whatever"),
                &rules,
                ExecutionMode::StaticOnly,
                DiagnosticDisclosure::IncludeValue,
            )
            .unwrap(),
            EvaluationOutcome::Deferred(vec![DeferredReason::PredicateContext])
        );
    }

    #[test]
    fn full_mode_collects_all_errors() {
        let rules = vec![
            Rule::min_length(10),
            Rule::max_length(2),
            Rule::pattern("^[0-9]+$").unwrap(),
        ];
        // "abc" fails all three
        let errs = validate_rules(
            &json!("abc"),
            &rules,
            ExecutionMode::Full,
            DiagnosticDisclosure::IncludeValue,
        )
        .unwrap_err();
        assert_eq!(errs.len(), 3);
    }

    #[test]
    fn validate_rules_with_combinator() {
        let rules = vec![Rule::all([Rule::min_length(3), Rule::max_length(10)]).unwrap()];
        assert!(
            validate_rules(
                &json!("hello"),
                &rules,
                ExecutionMode::StaticOnly,
                DiagnosticDisclosure::IncludeValue,
            )
            .is_ok()
        );
        assert!(
            validate_rules(
                &json!("ab"),
                &rules,
                ExecutionMode::StaticOnly,
                DiagnosticDisclosure::IncludeValue,
            )
            .is_err()
        );
    }

    #[test]
    fn default_execution_mode_is_static_only() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::StaticOnly);
    }
}
