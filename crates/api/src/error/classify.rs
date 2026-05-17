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

    out.push(ValidationFieldError {
        code: err.code.to_string(),
        detail: err.message.to_string(),
        pointer: normalize_pointer(Some(&pointer)),
    });

    for nested in err.nested() {
        flatten_validation_error(nested, Some(&pointer), out);
    }
}

// ── Map a WorkflowError to a JSON Pointer ───────────────────────────────────

/// Map a [`nebula_workflow::WorkflowError`] to a JSON Pointer (RFC 6901)
/// that identifies the offending location in the workflow JSON document.
///
/// The workflow JSON schema is:
/// ```json
/// {
///   "nodes":       [ { "id": "<key>", … }, … ],
///   "connections": [ { "from_node": "<f>", "to_node": "<t>", … }, … ],
///   "trigger":     { … },
///   …
/// }
/// ```
///
/// Pointer conventions used here:
/// - Node-specific errors: `/nodes/<node_key>`
/// - Connection-specific: `/connections/<from>/<to>`
/// - Trigger errors:      `/trigger`
/// - Structural / whole-document errors: `""` (the root pointer, RFC 6901 §4)
pub(super) fn workflow_error_pointer(err: &nebula_workflow::WorkflowError) -> String {
    use nebula_workflow::WorkflowError;
    match err {
        // Node-keyed errors
        WorkflowError::DuplicateNodeKey(key)
        | WorkflowError::UnknownNode(key)
        | WorkflowError::SelfLoop(key) => format!("/nodes/{key}"),

        WorkflowError::InvalidParameterReference { node_key, .. } => {
            format!("/nodes/{node_key}")
        },

        WorkflowError::InvalidActionKey { key, .. } => {
            // The key string is the node's action_key; best we can do without
            // the node key is to point at the nodes array.
            let _ = key;
            "/nodes".to_owned()
        },

        // Connection-keyed errors
        WorkflowError::DuplicateConnection { from, to } => {
            format!("/connections/{from}/{to}")
        },

        // Trigger errors
        WorkflowError::InvalidTrigger { .. } => "/trigger".to_owned(),

        // Schema-level / structural — point at root
        WorkflowError::EmptyName
        | WorkflowError::NoNodes
        | WorkflowError::CycleDetected
        | WorkflowError::NoEntryNodes
        | WorkflowError::UnsupportedSchema { .. }
        | WorkflowError::InvalidOwnerId
        | WorkflowError::GraphError(_) => String::new(), // RFC 6901 root pointer

        // `WorkflowError` is `#[non_exhaustive]`. Future variants without an
        // API-side mapping fall back to the root pointer rather than failing
        // to compile the API layer on every validator extension.
        _ => String::new(),
    }
}

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
        Self::InsufficientRole {
            required_role: pd.required_role,
            current_role: pd.current_role,
        }
    }
}
