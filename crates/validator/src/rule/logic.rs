//! Evaluation for logical nodes in the flat rule arena.

use super::{PredicateContext, RuleChildren, RuleRef};
use crate::{
    engine::{DiagnosticDisclosure, EvaluationOutcome, ExecutionMode},
    foundation::{ValidationError, ValidationErrorKind},
};

pub(super) fn matches_all(
    children: RuleChildren<'_>,
    ctx: &PredicateContext,
) -> Result<bool, ValidationError> {
    matches_many(children, ctx, true)
}

pub(super) fn matches_any(
    children: RuleChildren<'_>,
    ctx: &PredicateContext,
) -> Result<bool, ValidationError> {
    matches_many(children, ctx, false)
}

fn matches_many(
    children: RuleChildren<'_>,
    ctx: &PredicateContext,
    require_all: bool,
) -> Result<bool, ValidationError> {
    let mut matched = require_all;
    let mut unavailable = None;
    // Every condition is visited so pending data cannot hide invalid rule kinds.
    for child in children {
        match child.matches_bounded(ctx) {
            Ok(value) if require_all => matched &= value,
            Ok(value) => matched |= value,
            Err(error) if error.kind() == ValidationErrorKind::Unavailable => {
                if unavailable.is_none() {
                    unavailable = Some(error);
                }
            },
            Err(error) => return Err(error),
        }
    }
    unavailable.map_or(Ok(matched), Err)
}

/// Per-child classification shared by the `all` / `any` evaluators.
///
/// A non-violation diagnostic (invalid rule, unavailable evaluator) is
/// structural: it aborts the whole combinator instead of counting as a failed
/// alternative, so it is returned as `Err` from the collector.
struct ChildOutcomes {
    errors: Vec<ValidationError>,
    deferred: Vec<crate::engine::DeferredReason>,
    satisfied: bool,
}

fn collect_children(
    children: RuleChildren<'_>,
    input: &serde_json::Value,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<ChildOutcomes, ValidationError> {
    let mut outcomes = ChildOutcomes {
        errors: Vec::new(),
        deferred: Vec::new(),
        satisfied: false,
    };
    for child in children {
        match child.validate_bounded(input, ctx, mode, disclosure) {
            Ok(EvaluationOutcome::Satisfied) => outcomes.satisfied = true,
            Ok(EvaluationOutcome::Deferred(reasons)) => outcomes.deferred.extend(reasons),
            Err(error) if !error.is_violation() => return Err(error),
            Err(error) => outcomes.errors.push(error),
        }
    }
    Ok(outcomes)
}

pub(super) fn validate_all(
    children: RuleChildren<'_>,
    input: &serde_json::Value,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationError> {
    let ChildOutcomes {
        mut errors,
        deferred,
        ..
    } = collect_children(children, input, ctx, mode, disclosure)?;

    // A single failure is returned directly, not wrapped in `all_failed`.
    // Pop first, so the "exactly one" case needs no panic and no dead branch.
    let Some(last) = errors.pop() else {
        return Ok(EvaluationOutcome::from_deferred(deferred));
    };
    if errors.is_empty() {
        return Err(last);
    }
    errors.push(last);
    let count = errors.len();
    Err(
        ValidationError::new("all_failed", format!("{count} of the rules failed"))
            .with_nested(errors),
    )
}

pub(super) fn validate_any(
    children: RuleChildren<'_>,
    input: &serde_json::Value,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationError> {
    let ChildOutcomes {
        errors,
        deferred,
        satisfied,
    } = collect_children(children, input, ctx, mode, disclosure)?;

    if satisfied {
        return Ok(EvaluationOutcome::Satisfied);
    }
    if !deferred.is_empty() {
        return Ok(EvaluationOutcome::Deferred(deferred));
    }
    let count = errors.len();
    Err(
        ValidationError::new("any_failed", format!("All {count} alternatives failed"))
            .with_nested(errors),
    )
}

pub(super) fn validate_not(
    inner: RuleRef<'_>,
    input: &serde_json::Value,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationError> {
    match inner.validate_bounded(input, ctx, mode, disclosure) {
        Ok(EvaluationOutcome::Satisfied) => {
            Err(ValidationError::new("not_failed", "negated rule passed"))
        },
        Ok(deferred @ EvaluationOutcome::Deferred(_)) => Ok(deferred),
        Err(error) if error.is_violation() => Ok(EvaluationOutcome::Satisfied),
        Err(error) => Err(error),
    }
}
