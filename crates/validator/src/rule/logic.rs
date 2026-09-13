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

pub(super) fn validate_all(
    children: RuleChildren<'_>,
    input: &serde_json::Value,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationError> {
    let mut errors = Vec::new();
    let mut deferred = Vec::new();
    for child in children {
        match child.validate_bounded(input, ctx, mode, disclosure) {
            Ok(EvaluationOutcome::Satisfied) => {},
            Ok(EvaluationOutcome::Deferred(reasons)) => deferred.extend(reasons),
            Err(error) if error.kind() != ValidationErrorKind::Violation => return Err(error),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(EvaluationOutcome::from_deferred(deferred))
    } else if errors.len() == 1 {
        Err(errors.remove(0))
    } else {
        let count = errors.len();
        Err(
            ValidationError::new("all_failed", format!("{count} of the rules failed"))
                .with_nested(errors),
        )
    }
}

pub(super) fn validate_any(
    children: RuleChildren<'_>,
    input: &serde_json::Value,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationError> {
    let mut errors = Vec::new();
    let mut deferred = Vec::new();
    let mut satisfied = false;
    for child in children {
        match child.validate_bounded(input, ctx, mode, disclosure) {
            Ok(EvaluationOutcome::Satisfied) => satisfied = true,
            Ok(EvaluationOutcome::Deferred(reasons)) => deferred.extend(reasons),
            Err(error) if error.kind() != ValidationErrorKind::Violation => return Err(error),
            Err(error) => errors.push(error),
        }
    }
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
        Err(error) if error.kind() == ValidationErrorKind::Violation => {
            Ok(EvaluationOutcome::Satisfied)
        },
        Err(error) => Err(error),
    }
}
