/// Error type for parameter operations.
///
/// Covers key validation, lookup, type mismatches, serialization,
/// and declarative validation failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParameterError {
    /// Parameter key does not meet naming rules.
    #[error("invalid key format `{key}`: {reason}")]
    InvalidKeyFormat {
        /// The offending key string.
        key: String,
        /// Why the key is invalid.
        reason: String,
    },

    /// Parameter with the given key was not found.
    #[error("parameter not found: `{key}`")]
    NotFound {
        /// The missing key.
        key: String,
    },

    /// A parameter with the given key already exists.
    #[error("parameter already exists: `{key}`")]
    AlreadyExists {
        /// The duplicate key.
        key: String,
    },

    /// Value type does not match the expected parameter type.
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
    #[error("invalid value for `{key}`: {reason}")]
    InvalidValue {
        /// The parameter key.
        key: String,
        /// Why the value is invalid.
        reason: String,
    },

    /// A required parameter has no value.
    #[error("missing value for required parameter `{key}`")]
    MissingValue {
        /// The missing parameter key.
        key: String,
    },

    /// Input contains a field that is not defined by the schema.
    #[error("unknown field `{key}`")]
    UnknownField {
        /// The unrecognised field key.
        key: String,
    },

    /// A declarative validation rule failed.
    #[error("validation failed for `{key}`: {reason}")]
    ValidationError {
        /// The parameter key that failed validation.
        key: String,
        /// Human-readable failure reason.
        reason: String,
    },

    /// A declarative validation rule failed with structured validator details.
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
    #[error("deserialization failed for `{key}`: {error}")]
    DeserializationError {
        /// The parameter key that failed deserialization.
        key: String,
        /// The underlying error message.
        error: String,
    },

    /// Failed to serialize a parameter value.
    #[error("serialization failed: {error}")]
    SerializationError {
        /// The underlying error message.
        error: String,
    },
}

impl ParameterError {
    /// Broad error category for grouping in logs and metrics.
    #[must_use]
    pub fn category(&self) -> &str {
        match self {
            Self::InvalidKeyFormat { .. } => "format",
            Self::NotFound { .. } => "lookup",
            Self::AlreadyExists { .. } => "lookup",
            Self::InvalidType { .. } => "type",
            Self::InvalidValue { .. } => "value",
            Self::MissingValue { .. } => "value",
            Self::UnknownField { .. } => "value",
            Self::ValidationError { .. } => "validation",
            Self::ValidationIssue { .. } => "validation",
            Self::DeserializationError { .. } => "serialization",
            Self::SerializationError { .. } => "serialization",
        }
    }

    /// Machine-readable error code for programmatic handling.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidKeyFormat { .. } => "PARAM_INVALID_KEY",
            Self::NotFound { .. } => "PARAM_NOT_FOUND",
            Self::AlreadyExists { .. } => "PARAM_ALREADY_EXISTS",
            Self::InvalidType { .. } => "PARAM_INVALID_TYPE",
            Self::InvalidValue { .. } => "PARAM_INVALID_VALUE",
            Self::MissingValue { .. } => "PARAM_MISSING_VALUE",
            Self::UnknownField { .. } => "PARAM_UNKNOWN_FIELD",
            Self::ValidationError { .. } => "PARAM_VALIDATION",
            Self::ValidationIssue { .. } => "PARAM_VALIDATION_ISSUE",
            Self::DeserializationError { .. } => "PARAM_DESER",
            Self::SerializationError { .. } => "PARAM_SER",
        }
    }

    /// Whether the operation might succeed if retried with the same input.
    ///
    /// All parameter errors are deterministic — same input, same result.
    /// Returns `false` for every variant.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        false
    }

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

        let err = ParameterError::ValidationError {
            key: "url".into(),
            reason: "not a valid URL".into(),
        };
        assert_eq!(
            err.to_string(),
            "validation failed for `url`: not a valid URL"
        );

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
    fn categories_are_consistent() {
        let cases: Vec<(ParameterError, &str)> = vec![
            (
                ParameterError::InvalidKeyFormat {
                    key: String::new(),
                    reason: String::new(),
                },
                "format",
            ),
            (ParameterError::NotFound { key: String::new() }, "lookup"),
            (
                ParameterError::AlreadyExists { key: String::new() },
                "lookup",
            ),
            (
                ParameterError::InvalidType {
                    key: String::new(),
                    expected_type: String::new(),
                    actual_details: String::new(),
                },
                "type",
            ),
            (
                ParameterError::InvalidValue {
                    key: String::new(),
                    reason: String::new(),
                },
                "value",
            ),
            (ParameterError::MissingValue { key: String::new() }, "value"),
            (ParameterError::UnknownField { key: String::new() }, "value"),
            (
                ParameterError::ValidationError {
                    key: String::new(),
                    reason: String::new(),
                },
                "validation",
            ),
            (
                ParameterError::ValidationIssue {
                    key: String::new(),
                    code: String::new(),
                    reason: String::new(),
                    params: Vec::new(),
                },
                "validation",
            ),
            (
                ParameterError::DeserializationError {
                    key: String::new(),
                    error: String::new(),
                },
                "serialization",
            ),
            (
                ParameterError::SerializationError {
                    error: String::new(),
                },
                "serialization",
            ),
        ];

        for (err, expected_cat) in &cases {
            assert_eq!(err.category(), *expected_cat, "for {:?}", err);
        }
    }

    #[test]
    fn codes_are_unique_per_variant() {
        let errors = vec![
            ParameterError::InvalidKeyFormat {
                key: String::new(),
                reason: String::new(),
            },
            ParameterError::NotFound { key: String::new() },
            ParameterError::AlreadyExists { key: String::new() },
            ParameterError::InvalidType {
                key: String::new(),
                expected_type: String::new(),
                actual_details: String::new(),
            },
            ParameterError::InvalidValue {
                key: String::new(),
                reason: String::new(),
            },
            ParameterError::MissingValue { key: String::new() },
            ParameterError::ValidationError {
                key: String::new(),
                reason: String::new(),
            },
            ParameterError::ValidationIssue {
                key: String::new(),
                code: String::new(),
                reason: String::new(),
                params: Vec::new(),
            },
            ParameterError::DeserializationError {
                key: String::new(),
                error: String::new(),
            },
            ParameterError::SerializationError {
                error: String::new(),
            },
        ];

        let codes: Vec<&str> = errors.iter().map(|e| e.code()).collect();

        for code in &codes {
            assert!(
                code.starts_with("PARAM_"),
                "code should start with PARAM_: {code}"
            );
        }

        let mut sorted = codes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "codes should be unique");
    }

    #[test]
    fn none_are_retryable() {
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
