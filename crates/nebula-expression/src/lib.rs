#![warn(clippy::all)]
#![warn(missing_docs)]

//! # nebula-expression
//!
//! Expression language for workflow automation, compatible with n8n syntax.
//!
//! This crate provides a powerful expression language for evaluating dynamic values
//! in workflow automation contexts. It supports:
//!
//! - Variable access: `$node`, `$execution`, `$workflow`, `$input`
//! - Property access: `$node.data`, `$execution.id`
//! - Arithmetic operators: `+`, `-`, `*`, `/`, `%`, `**`
//! - Comparison operators: `==`, `!=`, `>`, `<`, `>=`, `<=`, `=~`
//! - Logical operators: `&&`, `||`, `!`
//! - Conditionals: `if condition then value1 else value2`
//! - Function calls: `functionName(arg1, arg2)`
//! - Index access: `array[0]`, `object['key']`
//! - Pipeline operator: `|` for function chaining
//! - Lambda expressions: `x => x > 5` (in filter/map/reduce)
//!
//! ## Quick Start
//!
//! ```
//! use nebula_expression::{ExpressionEngine, EvaluationContext};
//! use nebula_value::Value;
//!
//! // Create an engine
//! let engine = ExpressionEngine::new();
//! let mut context = EvaluationContext::new();
//!
//! // Evaluate an expression
//! context.set_execution_var("id", Value::text("exec-123"));
//! let result = engine.evaluate("$execution.id", &context).unwrap();
//! assert_eq!(result.as_str(), Some("exec-123"));
//! ```
//!
//! ## With Caching
//!
//! ```
//! use nebula_expression::ExpressionEngine;
//!
//! // Create an engine with caching for better performance
//! let engine = ExpressionEngine::with_cache_size(1000);
//! ```
//!
//! ## Template Rendering
//!
//! Use `evaluate_template()` to process templates with multiple `{{ }}` expressions:
//!
//! - Template delimiters: `{{ expression }}`
//! - Text outside expressions is preserved as-is
//! - Supports HTML, JSON, Markdown, and any text format
//! - All `{{ expression }}` patterns are replaced with their evaluated results
//!
//! ## Built-in Functions
//!
//! The expression language includes comprehensive built-in functions:
//!
//! ### String Functions
//! - `uppercase(str)` - Convert to uppercase
//! - `lowercase(str)` - Convert to lowercase
//! - `trim(str)` - Trim whitespace
//! - `split(str, delimiter)` - Split string
//! - `replace(str, from, to)` - Replace substring
//! - `substring(str, start, end)` - Get substring
//! - `contains(str, needle)` - Check if contains
//! - `starts_with(str, prefix)` - Check if starts with
//! - `ends_with(str, suffix)` - Check if ends with
//! - `length(str)` - Get string length
//!
//! ### Math Functions
//! - `abs(n)` - Absolute value
//! - `round(n)` - Round to nearest integer
//! - `floor(n)` - Floor function
//! - `ceil(n)` - Ceiling function
//! - `min(a, b, ...)` - Minimum value
//! - `max(a, b, ...)` - Maximum value
//! - `pow(base, exp)` - Power function
//! - `sqrt(n)` - Square root
//!
//! ### Array Functions
//! - `length(arr)` - Get array length
//! - `first(arr)` - Get first element
//! - `last(arr)` - Get last element
//! - `join(arr, separator)` - Join array to string
//! - `slice(arr, start, end)` - Slice array
//! - `reverse(arr)` - Reverse array
//! - `sort(arr)` - Sort array
//! - `concat(arr1, arr2, ...)` - Concatenate arrays
//! - `flatten(arr)` - Flatten nested array
//!
//! ### Object Functions
//! - `keys(obj)` - Get object keys
//! - `values(obj)` - Get object values
//! - `has(obj, key)` - Check if key exists
//!
//! ### Date/Time Functions
//! - `now()` - Get current timestamp
//! - `now_iso()` - Current time as ISO 8601 string
//! - `parse_date(str)` - Parse date string to timestamp
//! - `format_date(timestamp, format)` - Format timestamp
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
//!
//! ### Conversion Functions
//! - `to_string(value)` - Convert to string
//! - `to_number(value)` - Convert to number
//! - `to_boolean(value)` - Convert to boolean
//! - `to_json(value)` - Convert to JSON string
//! - `parse_json(str)` - Parse JSON string
//!
//! ### Utility Functions
//! - `is_null(value)` - Check if null
//! - `is_array(value)` - Check if array
//! - `is_object(value)` - Check if object
//! - `is_string(value)` - Check if string
//! - `is_number(value)` - Check if number
//! - `uuid()` - Generate UUID

// Core modules
pub mod builtins;
pub mod context;
pub mod core;
pub mod engine;
pub mod error_formatter;
pub mod eval;
pub mod lexer;
pub mod maybe;
pub mod parser;
pub mod template;

// Re-exports
pub use context::{EvaluationContext, EvaluationContextBuilder};
pub use core::ast::{BinaryOp, Expr};
pub use core::error::{ExpressionErrorExt, ExpressionResult};
pub use core::token::Token;
pub use engine::ExpressionEngine;
pub use maybe::MaybeExpression;
pub use template::{MaybeTemplate, Position, Template, TemplatePart};

// Re-export nebula types for convenience
pub use nebula_error::NebulaError;
pub use nebula_value::Value;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::{
        EvaluationContext, EvaluationContextBuilder, ExpressionEngine, ExpressionErrorExt,
        ExpressionResult, MaybeExpression, MaybeTemplate, NebulaError, Position, Template,
        TemplatePart, Value,
    };
}
