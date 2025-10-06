#![warn(clippy::all)]
#![warn(missing_docs)]
// Core modules
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//! # nebula-expression
//! ## Built-in Functions
//! ## Quick Start
//! ## With Caching
//! ### Array Functions
//! ### Conversion Functions
//! ### Math Functions
//! ### Object Functions
//! ### String Functions
//! ### Utility Functions
//! - Arithmetic operators: `+`, `-`, `*`, `/`, `%`, `**`
//! - Comparison operators: `==`, `!=`, `>`, `<`, `>=`, `<=`, `=~`
//! - Conditionals: `if condition then value1 else value2`
//! - Function calls: `functionName(arg1, arg2)`
//! - Index access: `array[0]`, `object['key']`
//! - Lambda expressions: `x => x > 5` (in filter/map/reduce)
//! - Logical operators: `&&`, `||`, `!`
//! - Pipeline operator: `|` for function chaining
//! - Property access: `$node.data`, `$execution.id`
//! - Template delimiters: `{{ expression }}`
//! - Variable access: `$node`, `$execution`, `$workflow`, `$input`
//!
//! ## Template Rendering
//!
//! Use `evaluate_template()` to process templates with multiple `{{ }}` expressions:
//! - Supports HTML, JSON, Markdown, and any text format
//! - All `{{ expression }}` patterns are replaced with their evaluated results
//! - Text outside expressions is preserved as-is
//! - `abs(n)` - Absolute value
//! - `ceil(n)` - Ceiling function
//! - `concat(arr1, arr2, ...)` - Concatenate arrays
//! - `contains(str, needle)` - Check if contains
//! - `ends_with(str, suffix)` - Check if ends with
//! - `first(arr)` - Get first element
//! - `flatten(arr)` - Flatten nested array
//! - `floor(n)` - Floor function
//! - `has(obj, key)` - Check if key exists
//! - `is_array(value)` - Check if array
//! - `is_null(value)` - Check if null
//! - `is_number(value)` - Check if number
//! - `is_object(value)` - Check if object
//! - `is_string(value)` - Check if string
//! - `join(arr, separator)` - Join array to string
//! - `keys(obj)` - Get object keys
//! - `last(arr)` - Get last element
//! - `length(arr)` - Get array length
//! - `length(str)` - Get string length
//! - `lowercase(str)` - Convert to lowercase
//! - `max(a, b, ...)` - Maximum value
//! - `min(a, b, ...)` - Minimum value
//! - `now()` - Get current timestamp
//! - `parse_json(str)` - Parse JSON string
//! - `pow(base, exp)` - Power function
//! - `replace(str, from, to)` - Replace substring
//! - `reverse(arr)` - Reverse array
//! - `round(n)` - Round to nearest integer
//! - `slice(arr, start, end)` - Slice array
//! - `sort(arr)` - Sort array
//! - `split(str, delimiter)` - Split string
//! - `sqrt(n)` - Square root
//! - `starts_with(str, prefix)` - Check if starts with
//! - `substring(str, start, end)` - Get substring
//! - `to_boolean(value)` - Convert to boolean
//! - `to_json(value)` - Convert to JSON string
//! - `to_number(value)` - Convert to number
//! - `to_string(value)` - Convert to string
//! - `trim(str)` - Trim whitespace
//! - `uppercase(str)` - Convert to uppercase
//! - `uuid()` - Generate UUID
//! - `values(obj)` - Get object values
//!
//! ### Date/Time Functions
//! - `now()` - Current timestamp (Unix seconds)
//! - `now_iso()` - Current time as ISO 8601 string
//! - `format_date(timestamp, format)` - Format timestamp
//! - `parse_date(str)` - Parse date string to timestamp
//! - `date_add(timestamp, amount, unit)` - Add duration
//! - `date_subtract(timestamp, amount, unit)` - Subtract duration
//! - `date_diff(ts1, ts2, unit)` - Difference between dates
//! - `date_year(timestamp)` - Extract year
//! - `date_month(timestamp)` - Extract month (1-12)
//! - `date_day(timestamp)` - Extract day (1-31)
//! - `date_hour(timestamp)` - Extract hour (0-23)
//! - `date_minute(timestamp)` - Extract minute (0-59)
//! - `date_second(timestamp)` - Extract second (0-59)
//! - `date_day_of_week(timestamp)` - Day of week (0=Sunday)
//! // Create a context
//! // Create an engine
//! // Create an engine with caching for better performance
//! // Evaluate an expression
//! Expression language for workflow automation, compatible with n8n syntax.
//! The expression language includes comprehensive built-in functions:
//! This crate provides a powerful expression language for evaluating dynamic values
//! ```
//! ```
//! ```rust
//! ```rust
//! assert_eq!(result.as_str(), Some("exec-123"));
//! context.set_execution_var("id", Value::text("exec-123"));
//! in workflow automation contexts. It supports:
//! let engine = ExpressionEngine::new();
//! let engine = ExpressionEngine::with_cache_size(1000);
//! let mut context = EvaluationContext::new();
//! let result = engine.evaluate("$execution.id", &context).unwrap();
//! use nebula_expression::ExpressionEngine;
//! use nebula_expression::{ExpressionEngine, EvaluationContext};
//! use nebula_value::Value;
pub mod maybe;
pub mod core;
pub mod lexer;
pub mod parser;
pub mod eval;
pub mod context;
pub mod builtins;
pub mod engine;

// Re-exports
pub use context::{EvaluationContext, EvaluationContextBuilder};
pub use engine::ExpressionEngine;
pub use core::error::{ExpressionErrorExt, ExpressionResult};
pub use core::ast::{Expr, BinaryOp};
pub use core::token::Token;
pub use maybe::MaybeExpression;

// Re-export nebula types for convenience
pub use nebula_value::Value;
pub use nebula_error::NebulaError;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::{
        EvaluationContext, EvaluationContextBuilder, ExpressionEngine, ExpressionErrorExt,
        ExpressionResult, MaybeExpression, Value, NebulaError,
    };
}

