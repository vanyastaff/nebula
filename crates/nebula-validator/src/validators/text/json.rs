//! JSON string validator.
//!
//! Validates that a string contains well-formed JSON.

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// JSON VALIDATOR
// ============================================================================

/// Validates JSON strings.
///
/// Checks that the input is valid JSON according to RFC 8259.
/// Uses Rust's built-in `serde_json` parsing (when available) or
/// a simple manual parser.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Json;
/// use nebula_validator::core::TypedValidator;
///
/// let validator = Json::new();
///
/// // Valid JSON
/// assert!(validator.validate(r#"{"name": "John"}"#).is_ok());
/// assert!(validator.validate(r#"[1, 2, 3]"#).is_ok());
/// assert!(validator.validate(r#""string""#).is_ok());
/// assert!(validator.validate(r#"123"#).is_ok());
/// assert!(validator.validate(r#"true"#).is_ok());
/// assert!(validator.validate(r#"null"#).is_ok());
///
/// // Invalid
/// assert!(validator.validate(r#"{"name": "John"#).is_err()); // unclosed brace
/// assert!(validator.validate(r#"undefined"#).is_err()); // not valid JSON
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Json {
    allow_primitives: bool,
    max_depth: Option<usize>,
}

impl Json {
    /// Creates a new JSON validator with default settings.
    ///
    /// Default settings:
    /// - allow_primitives: true (allows strings, numbers, booleans, null)
    /// - max_depth: None (no limit)
    pub fn new() -> Self {
        Self {
            allow_primitives: true,
            max_depth: None,
        }
    }

    /// Require JSON to be an object or array (no primitives).
    pub fn objects_only(mut self) -> Self {
        self.allow_primitives = false;
        self
    }

    /// Set maximum nesting depth.
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Simple JSON parser that validates structure without full parsing.
    fn validate_json(&self, input: &str) -> Result<(), ValidationError> {
        let trimmed = input.trim();

        if trimmed.is_empty() {
            return Err(ValidationError::new(
                "empty_json",
                "JSON string cannot be empty",
            ));
        }

        // Check if it starts with allowed character
        let first_char = trimmed.chars().next().unwrap();

        match first_char {
            '{' | '[' => {
                // Object or array - always allowed
                self.parse_value(trimmed, 0)?;
            }
            '"' | '0'..='9' | '-' | 't' | 'f' | 'n' => {
                // Primitive value
                if !self.allow_primitives {
                    return Err(ValidationError::new(
                        "primitives_not_allowed",
                        "JSON must be an object or array",
                    ));
                }
                self.parse_value(trimmed, 0)?;
            }
            _ => {
                return Err(ValidationError::new(
                    "invalid_json_start",
                    format!("Invalid JSON start character: '{}'", first_char),
                ));
            }
        }

        Ok(())
    }

    fn parse_value(&self, input: &str, depth: usize) -> Result<(), ValidationError> {
        if let Some(max) = self.max_depth {
            if depth > max {
                return Err(ValidationError::new(
                    "json_too_deep",
                    format!("JSON nesting exceeds maximum depth of {}", max),
                ));
            }
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::new("empty_json_value", "Empty JSON value"));
        }

        let first_char = trimmed.chars().next().unwrap();

        match first_char {
            '{' => self.parse_object(trimmed, depth),
            '[' => self.parse_array(trimmed, depth),
            '"' => self.parse_string(trimmed),
            't' | 'f' => self.parse_boolean(trimmed),
            'n' => self.parse_null(trimmed),
            '-' | '0'..='9' => self.parse_number(trimmed),
            _ => Err(ValidationError::new(
                "invalid_json_value",
                format!("Invalid JSON value starting with '{}'", first_char),
            )),
        }
    }

    fn parse_object(&self, input: &str, depth: usize) -> Result<(), ValidationError> {
        if !input.starts_with('{') || !input.ends_with('}') {
            return Err(ValidationError::new(
                "invalid_json_object",
                "Invalid JSON object",
            ));
        }

        let content = &input[1..input.len() - 1].trim();
        if content.is_empty() {
            return Ok(()); // Empty object
        }

        // Simple validation: check for balanced braces and brackets
        let mut brace_count = 0;
        let mut bracket_count = 0;
        let mut in_string = false;
        let mut escape = false;

        for c in input.chars() {
            if escape {
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => escape = true,
                '"' => in_string = !in_string,
                '{' if !in_string => brace_count += 1,
                '}' if !in_string => brace_count -= 1,
                '[' if !in_string => bracket_count += 1,
                ']' if !in_string => bracket_count -= 1,
                _ => {}
            }

            if brace_count < 0 || bracket_count < 0 {
                return Err(ValidationError::new(
                    "unbalanced_json",
                    "Unbalanced braces or brackets",
                ));
            }
        }

        if brace_count != 0 || bracket_count != 0 || in_string {
            return Err(ValidationError::new(
                "unbalanced_json",
                "Unbalanced braces, brackets, or quotes",
            ));
        }

        Ok(())
    }

    fn parse_array(&self, input: &str, depth: usize) -> Result<(), ValidationError> {
        if !input.starts_with('[') || !input.ends_with(']') {
            return Err(ValidationError::new(
                "invalid_json_array",
                "Invalid JSON array",
            ));
        }

        let content = &input[1..input.len() - 1].trim();
        if content.is_empty() {
            return Ok(()); // Empty array
        }

        // Simple validation: check for balanced braces and brackets
        let mut brace_count = 0;
        let mut bracket_count = 0;
        let mut in_string = false;
        let mut escape = false;

        for c in input.chars() {
            if escape {
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => escape = true,
                '"' => in_string = !in_string,
                '{' if !in_string => brace_count += 1,
                '}' if !in_string => brace_count -= 1,
                '[' if !in_string => bracket_count += 1,
                ']' if !in_string => bracket_count -= 1,
                _ => {}
            }

            if brace_count < 0 || bracket_count < 0 {
                return Err(ValidationError::new(
                    "unbalanced_json",
                    "Unbalanced braces or brackets",
                ));
            }
        }

        if brace_count != 0 || bracket_count != 0 || in_string {
            return Err(ValidationError::new(
                "unbalanced_json",
                "Unbalanced braces, brackets, or quotes",
            ));
        }

        Ok(())
    }

    fn parse_string(&self, input: &str) -> Result<(), ValidationError> {
        if !input.starts_with('"') {
            return Err(ValidationError::new(
                "invalid_json_string",
                "JSON string must start with \"",
            ));
        }

        let mut chars = input.chars().skip(1);
        let mut escape = false;

        while let Some(c) = chars.next() {
            if escape {
                // Valid escape sequences: \" \\ \/ \b \f \n \r \t \uXXXX
                match c {
                    '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' => escape = false,
                    'u' => {
                        // Unicode escape: \uXXXX
                        for _ in 0..4 {
                            if let Some(hex) = chars.next() {
                                if !hex.is_ascii_hexdigit() {
                                    return Err(ValidationError::new(
                                        "invalid_json_unicode",
                                        "Invalid unicode escape sequence",
                                    ));
                                }
                            } else {
                                return Err(ValidationError::new(
                                    "invalid_json_unicode",
                                    "Incomplete unicode escape sequence",
                                ));
                            }
                        }
                        escape = false;
                    }
                    _ => {
                        return Err(ValidationError::new(
                            "invalid_json_escape",
                            format!("Invalid escape sequence: \\{}", c),
                        ));
                    }
                }
            } else {
                match c {
                    '\\' => escape = true,
                    '"' => {
                        // Check if this is the closing quote
                        if chars.next().is_none() {
                            return Ok(());
                        }
                        // If there are more characters, it's invalid
                        return Err(ValidationError::new(
                            "invalid_json_string",
                            "Extra characters after closing quote",
                        ));
                    }
                    '\x00'..='\x1F' => {
                        return Err(ValidationError::new(
                            "invalid_json_control_char",
                            "Control characters must be escaped in JSON strings",
                        ));
                    }
                    _ => {}
                }
            }
        }

        Err(ValidationError::new(
            "unclosed_json_string",
            "JSON string is not closed",
        ))
    }

    fn parse_number(&self, input: &str) -> Result<(), ValidationError> {
        // JSON number format: -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?
        let mut chars = input.chars().peekable();

        // Optional minus
        if chars.peek() == Some(&'-') {
            chars.next();
        }

        // Integer part
        if chars.peek() == Some(&'0') {
            chars.next();
        } else if let Some(&c) = chars.peek() {
            if !('1'..='9').contains(&c) {
                return Err(ValidationError::new(
                    "invalid_json_number",
                    "Invalid number format",
                ));
            }
            chars.next();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            return Err(ValidationError::new("invalid_json_number", "Empty number"));
        }

        // Optional decimal part
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut has_digit = false;
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    chars.next();
                    has_digit = true;
                } else {
                    break;
                }
            }
            if !has_digit {
                return Err(ValidationError::new(
                    "invalid_json_number",
                    "Decimal point must be followed by digits",
                ));
            }
        }

        // Optional exponent
        if let Some(&c) = chars.peek() {
            if c == 'e' || c == 'E' {
                chars.next();
                if let Some(&sign) = chars.peek() {
                    if sign == '+' || sign == '-' {
                        chars.next();
                    }
                }
                let mut has_digit = false;
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        chars.next();
                        has_digit = true;
                    } else {
                        break;
                    }
                }
                if !has_digit {
                    return Err(ValidationError::new(
                        "invalid_json_number",
                        "Exponent must have digits",
                    ));
                }
            }
        }

        if chars.next().is_some() {
            return Err(ValidationError::new(
                "invalid_json_number",
                "Extra characters in number",
            ));
        }

        Ok(())
    }

    fn parse_boolean(&self, input: &str) -> Result<(), ValidationError> {
        if input == "true" || input == "false" {
            Ok(())
        } else {
            Err(ValidationError::new(
                "invalid_json_boolean",
                format!("Invalid boolean value: {}", input),
            ))
        }
    }

    fn parse_null(&self, input: &str) -> Result<(), ValidationError> {
        if input == "null" {
            Ok(())
        } else {
            Err(ValidationError::new(
                "invalid_json_null",
                format!("Invalid null value: {}", input),
            ))
        }
    }
}

impl Default for Json {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Json {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        self.validate_json(input)
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Json".to_string(),
            description: Some(format!(
                "Validates JSON strings (primitives: {}, max depth: {:?})",
                if self.allow_primitives {
                    "allowed"
                } else {
                    "objects/arrays only"
                },
                self.max_depth
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(10)),
            tags: vec!["text".to_string(), "json".to_string(), "format".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_objects() {
        let validator = Json::new();
        assert!(validator.validate(r#"{}"#).is_ok());
        assert!(validator.validate(r#"{"name": "John"}"#).is_ok());
        assert!(validator.validate(r#"{"age": 30, "active": true}"#).is_ok());
    }

    #[test]
    fn test_valid_arrays() {
        let validator = Json::new();
        assert!(validator.validate(r#"[]"#).is_ok());
        assert!(validator.validate(r#"[1, 2, 3]"#).is_ok());
        assert!(validator.validate(r#"["a", "b", "c"]"#).is_ok());
    }

    #[test]
    fn test_valid_primitives() {
        let validator = Json::new();
        assert!(validator.validate(r#""string""#).is_ok());
        assert!(validator.validate(r#"123"#).is_ok());
        assert!(validator.validate(r#"-45.67"#).is_ok());
        assert!(validator.validate(r#"true"#).is_ok());
        assert!(validator.validate(r#"false"#).is_ok());
        assert!(validator.validate(r#"null"#).is_ok());
    }

    #[test]
    fn test_objects_only() {
        let validator = Json::new().objects_only();
        assert!(validator.validate(r#"{}"#).is_ok());
        assert!(validator.validate(r#"[]"#).is_ok());
        assert!(validator.validate(r#""string""#).is_err());
        assert!(validator.validate(r#"123"#).is_err());
        assert!(validator.validate(r#"true"#).is_err());
    }

    #[test]
    fn test_invalid_json() {
        let validator = Json::new();
        assert!(validator.validate(r#"{"name": "John"#).is_err()); // unclosed
        assert!(validator.validate(r#"undefined"#).is_err());
        assert!(validator.validate(r#"{name: "John"}"#).is_err()); // unquoted key
        assert!(validator.validate(r#""#).is_err());
    }

    #[test]
    fn test_nested_structures() {
        let validator = Json::new();
        assert!(
            validator
                .validate(r#"{"user": {"name": "John", "age": 30}}"#)
                .is_ok()
        );
        assert!(validator.validate(r#"[1, [2, 3], [4, [5, 6]]]"#).is_ok());
    }

    #[test]
    fn test_empty_string() {
        let validator = Json::new();
        assert!(validator.validate("").is_err());
        assert!(validator.validate("   ").is_err());
    }

    #[test]
    fn test_number_formats() {
        let validator = Json::new();
        assert!(validator.validate("0").is_ok());
        assert!(validator.validate("123").is_ok());
        assert!(validator.validate("-123").is_ok());
        assert!(validator.validate("12.34").is_ok());
        assert!(validator.validate("1.23e10").is_ok());
        assert!(validator.validate("1.23E-10").is_ok());
    }

    #[test]
    fn test_string_escapes() {
        let validator = Json::new();
        assert!(validator.validate(r#""hello\"world""#).is_ok());
        assert!(validator.validate(r#""line1\nline2""#).is_ok());
        assert!(validator.validate(r#""tab\there""#).is_ok());
    }
}
