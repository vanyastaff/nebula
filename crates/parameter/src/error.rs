/// Error type for parameter operations.
///
/// Covers key validation, lookup, type mismatches, serialization,
/// and declarative validation failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
pub enum ParameterError {
    /// Parameter key does not meet naming rules.
    #[classify(category = "validation", code = "PARAM_INVALID_KEY")]
    #[error("invalid key format `{key}`: {reason}")]
    InvalidKeyFormat {
        /// The offending key string.
        key: String,
        /// Why the key is invalid.
        reason: String,
    },

    /// Parameter with the given key was not found.
    #[classify(category = "not_found", code = "PARAM_NOT_FOUND")]
    #[error("parameter not found: `{key}`")]
    NotFound {
        /// The missing key.
        key: String,
    },

    /// A parameter with the given key already exists.
    #[classify(category = "conflict", code = "PARAM_ALREADY_EXISTS")]
    #[error("parameter already exists: `{key}`")]
    AlreadyExists {
        /// The duplicate key.
        key: String,
    },

    /// Value type does not match the expected parameter type.
    #[classify(category = "validation", code = "PARAM_INVALID_TYPE")]
    #[error("invalid type for `{key}`: expected {expected_type}, got {actual_details}")]
    InvalidType {
        /// The parameter key.
        key: String,
        /// The type that was expected.
        expected_type: String,
        /// Description of the actual value type.
        actual_details: String,
    },

    /// Value is present but invalid for the parameter's constraints.
    #[classify(category = "validation", code = "PARAM_INVALID_VALUE")]
    #[error("invalid value for `{key}`: {reason}")]
    InvalidValue {
        /// The parameter key.
        key: String,
        /// Why the value is invalid.
        reason: String,
    },

    /// A required parameter has no value.
    #[classify(category = "validation", code = "PARAM_MISSING_VALUE")]
    #[error("missing value for required parameter `{key}`")]
    MissingValue {
        /// The missing parameter key.
        key: String,
    },

    /// Input contains a field that is not defined by the schema.
    #[classify(category = "validation", code = "PARAM_UNKNOWN_FIELD")]
    #[error("unknown field `{key}`")]
    UnknownField {
        /// The unrecognised field key.
        key: String,
    },

    /// A declarative validation rule failed with structured validator details.
    #[classify(category = "validation", code = "PARAM_VALIDATION_ISSUE")]
    #[error("validation failed for `{key}` [{code}]: {reason}")]
    ValidationIssue {
        /// The parameter key that failed validation.
        key: String,
        /// Machine-readable error code.
        code: String,
        /// Human-readable failure reason.
        reason: String,
        /// Additional key-value context for the failure.
        params: Vec<(String, String)>,
    },

    /// Failed to deserialize a parameter value.
    #[classify(category = "internal", code = "PARAM_DESER")]
    #[error("deserialization failed for `{key}`: {error}")]
    DeserializationError {
        /// The parameter key that failed deserialization.
        key: String,
        /// The underlying error message.
        error: String,
    },

    /// Failed to serialize a parameter value.
    #[classify(category = "internal", code = "PARAM_SER")]
    #[error("serialization failed: {error}")]
    SerializationError {
        /// The underlying error message.
        error: String,
    },
}

impl ParameterError {
    /// Returns structured validation code when available.
    #[must_use]
    pub fn validation_code(&self) -> Option<&str> {
        match self {
            Self::ValidationIssue { code, .. } => Some(code.as_str()),
            _ => None,
        }
    }

    /// Returns structured validation params when available.
    #[must_use]
    pub fn validation_params(&self) -> Option<&[(String, String)]> {
        match self {
            Self::ValidationIssue { params, .. } => Some(params.as_slice()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        let err = ParameterError::InvalidKeyFormat {
            key: "bad key".into(),
            reason: "contains space".into(),
        };
        assert_eq!(
            err.to_string(),
            "invalid key format `bad key`: contains space"
        );

        let err = ParameterError::NotFound {
            key: "email".into(),
        };
        assert_eq!(err.to_string(), "parameter not found: `email`");

        let err = ParameterError::AlreadyExists {
            key: "email".into(),
        };
        assert_eq!(err.to_string(), "parameter already exists: `email`");

        let err = ParameterError::InvalidType {
            key: "age".into(),
            expected_type: "number".into(),
            actual_details: "string \"abc\"".into(),
        };
        assert_eq!(
            err.to_string(),
            "invalid type for `age`: expected number, got string \"abc\""
        );

        let err = ParameterError::InvalidValue {
            key: "port".into(),
            reason: "must be 1..65535".into(),
        };
        assert_eq!(
            err.to_string(),
            "invalid value for `port`: must be 1..65535"
        );

        let err = ParameterError::MissingValue { key: "name".into() };
        assert_eq!(
            err.to_string(),
            "missing value for required parameter `name`"
        );

        let err = ParameterError::UnknownField {
            key: "unexpected".into(),
        };
        assert_eq!(err.to_string(), "unknown field `unexpected`");

        let err = ParameterError::ValidationIssue {
            key: "email".into(),
            code: "invalid_format".into(),
            reason: "Invalid format".into(),
            params: vec![("expected".into(), "email".into())],
        };
        assert_eq!(
            err.to_string(),
            "validation failed for `email` [invalid_format]: Invalid format"
        );

        let err = ParameterError::DeserializationError {
            key: "config".into(),
            error: "expected object".into(),
        };
        assert_eq!(
            err.to_string(),
            "deserialization failed for `config`: expected object"
        );

        let err = ParameterError::SerializationError {
            error: "recursive type".into(),
        };
        assert_eq!(err.to_string(), "serialization failed: recursive type");
    }

    #[test]
    fn classify_categories_are_correct() {
        use nebula_error::Classify;

        let not_found = ParameterError::NotFound { key: "x".into() };
        assert_eq!(not_found.category().as_str(), "not_found");

        let already = ParameterError::AlreadyExists { key: "x".into() };
        assert_eq!(already.category().as_str(), "conflict");

        let invalid = ParameterError::InvalidValue {
            key: "x".into(),
            reason: "bad".into(),
        };
        assert_eq!(invalid.category().as_str(), "validation");
    }

    #[test]
    fn classify_codes_start_with_param() {
        use nebula_error::Classify;

        let errors: Vec<ParameterError> = vec![
            ParameterError::InvalidKeyFormat {
                key: String::new(),
                reason: String::new(),
            },
            ParameterError::NotFound { key: String::new() },
            ParameterError::AlreadyExists { key: String::new() },
            ParameterError::SerializationError {
                error: String::new(),
            },
        ];

        for err in &errors {
            let code = err.code();
            assert!(
                code.as_str().starts_with("PARAM_"),
                "code should start with PARAM_: {code}"
            );
        }
    }

    #[test]
    fn none_are_retryable() {
        use nebula_error::Classify;

        let errors = vec![
            ParameterError::InvalidKeyFormat {
                key: String::new(),
                reason: String::new(),
            },
            ParameterError::NotFound { key: String::new() },
            ParameterError::SerializationError {
                error: String::new(),
            },
        ];

        for err in &errors {
            assert!(!err.is_retryable(), "should not be retryable: {:?}", err);
        }
    }

    #[test]
    fn validation_issue_helpers() {
        let err = ParameterError::ValidationIssue {
            key: "username".into(),
            code: "min_length".into(),
            reason: "min_length: Must be at least 5 characters".into(),
            params: vec![("min".into(), "5".into())],
        };

        assert_eq!(err.validation_code(), Some("min_length"));
        assert_eq!(
            err.validation_params(),
            Some(&[("min".into(), "5".into())][..])
        );

        let plain = ParameterError::InvalidValue {
            key: "username".into(),
            reason: "bad value".into(),
        };
        assert_eq!(plain.validation_code(), None);
        assert_eq!(plain.validation_params(), None);
    }
}
