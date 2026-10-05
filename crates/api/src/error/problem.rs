//! API-owned conversion from runtime diagnostics to RFC9457 wire data.

pub use nebula_api_contract::v1::problem::{ProblemDetails, ValidationFieldError};

/// Carry all five activation-diagnostic fields onto the wire without flattening them.
///
/// `detail` stays populated for a human reading the response, but every
/// field it summarises is also present on its own, so a client never has to
/// parse prose to recover a value.
pub(crate) fn validation_error_from_diagnostic(
    diagnostic: &nebula_error::ActivationDiagnostic,
) -> ValidationFieldError {
    ValidationFieldError {
        code: diagnostic.code().to_owned(),
        detail: format!(
            "{}: expected {}, found {}",
            diagnostic.code(),
            diagnostic.expected(),
            diagnostic.actual()
        ),
        pointer: None,
        path: Some(diagnostic.path().to_owned()),
        expected: Some(diagnostic.expected().to_owned()),
        actual: Some(diagnostic.actual().to_owned()),
        remediation: Some(diagnostic.remediation().to_owned()),
    }
}

/// Construct transport data from a server-owned HTTP status.
pub(crate) fn new_problem_details(
    type_uri: impl Into<String>,
    title: impl Into<String>,
    status: axum::http::StatusCode,
) -> ProblemDetails {
    ProblemDetails::new(type_uri, title, status.as_u16())
}
