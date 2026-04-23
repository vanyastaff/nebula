//! Shared validation helpers for credential endpoints.
//!
//! Centralizes input validation logic and constraint constants to avoid
//! duplication across CRUD, lifecycle, acquisition, and type discovery
//! controllers. All validators return [`ApiResult`] with structured
//! [`ApiError::Validation`] errors following RFC 9457.

use crate::errors::{ApiError, ApiResult};

// ── Constraint constants ────────────────────────────────────────────────

/// Maximum length for credential identifiers.
pub const MAX_CREDENTIAL_ID_LEN: usize = 128;

/// Maximum length for human-readable credential names.
pub const MAX_CREDENTIAL_NAME_LEN: usize = 256;

/// Maximum length for credential type keys (e.g. "oauth2", "api_key").
pub const MAX_CREDENTIAL_KEY_LEN: usize = 64;

/// Maximum items per page in list queries.
pub const MAX_PAGE_SIZE: usize = 100;

// ── Validators ──────────────────────────────────────────────────────────

/// Validate a credential ID from a path parameter.
///
/// Checks that the ID is non-empty and within the maximum length.
/// Returns [`ApiError::Validation`] on failure.
pub fn validate_credential_id(id: &str) -> ApiResult<()> {
    if id.is_empty() {
        return Err(ApiError::Validation {
            detail: "Credential ID must not be empty".to_string(),
            errors: vec![],
        });
    }
    if id.len() > MAX_CREDENTIAL_ID_LEN {
        return Err(ApiError::Validation {
            detail: format!(
                "Credential ID exceeds maximum length of {MAX_CREDENTIAL_ID_LEN} characters"
            ),
            errors: vec![],
        });
    }
    Ok(())
}

/// Validate a credential display name.
///
/// Trims whitespace and checks that the name is non-empty and within
/// the maximum length. Returns the trimmed name on success.
pub fn validate_credential_name(name: &str) -> ApiResult<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Validation {
            detail: "Credential name must not be empty".to_string(),
            errors: vec![],
        });
    }
    if trimmed.len() > MAX_CREDENTIAL_NAME_LEN {
        return Err(ApiError::Validation {
            detail: format!(
                "Credential name exceeds maximum length of {MAX_CREDENTIAL_NAME_LEN} characters"
            ),
            errors: vec![],
        });
    }
    Ok(trimmed)
}

/// Validate a credential type key (e.g. "oauth2", "api_key").
///
/// Checks that the key is non-empty and within the maximum length.
pub fn validate_credential_key(key: &str) -> ApiResult<()> {
    if key.is_empty() {
        return Err(ApiError::Validation {
            detail: "Credential type key must not be empty".to_string(),
            errors: vec![],
        });
    }
    if key.len() > MAX_CREDENTIAL_KEY_LEN {
        return Err(ApiError::Validation {
            detail: format!(
                "Credential type key exceeds maximum length of {MAX_CREDENTIAL_KEY_LEN} characters"
            ),
            errors: vec![],
        });
    }
    Ok(())
}

/// Validate that a JSON value is an object (used for credential data fields).
pub fn validate_data_is_object(data: &serde_json::Value) -> ApiResult<()> {
    if !data.is_object() {
        return Err(ApiError::Validation {
            detail: "data must be a JSON object containing the credential's input fields"
                .to_string(),
            errors: vec![],
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_credential_id() {
        assert!(validate_credential_id("cred_abc123").is_ok());
    }

    #[test]
    fn empty_credential_id_rejected() {
        assert!(validate_credential_id("").is_err());
    }

    #[test]
    fn oversized_credential_id_rejected() {
        let long = "x".repeat(MAX_CREDENTIAL_ID_LEN + 1);
        assert!(validate_credential_id(&long).is_err());
    }

    #[test]
    fn boundary_credential_id_accepted() {
        let exact = "x".repeat(MAX_CREDENTIAL_ID_LEN);
        assert!(validate_credential_id(&exact).is_ok());
    }

    #[test]
    fn valid_credential_name() {
        assert!(validate_credential_name("My API Key").is_ok());
    }

    #[test]
    fn whitespace_only_name_rejected() {
        assert!(validate_credential_name("   ").is_err());
    }

    #[test]
    fn name_trimmed_on_validation() {
        let result = validate_credential_name("  hello  ").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn valid_credential_key() {
        assert!(validate_credential_key("oauth2").is_ok());
        assert!(validate_credential_key("api_key").is_ok());
    }

    #[test]
    fn empty_credential_key_rejected() {
        assert!(validate_credential_key("").is_err());
    }

    #[test]
    fn data_object_accepted() {
        let obj = serde_json::json!({"key": "value"});
        assert!(validate_data_is_object(&obj).is_ok());
    }

    #[test]
    fn data_non_object_rejected() {
        let arr = serde_json::json!([1, 2, 3]);
        assert!(validate_data_is_object(&arr).is_err());
        let str_val = serde_json::json!("hello");
        assert!(validate_data_is_object(&str_val).is_err());
    }
}
