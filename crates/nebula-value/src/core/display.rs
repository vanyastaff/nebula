//! Display and Debug implementations for Value
//!
//! This module provides human-readable formatting for all Value types.

use crate::core::value::Value;
use std::fmt;

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),

            Value::Boolean(b) => write!(f, "{}", b),

            Value::Integer(i) => write!(f, "{}", i.value()),

            Value::Float(fl) => {
                if fl.is_nan() {
                    write!(f, "NaN")
                } else if fl.is_positive_infinity() {
                    write!(f, "+Infinity")
                } else if fl.is_negative_infinity() {
                    write!(f, "-Infinity")
                } else {
                    write!(f, "{}", fl.value())
                }
            }

            Value::Decimal(d) => write!(f, "{}", d),

            Value::Text(t) => write!(f, "{}", t.as_str()),

            Value::Bytes(b) => {
                // Display as base64
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(b.as_slice());
                write!(f, "Bytes({})", encoded)
            }

            Value::Array(arr) => {
                write!(f, "[")?;
                let mut first = true;
                for item in arr.iter() {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    // Array/Object use serde_json::Value internally
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }

            Value::Object(obj) => {
                write!(f, "{{")?;
                let mut first = true;
                for (key, value) in obj.entries() {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    // Array/Object use serde_json::Value internally
                    write!(f, "{}: {}", key, value)?;
                }
                write!(f, "}}")
            }

            #[cfg(feature = "temporal")]
            Value::Date(d) => write!(f, "{}", d),

            #[cfg(feature = "temporal")]
            Value::Time(t) => write!(f, "{}", t),

            #[cfg(feature = "temporal")]
            Value::DateTime(dt) => write!(f, "{}", dt),

            #[cfg(feature = "temporal")]
            Value::Duration(dur) => write!(f, "{}", dur),
        }
    }
}

/// Pretty-print formatting options
#[derive(Debug, Clone, Copy)]
pub struct PrettyConfig {
    /// Indentation string (e.g., "  " or "\t")
    pub indent: &'static str,
    /// Maximum depth before collapsing
    pub max_depth: Option<usize>,
    /// Maximum array/object items to show
    pub max_items: Option<usize>,
}

impl Default for PrettyConfig {
    fn default() -> Self {
        Self {
            indent: "  ",
            max_depth: None,
            max_items: None,
        }
    }
}

impl PrettyConfig {
    /// Compact configuration (minimal whitespace)
    pub const fn compact() -> Self {
        Self {
            indent: "",
            max_depth: None,
            max_items: None,
        }
    }

    /// Standard pretty-print configuration
    pub const fn pretty() -> Self {
        Self {
            indent: "  ",
            max_depth: None,
            max_items: None,
        }
    }

    /// Compact configuration with limits (for large values)
    pub const fn compact_limited() -> Self {
        Self {
            indent: "",
            max_depth: Some(10),
            max_items: Some(100),
        }
    }
}

impl Value {
    /// Format this value with custom configuration
    pub fn format_with(&self, config: &PrettyConfig) -> String {
        let mut output = String::new();
        self.format_recursive(&mut output, config, 0).ok();
        output
    }

    /// Pretty-print with default configuration
    pub fn pretty_print(&self) -> String {
        self.format_with(&PrettyConfig::pretty())
    }

    fn format_recursive(
        &self,
        output: &mut String,
        config: &PrettyConfig,
        depth: usize,
    ) -> fmt::Result {
        use std::fmt::Write;

        // Check max depth
        if let Some(max) = config.max_depth {
            if depth >= max {
                return write!(output, "...");
            }
        }

        match self {
            Value::Null => write!(output, "null"),
            Value::Boolean(b) => write!(output, "{}", b),
            Value::Integer(i) => write!(output, "{}", i.value()),
            Value::Float(f) => {
                if f.is_nan() {
                    write!(output, "NaN")
                } else if f.is_positive_infinity() {
                    write!(output, "+Infinity")
                } else if f.is_negative_infinity() {
                    write!(output, "-Infinity")
                } else {
                    write!(output, "{}", f.value())
                }
            }
            Value::Decimal(d) => write!(output, "{}", d),
            Value::Text(t) => write!(output, "\"{}\"", t.as_str()),
            Value::Bytes(b) => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(b.as_slice());
                write!(output, "Bytes({})", encoded)
            }
            #[cfg(feature = "temporal")]
            Value::Date(d) => write!(output, "\"{}\"", d),
            #[cfg(feature = "temporal")]
            Value::Time(t) => write!(output, "\"{}\"", t),
            #[cfg(feature = "temporal")]
            Value::DateTime(dt) => write!(output, "\"{}\"", dt),
            #[cfg(feature = "temporal")]
            Value::Duration(dur) => write!(output, "\"{}\"", dur),
            Value::Array(arr) => {
                write!(output, "[")?;

                let len = arr.len();
                let limit = config.max_items.unwrap_or(len);

                if !config.indent.is_empty() && len > 0 {
                    writeln!(output)?;
                }

                for (idx, item) in arr.iter().enumerate().take(limit) {
                    if !config.indent.is_empty() {
                        for _ in 0..=depth {
                            write!(output, "{}", config.indent)?;
                        }
                    }

                    // Array/Object use serde_json::Value internally
                    write!(output, "{}", item)?;

                    if idx < len.min(limit) - 1 {
                        write!(output, ",")?;
                    }

                    if !config.indent.is_empty() {
                        writeln!(output)?;
                    }
                }

                if len > limit {
                    if !config.indent.is_empty() {
                        for _ in 0..=depth {
                            write!(output, "{}", config.indent)?;
                        }
                    }
                    write!(output, "... ({} more)", len - limit)?;
                    if !config.indent.is_empty() {
                        writeln!(output)?;
                    }
                }

                if !config.indent.is_empty() && len > 0 {
                    for _ in 0..depth {
                        write!(output, "{}", config.indent)?;
                    }
                }

                write!(output, "]")
            }
            Value::Object(obj) => {
                write!(output, "{{")?;

                let len = obj.len();
                let limit = config.max_items.unwrap_or(len);

                if !config.indent.is_empty() && len > 0 {
                    writeln!(output)?;
                }

                for (idx, (key, value)) in obj.entries().enumerate().take(limit) {
                    if !config.indent.is_empty() {
                        for _ in 0..=depth {
                            write!(output, "{}", config.indent)?;
                        }
                    }

                    // Array/Object use serde_json::Value internally
                    write!(output, "\"{}\": {}", key, value)?;

                    if idx < len.min(limit) - 1 {
                        write!(output, ",")?;
                    }

                    if !config.indent.is_empty() {
                        writeln!(output)?;
                    }
                }

                if len > limit {
                    if !config.indent.is_empty() {
                        for _ in 0..=depth {
                            write!(output, "{}", config.indent)?;
                        }
                    }
                    write!(output, "... ({} more)", len - limit)?;
                    if !config.indent.is_empty() {
                        writeln!(output)?;
                    }
                }

                if !config.indent.is_empty() && len > 0 {
                    for _ in 0..depth {
                        write!(output, "{}", config.indent)?;
                    }
                }

                write!(output, "}}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scalar::{Integer, Float, Text};

    #[test]
    fn test_display_null() {
        let val = Value::Null;
        assert_eq!(val.to_string(), "null");
    }

    #[test]
    fn test_display_boolean() {
        assert_eq!(Value::Boolean(true).to_string(), "true");
        assert_eq!(Value::Boolean(false).to_string(), "false");
    }

    #[test]
    fn test_display_integer() {
        let val = Value::integer(42);
        assert_eq!(val.to_string(), "42");
    }

    #[test]
    fn test_display_float() {
        let val = Value::float(3.14);
        assert_eq!(val.to_string(), "3.14");
    }

    #[test]
    fn test_display_nan() {
        let val = Value::float(f64::NAN);
        assert_eq!(val.to_string(), "NaN");
    }

    #[test]
    fn test_display_infinity() {
        let pos_inf = Value::float(f64::INFINITY);
        assert_eq!(pos_inf.to_string(), "+Infinity");

        let neg_inf = Value::float(f64::NEG_INFINITY);
        assert_eq!(neg_inf.to_string(), "-Infinity");
    }

    #[test]
    fn test_display_text() {
        let val = Value::text("hello world");
        assert_eq!(val.to_string(), "hello world");
    }

    #[test]
    fn test_display_bytes() {
        let val = Value::bytes(vec![1, 2, 3]);
        let display = val.to_string();
        assert!(display.starts_with("Bytes("));
        assert!(display.contains("AQID")); // Base64 of [1, 2, 3]
    }

    #[test]
    fn test_pretty_print_simple() {
        let val = Value::integer(42);
        let pretty = val.pretty_print();
        assert_eq!(pretty, "42");
    }

    #[test]
    fn test_format_compact() {
        let val = Value::integer(42);
        let compact = val.format_with(&PrettyConfig::compact());
        assert_eq!(compact, "42");
    }
}