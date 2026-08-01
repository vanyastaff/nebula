//! From-conversions from domain error types → [`ApiError`].
//!
//! This module houses the [`nebula_error::Classify`] integration and every
//! `From<…>` implementation that maps crate-external error types
//! (`nebula_storage::*`, `nebula_core::*`, `nebula_validator::*`,
//! `nebula_workflow::*`) into [`ApiError`].

use nebula_validator::foundation::{ValidationError, ValidationErrors};

use super::{ApiError, problem::ValidationFieldError};

// ── ValidationError helpers ──────────────────────────────────────────────────

pub(super) fn normalize_pointer(pointer: Option<&str>) -> String {
    let pointer = pointer.unwrap_or("/").trim();
    if pointer.is_empty() || pointer == "#" {
        return "/".to_owned();
    }

    if let Some(rest) = pointer.strip_prefix('#') {
        if rest.is_empty() {
            return "/".to_owned();
        }
        if rest.starts_with('/') {
            return rest.to_owned();
        }
    }

    if pointer.starts_with('/') {
        pointer.to_owned()
    } else {
        format!("/{pointer}")
    }
}

pub(super) fn flatten_validation_error(
    err: &ValidationError,
    inherited_pointer: Option<&str>,
    out: &mut Vec<ValidationFieldError>,
) {
    let pointer = err
        .field_pointer()
        .map(std::borrow::Cow::into_owned)
        .or_else(|| inherited_pointer.map(str::to_owned))
        .unwrap_or_else(|| "/".to_owned());

    out.push(ValidationFieldError::field(
        err.code.to_string(),
        err.message.to_string(),
        normalize_pointer(Some(&pointer)),
    ));

    for nested in err.nested() {
        flatten_validation_error(nested, Some(&pointer), out);
    }
}

// ── Map a WorkflowError to a JSON Pointer ───────────────────────────────────

// ── From<ValidationError> ────────────────────────────────────────────────────

impl From<ValidationError> for ApiError {
    fn from(value: ValidationError) -> Self {
        let mut errors = Vec::new();
        flatten_validation_error(&value, None, &mut errors);
        let detail = if value.code.is_empty() {
            value.message.to_string()
        } else {
            format!("[{}] {}", value.code, value.message)
        };

        Self::Validation { detail, errors }
    }
}

impl From<ValidationErrors> for ApiError {
    fn from(value: ValidationErrors) -> Self {
        let mut errors = Vec::new();
        for item in value.errors() {
            flatten_validation_error(item, None, &mut errors);
        }

        Self::Validation {
            detail: format!("Validation failed with {} error(s)", errors.len()),
            errors,
        }
    }
}

// ── From<nebula_core::*> ─────────────────────────────────────────────────────

impl From<nebula_core::PermissionDenied> for ApiError {
    fn from(pd: nebula_core::PermissionDenied) -> Self {
        let (required_role, current_role) = match &pd.denial {
            nebula_core::PermissionDenial::Workspace { required, current } => (
                required.to_string(),
                current
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "none".to_owned()),
            ),
            nebula_core::PermissionDenial::Org { required, current } => (
                required.to_string(),
                current
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "none".to_owned()),
            ),
            // `PermissionDenial` is `#[non_exhaustive]`; future variants forward their
            // human-readable Display output as `required_role`. `current_role` uses
            // the same string so neither field is silently left blank/half-populated.
            _ => {
                let display = pd.denial.to_string();
                (display.clone(), display)
            },
        };
        Self::InsufficientRole {
            required_role,
            current_role,
        }
    }
}
