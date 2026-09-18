//! Built-in functions for the expression language
//! This module provides all built-in functions organized by category.
pub(crate) mod array;
pub(crate) mod conversion;
#[cfg(feature = "datetime")]
pub(crate) mod datetime;
pub(crate) mod higher_order;
pub(crate) mod math;
pub(crate) mod methods;
pub(crate) mod object;
mod output;
pub(crate) mod string;
pub(crate) mod util;

use std::collections::HashMap;

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

pub(crate) use output::{ArrayOutputBudget, GroupOutputBudget};
pub use output::{BuiltinOutput, BuiltinOutputBuilder, BuiltinOutputLimit};

/// Type alias for a builtin function.
///
/// The middle parameter is `BuiltinView<'_>`, NOT `&Evaluator`. The view
/// exposes policy queries, shared work charging (`charge_work`,
/// `check_output_bytes`), and bounded lambda invocation (`invoke_lambda`,
/// `eval_body`) over the caller's frame. Registered builtins therefore cannot
/// reset the step budget or recursion depth. This is a type-enforced
/// replacement for the discipline rule documented in the crate `lib.rs`
/// "Known limitation" note (CO-C1-01 step-budget bypass, issue #252).
pub type BuiltinFunction = fn(
    &[Argument<'_>],
    BuiltinView<'_>,
    &EvaluationContext,
    BuiltinOutputBuilder,
) -> ExpressionResult<BuiltinOutput>;

type TrustedBuiltinFunction =
    fn(&[Argument<'_>], BuiltinView<'_>, &EvaluationContext) -> ExpressionResult<RuntimeValue>;

#[derive(Clone, Copy)]
enum RegisteredBuiltin {
    Bounded(BuiltinFunction),
    Trusted(TrustedBuiltinFunction),
}

/// Registry of all builtin functions
#[derive(Clone)]
pub struct BuiltinRegistry {
    functions: HashMap<String, RegisteredBuiltin>,
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
        registry.register_higher_order_functions();
        #[cfg(feature = "datetime")]
        registry.register_datetime_functions();

        registry
    }

    fn register(&mut self, name: impl AsRef<str>, function: TrustedBuiltinFunction) {
        self.functions.insert(
            name.as_ref().to_owned(),
            RegisteredBuiltin::Trusted(function),
        );
    }

    pub(crate) fn register_bounded(&mut self, name: impl AsRef<str>, function: BuiltinFunction) {
        self.functions.insert(
            name.as_ref().to_owned(),
            RegisteredBuiltin::Bounded(function),
        );
    }

    /// Call a builtin function by name.
    ///
    /// Requires the calling evaluator's policy and budget view. The registered
    /// function cannot construct a fresh budget and lambda arguments are
    /// invoked through the view's shared frame.
    pub fn call(
        &self,
        name: &str,
        args: &[Argument<'_>],
        view: BuiltinView<'_>,
        context: &EvaluationContext,
    ) -> ExpressionResult<RuntimeValue> {
        let function = self
            .functions
            .get(name)
            .ok_or_else(|| ExpressionError::function_not_found(name))?;
        let output = view.output_builder();

        match function {
            RegisteredBuiltin::Bounded(function) => {
                output.accept(function(args, view, context, output)?)
            },
            RegisteredBuiltin::Trusted(function) => output.value(function(args, view, context)?),
        }
        .map(BuiltinOutput::into_value)
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
        self.register("pad_start", string::pad_start);
        self.register("pad_end", string::pad_end);
        self.register("repeat", string::repeat);
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
        self.register("sort", array::sort);
        self.register("reverse", array::reverse);
        self.register("join", array::join);
        self.register("slice", array::slice);
        self.register("concat", array::concat);
        self.register("flatten", array::flatten);
        self.register("unique", array::unique);
        self.register("index_of", array::index_of);
    }

    /// Higher-order combinators are ordinary builtins: they receive
    /// [`Argument`]s and invoke lambdas through the shared frame.
    fn register_higher_order_functions(&mut self) {
        self.register("filter", higher_order::filter);
        self.register("map", higher_order::map);
        self.register("reduce", higher_order::reduce);
        self.register("find", higher_order::find);
        self.register("find_index", higher_order::find_index);
        self.register("every", higher_order::every);
        self.register("all", higher_order::every);
        self.register("some", higher_order::some);
        self.register("any", higher_order::some);
        self.register("group_by", higher_order::group_by);
        self.register("flat_map", higher_order::flat_map);
    }

    fn register_object_functions(&mut self) {
        self.register("keys", object::keys);
        self.register("values", object::values);
        self.register("has", object::has);
        self.register("merge", object::merge);
        self.register("pick", object::pick);
        self.register("omit", object::omit);
        self.register("entries", object::entries);
        self.register("from_entries", object::from_entries);
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
        self.register("coalesce", util::coalesce);
        self.register("type_of", util::type_of);
    }

    #[cfg(feature = "datetime")]
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
    args: &[Argument<'_>],
    expected: usize,
) -> ExpressionResult<()> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(ExpressionError::invalid_argument(
            func_name,
            format!("Expected {} arguments, got {}", expected, args.len()),
        ))
    }
}

/// Helper to check minimum argument count
pub(crate) fn check_min_arg_count(
    func_name: &str,
    args: &[Argument<'_>],
    min: usize,
) -> ExpressionResult<()> {
    if args.len() < min {
        Err(ExpressionError::invalid_argument(
            func_name,
            format!("Expected at least {} arguments, got {}", min, args.len()),
        ))
    } else {
        Ok(())
    }
}

/// Borrow the value in argument slot `index`, reporting a missing or lambda
/// argument as an invalid-argument error.
pub(crate) fn get_value_arg<'a>(
    func_name: &str,
    args: &'a [Argument<'_>],
    index: usize,
    arg_name: &str,
) -> ExpressionResult<&'a RuntimeValue> {
    let argument = args.get(index).ok_or_else(|| {
        ExpressionError::invalid_argument(
            func_name,
            format!("Missing argument '{arg_name}' at position {index}"),
        )
    })?;
    argument.as_value().ok_or_else(|| {
        ExpressionError::invalid_argument(
            func_name,
            format!("Argument '{arg_name}' must be a value, not a lambda"),
        )
    })
}

/// Preflight a string-producing builtin's exact output size.
///
/// Shared by every string/date builtin: charges the work, then checks the
/// string and total byte bounds before the allocation happens.
pub(crate) fn preflight_string_output(
    view: BuiltinView<'_>,
    output_bytes: usize,
) -> ExpressionResult<()> {
    view.check_output_bytes(output_bytes)?;
    let output = view.output_builder();
    output.ensure_string_bytes(output_bytes)?;
    output.ensure_total_bytes(output_bytes)
}

/// Helper to get a string argument with better error message
pub(crate) fn get_string_arg<'a>(
    func_name: &str,
    args: &'a [Argument<'_>],
    index: usize,
    arg_name: &str,
) -> ExpressionResult<&'a str> {
    let value = get_value_arg(func_name, args, index, arg_name)?;
    value.as_str().ok_or_else(|| {
        ExpressionError::invalid_argument(
            func_name,
            format!(
                "Argument '{}' must be a string, got {}",
                arg_name,
                crate::value_utils::value_type_name(value)
            ),
        )
    })
}

/// Helper to get an integer argument with better error message
pub(crate) fn get_int_arg(
    func_name: &str,
    args: &[Argument<'_>],
    index: usize,
    arg_name: &str,
) -> ExpressionResult<i64> {
    let value = get_value_arg(func_name, args, index, arg_name)?;
    crate::value_utils::to_integer(value).map_err(|_| {
        ExpressionError::invalid_argument(
            func_name,
            format!(
                "Argument '{}' must be an integer, got {}",
                arg_name,
                crate::value_utils::value_type_name(value)
            ),
        )
    })
}

/// Helper to get an integer argument with strict-mode awareness.
pub(crate) fn get_int_arg_with_policy(
    func_name: &str,
    args: &[Argument<'_>],
    index: usize,
    arg_name: &str,
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<i64> {
    let value = get_value_arg(func_name, args, index, arg_name)?;

    if view.is_strict_mode() {
        return match value {
            RuntimeValue::Integer(integer) => Ok(*integer),
            RuntimeValue::Unsigned(integer) => i64::try_from(*integer).map_err(|_| {
                ExpressionError::invalid_argument(
                    func_name,
                    format!(
                        "Argument '{}' must be an integer number in strict mode, got {}",
                        arg_name,
                        crate::value_utils::value_type_name(value)
                    ),
                )
            }),
            _ => Err(ExpressionError::invalid_argument(
                func_name,
                format!(
                    "Argument '{}' must be an integer number in strict mode, got {}",
                    arg_name,
                    crate::value_utils::value_type_name(value)
                ),
            )),
        };
    }

    get_int_arg(func_name, args, index, arg_name)
}

/// Helper to get a number argument (int or float) with better error message
pub(crate) fn get_number_arg(
    func_name: &str,
    args: &[Argument<'_>],
    index: usize,
    arg_name: &str,
) -> ExpressionResult<f64> {
    let value = get_value_arg(func_name, args, index, arg_name)?;
    crate::value_utils::to_float(value).map_err(|_| {
        ExpressionError::invalid_argument(
            func_name,
            format!(
                "Argument '{}' must be a number, got {}",
                arg_name,
                crate::value_utils::value_type_name(value)
            ),
        )
    })
}

/// Helper to get a number argument with strict-mode awareness.
pub(crate) fn get_number_arg_with_policy(
    func_name: &str,
    args: &[Argument<'_>],
    index: usize,
    arg_name: &str,
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<f64> {
    let value = get_value_arg(func_name, args, index, arg_name)?;

    if view.is_strict_mode() {
        return match value {
            RuntimeValue::Integer(integer) => Ok(*integer as f64),
            RuntimeValue::Unsigned(integer) => Ok(*integer as f64),
            RuntimeValue::Float(float) => Ok(*float),
            _ => Err(ExpressionError::invalid_argument(
                func_name,
                format!(
                    "Argument '{}' must be a number in strict mode, got {}",
                    arg_name,
                    crate::value_utils::value_type_name(value)
                ),
            )),
        };
    }

    get_number_arg(func_name, args, index, arg_name)
}

/// Helper to get an array argument with better error message
pub(crate) fn get_array_arg<'a>(
    func_name: &str,
    args: &'a [Argument<'_>],
    index: usize,
    arg_name: &str,
) -> ExpressionResult<&'a [RuntimeValue]> {
    let value = get_value_arg(func_name, args, index, arg_name)?;
    value.as_array().ok_or_else(|| {
        ExpressionError::invalid_argument(
            func_name,
            format!(
                "Argument '{}' must be an array, got {}",
                arg_name,
                crate::value_utils::value_type_name(value)
            ),
        )
    })
}

/// Helper to get an object argument with better error message
pub(crate) fn get_object_arg<'a>(
    func_name: &str,
    args: &'a [Argument<'_>],
    index: usize,
    arg_name: &str,
) -> ExpressionResult<&'a std::collections::BTreeMap<std::sync::Arc<str>, RuntimeValue>> {
    let value = get_value_arg(func_name, args, index, arg_name)?;
    value.as_object().ok_or_else(|| {
        ExpressionError::invalid_argument(
            func_name,
            format!(
                "Argument '{}' must be an object, got {}",
                arg_name,
                crate::value_utils::value_type_name(value)
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[RuntimeValue]) -> Vec<Argument<'_>> {
        values
            .iter()
            .map(|value| Argument::Value(std::borrow::Cow::Borrowed(value)))
            .collect()
    }

    #[test]
    fn test_get_string_arg_type_error() {
        let values = [RuntimeValue::Integer(42)];
        let args = arguments(&values);
        let result = get_string_arg("test_func", &args, 0, "text");

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Argument 'text' must be a string"));
        assert!(msg.contains("number"));
    }

    #[test]
    fn test_get_int_arg_type_error() {
        let values = [RuntimeValue::string("hello")];
        let args = arguments(&values);
        let result = get_int_arg("test_func", &args, 0, "count");

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Argument 'count' must be an integer"));
    }

    #[expect(
        clippy::approx_constant,
        reason = "3.14 is a representative float literal, not an approximation of π"
    )]
    #[test]
    fn test_get_number_arg_accepts_int_and_float() {
        let values = [RuntimeValue::Integer(42)];
        let args = arguments(&values);
        assert_eq!(
            get_number_arg("test_func", &args, 0, "value").unwrap(),
            42.0
        );

        let values = [RuntimeValue::Float(3.14)];
        let args = arguments(&values);
        assert_eq!(
            get_number_arg("test_func", &args, 0, "value").unwrap(),
            3.14
        );
    }

    #[test]
    fn test_get_array_arg_type_error() {
        let values = [RuntimeValue::string("not an array")];
        let args = arguments(&values);
        let result = get_array_arg("test_func", &args, 0, "items");

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Argument 'items' must be an array"));
    }

    #[test]
    fn lambda_argument_is_a_typed_error_not_a_silent_value() {
        let lambda = crate::ast::Expr::Lambda {
            params: Box::from([std::sync::Arc::from("x")]),
            body: Box::new(crate::ast::Expr::Identifier(std::sync::Arc::from("x"))),
        };
        let args = [Argument::Lambda(&lambda)];
        let result = get_string_arg("test_func", &args, 0, "text");
        assert!(
            result.is_err(),
            "lambda argument must not coerce to a value"
        );
    }
}
