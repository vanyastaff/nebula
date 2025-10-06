//!
//! Built-in functions for the expression language
//! This module provides all built-in functions organized by category.
pub mod datetime;
pub mod array;
pub mod conversion;
pub mod math;
pub mod object;
pub mod string;
pub mod util;

use crate::context::EvaluationContext;
use crate::core::ast::Expr;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;
use std::collections::HashMap;

/// Type alias for a builtin function
pub type BuiltinFunction = fn(&[Value], &Evaluator, &EvaluationContext) -> ExpressionResult<Value>;

/// Registry of all builtin functions
pub struct BuiltinRegistry {
    functions: HashMap<String, BuiltinFunction>,
}

impl BuiltinRegistry {
    /// Create a new builtin registry with all standard functions
    pub fn new() -> Self {
        let mut registry = Self {
            functions: HashMap::new(),
        };

        // Register all builtin functions
        registry.register_string_functions();
        registry.register_math_functions();
        registry.register_array_functions();
        registry.register_object_functions();
        registry.register_conversion_functions();
        registry.register_util_functions();
        registry.register_datetime_functions();

        registry
    }

    /// Register a builtin function
    pub fn register(&mut self, name: impl Into<String>, func: BuiltinFunction) {
        self.functions.insert(name.into(), func);
    }

    /// Call a builtin function by name
    pub fn call(
        &self,
        name: &str,
        args: &[Value],
        evaluator: &Evaluator,
        context: &EvaluationContext,
    ) -> ExpressionResult<Value> {
        let func = self
            .functions
            .get(name)
            .ok_or_else(|| NebulaError::expression_function_not_found(name))?;

        func(args, evaluator, context)
    }

    /// Check if a function exists
    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    /// Get all function names
    pub fn function_names(&self) -> Vec<String> {
        self.functions.keys().cloned().collect()
    }

    // Registration methods for each category

    fn register_string_functions(&mut self) {
        self.register("uppercase", string::uppercase);
        self.register("lowercase", string::lowercase);
        self.register("trim", string::trim);
        self.register("split", string::split);
        self.register("replace", string::replace);
        self.register("substring", string::substring);
        self.register("contains", string::contains);
        self.register("starts_with", string::starts_with);
        self.register("ends_with", string::ends_with);
    }

    fn register_math_functions(&mut self) {
        self.register("abs", math::abs);
        self.register("round", math::round);
        self.register("floor", math::floor);
        self.register("ceil", math::ceil);
        self.register("min", math::min);
        self.register("max", math::max);
        self.register("sqrt", math::sqrt);
        self.register("pow", math::pow);
    }

    fn register_array_functions(&mut self) {
        self.register("first", array::first);
        self.register("last", array::last);
        self.register("filter", array::filter);
        self.register("map", array::map);
        self.register("reduce", array::reduce);
        self.register("sort", array::sort);
        self.register("reverse", array::reverse);
        self.register("join", array::join);
        self.register("slice", array::slice);
        self.register("concat", array::concat);
        self.register("flatten", array::flatten);
    }

    fn register_object_functions(&mut self) {
        self.register("keys", object::keys);
        self.register("values", object::values);
        self.register("has", object::has);
    }

    fn register_conversion_functions(&mut self) {
        self.register("to_string", conversion::to_string);
        self.register("to_number", conversion::to_number);
        self.register("to_boolean", conversion::to_boolean);
        self.register("to_json", conversion::to_json);
        self.register("parse_json", conversion::parse_json);
    }

    fn register_util_functions(&mut self) {
        self.register("length", util::length); // Universal length for strings and arrays
        self.register("is_null", util::is_null);
        self.register("is_array", util::is_array);
        self.register("is_object", util::is_object);
        self.register("is_string", util::is_string);
        self.register("is_number", util::is_number);
        self.register("uuid", util::uuid);
    }

    fn register_datetime_functions(&mut self) {
        // Current time
        self.register("now", datetime::now);
        self.register("now_iso", datetime::now_iso);

        // Formatting and parsing
        self.register("format_date", datetime::format_date);
        self.register("parse_date", datetime::parse_date);

        // Date arithmetic
        self.register("date_add", datetime::date_add);
        self.register("date_subtract", datetime::date_subtract);
        self.register("date_diff", datetime::date_diff);

        // Date extraction
        self.register("date_year", datetime::date_year);
        self.register("date_month", datetime::date_month);
        self.register("date_day", datetime::date_day);
        self.register("date_hour", datetime::date_hour);
        self.register("date_minute", datetime::date_minute);
        self.register("date_second", datetime::date_second);
        self.register("date_day_of_week", datetime::date_day_of_week);
    }
}

impl Default for BuiltinRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to check argument count
pub(crate) fn check_arg_count(
    func_name: &str,
    args: &[Value],
    expected: usize,
) -> ExpressionResult<()> {
    if args.len() != expected {
        Err(NebulaError::expression_invalid_argument(
            func_name,
            format!("Expected {} arguments, got {}", expected, args.len()),
        ))
    } else {
        Ok(())
    }
}

/// Helper to check minimum argument count
pub(crate) fn check_min_arg_count(
    func_name: &str,
    args: &[Value],
    min: usize,
) -> ExpressionResult<()> {
    if args.len() < min {
        Err(NebulaError::expression_invalid_argument(
            func_name,
            format!("Expected at least {} arguments, got {}", min, args.len()),
        ))
    } else {
        Ok(())
    }
}

/// Helper to extract a lambda expression from args
pub(crate) fn extract_lambda(arg: &Expr) -> ExpressionResult<(&str, &Expr)> {
    match arg {
        Expr::Lambda { param, body } => Ok((param, body)),
        _ => Err(NebulaError::expression_invalid_argument(
            "lambda",
            "Expected a lambda expression",
        )),
    }
}

