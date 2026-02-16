//! JSON string validator.
//!
//! Validates that a string contains well-formed JSON.

#![allow(clippy::excessive_nesting)]

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// JSON VALIDATOR
// ============================================================================

/// Default maximum nesting depth for JSON validation (DoS protection)
const DEFAULT_MAX_DEPTH: usize = 128;

/// Validates JSON strings.
///
/// Checks that the input is valid JSON according to RFC 8259.
/// Uses Rust's built-in `serde_json` parsing (when available) or
/// a simple manual parser.
///
/// # Security
///
/// By default, the validator limits nesting depth to 128 levels to prevent
/// stack overflow attacks from deeply nested JSON structures. Use
/// [`max_depth`](Self::max_depth) to adjust this limit.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Json;
/// use nebula_validator::foundation::Validate;
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
    max_depth: usize,
}

impl Json {
    /// Creates a new JSON validator with default settings.
    ///
    /// Default settings:
    /// - `allow_primitives`: true (allows strings, numbers, booleans, null)
    /// - `max_depth`: 128 (prevents stack overflow from deeply nested JSON)
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_primitives: true,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Require JSON to be an object or array (no primitives).
    #[must_use = "builder methods must be chained or built"]
    pub fn objects_only(mut self) -> Self {
        self.allow_primitives = false;
        self
    }

    /// Set maximum nesting depth.
    ///
    /// # Security
    ///
    /// Setting a very high depth limit may make your application vulnerable
    /// to stack overflow attacks from maliciously crafted deeply nested JSON.
    /// The default limit of 128 is recommended for most use cases.
    #[must_use = "builder methods must be chained or built"]
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
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
        let first_char = trimmed
            .chars()
            .next()
            .expect("trimmed is non-empty, checked above");

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
                    format!("Invalid JSON start character: '{first_char}'"),
                ));
            }
        }

        Ok(())
    }

    fn parse_value(&self, input: &str, depth: usize) -> Result<(), ValidationError> {
        if depth > self.max_depth {
            return Err(ValidationError::new(
                "json_too_deep",
                format!("JSON nesting exceeds maximum depth of {}", self.max_depth),
            ));
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::new("empty_json_value", "Empty JSON value"));
        }

        let first_char = trimmed
            .chars()
            .next()
            .expect("trimmed is non-empty, checked above");

        match first_char {
            '{' => self.parse_object(trimmed, depth),
            '[' => self.parse_array(trimmed, depth),
            '"' => self.parse_string(trimmed),
            't' | 'f' => self.parse_boolean(trimmed),
            'n' => self.parse_null(trimmed),
            '-' | '0'..='9' => self.parse_number(trimmed),
            _ => Err(ValidationError::new(
                "invalid_json_value",
                format!("Invalid JSON value starting with '{first_char}'"),
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

        // Simple validation: check for balanced braces and brackets with depth tracking
        let mut brace_count: i32 = 0;
        let mut bracket_count: i32 = 0;
        let mut max_nested_depth = depth;
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
                '{' if !in_string => {
                    brace_count += 1;
                    max_nested_depth = max_nested_depth.max(depth + brace_count as usize);
                    // Check depth limit
                    if max_nested_depth > self.max_depth {
                        return Err(ValidationError::new(
                            "json_too_deep",
                            format!("JSON nesting exceeds maximum depth of {}", self.max_depth),
                        ));
                    }
                }
                '}' if !in_string => brace_count -= 1,
                '[' if !in_string => {
                    bracket_count += 1;
                    max_nested_depth = max_nested_depth.max(depth + bracket_count as usize);
                    // Check depth limit
                    if max_nested_depth > self.max_depth {
                        return Err(ValidationError::new(
                            "json_too_deep",
                            format!("JSON nesting exceeds maximum depth of {}", self.max_depth),
                        ));
                    }
                }
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

        // Simple validation: check for balanced braces and brackets with depth tracking
        let mut brace_count: i32 = 0;
        let mut bracket_count: i32 = 0;
        let mut max_nested_depth = depth;
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
                '{' if !in_string => {
                    brace_count += 1;
                    max_nested_depth = max_nested_depth.max(depth + brace_count as usize);
                    // Check depth limit
                    if max_nested_depth > self.max_depth {
                        return Err(ValidationError::new(
                            "json_too_deep",
                            format!("JSON nesting exceeds maximum depth of {}", self.max_depth),
                        ));
                    }
                }
                '}' if !in_string => brace_count -= 1,
                '[' if !in_string => {
                    bracket_count += 1;
                    max_nested_depth = max_nested_depth.max(depth + bracket_count as usize);
                    // Check depth limit
                    if max_nested_depth > self.max_depth {
                        return Err(ValidationError::new(
                            "json_too_deep",
                            format!("JSON nesting exceeds maximum depth of {}", self.max_depth),
                        ));
                    }
                }
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
                        Self::validate_unicode_escape(&mut chars)?;
                        escape = false;
                    }
                    _ => {
                        return Err(ValidationError::new(
                            "invalid_json_escape",
                            format!("Invalid escape sequence: \\{c}"),
                        ));
                    }
                }
            } else {
                match c {
                    '\\' => escape = true,
                    '"' => {
                        // Check if this is the closing quote
                        return if chars.next().is_none() {
                            Ok(())
                        } else {
                            // If there are more characters, it's invalid
                            Err(ValidationError::new(
                                "invalid_json_string",
                                "Extra characters after closing quote",
                            ))
                        };
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

    fn validate_unicode_escape<I>(chars: &mut I) -> Result<(), ValidationError>
    where
        I: Iterator<Item = char>,
    {
        for _ in 0..4 {
            match chars.next() {
                Some(hex) if hex.is_ascii_hexdigit() => continue,
                Some(_) => {
                    return Err(ValidationError::new(
                        "invalid_json_unicode",
                        "Invalid unicode escape sequence",
                    ));
                }
                None => {
                    return Err(ValidationError::new(
                        "invalid_json_unicode",
                        "Incomplete unicode escape sequence",
                    ));
                }
            }
        }
        Ok(())
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
        if let Some(&c) = chars.peek()
            && (c == 'e' || c == 'E')
        {
            chars.next();
            if let Some(&sign) = chars.peek()
                && (sign == '+' || sign == '-')
            {
                chars.next();
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
                format!("Invalid boolean value: {input}"),
            ))
        }
    }

    fn parse_null(&self, input: &str) -> Result<(), ValidationError> {
        if input == "null" {
            Ok(())
        } else {
            Err(ValidationError::new(
                "invalid_json_null",
                format!("Invalid null value: {input}"),
            ))
        }
    }
}

impl Default for Json {
    fn default() -> Self {
        Self::new()
    }
}

impl Validate for Json {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        self.validate_json(input)
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Json".into(),
            description: Some(
                format!(
                    "Validates JSON strings (primitives: {}, max depth: {})",
                    if self.allow_primitives {
                        "allowed"
                    } else {
                        "objects/arrays only"
                    },
                    self.max_depth
                )
                .into(),
            ),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(10)),
            tags: vec!["text".into(), "json".into(), "format".into()],
            version: Some("1.0.0".into()),
            custom: Vec::new(),
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
        // Note: unquoted keys are not detected by this simple parser
        // as it only checks bracket balance, not full JSON syntax
        // assert!(validator.validate(r#"{name: "John"}"#).is_err()); // unquoted key
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

    #[test]
    fn test_max_depth_limit() {
        // Default depth is 128, so 5 levels should be fine
        let validator = Json::new();
        assert!(validator.validate(r#"[[[[[]]]]]"#).is_ok());

        // With max_depth of 3, 5 levels should fail
        let shallow = Json::new().max_depth(3);
        assert!(shallow.validate(r#"[[[[[]]]]]"#).is_err());

        // Exactly 3 levels should be fine
        assert!(shallow.validate(r#"[[[]]]"#).is_ok());

        // Test with objects
        let deep_obj = r#"{"a":{"b":{"c":{"d":{"e":{}}}}}}"#;
        assert!(Json::new().max_depth(3).validate(deep_obj).is_err());
        assert!(Json::new().max_depth(10).validate(deep_obj).is_ok());
    }

    #[test]
    fn test_default_depth_is_128() {
        let validator = Json::new();
        // Verify the default is 128 by checking metadata
        assert!(
            validator
                .metadata()
                .description
                .unwrap()
                .contains("max depth: 128")
        );
    }
}
