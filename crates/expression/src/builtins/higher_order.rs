//! Higher-order array combinators.
//!
//! These are registered builtins like any other: they receive
//! [`Argument`]s, invoke lambdas through [`BuiltinView::invoke_lambda`], and
//! therefore share the caller's `EvalFrame`. The step budget and recursion
//! depth accumulate across every iteration; a lambda cannot be used to reset
//! either. This is the type-level replacement for the previous hardcoded
//! dispatch list in the evaluator (CO-C1-01, issue #252).

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    ExpressionError,
    builtins::{ArrayOutputBudget, GroupOutputBudget, check_arg_count},
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

/// Borrow the array in argument slot `0`.
fn array_argument<'a>(
    args: &'a [Argument<'_>],
    function: &'static str,
) -> ExpressionResult<&'a [RuntimeValue]> {
    let value = args
        .first()
        .and_then(Argument::as_value)
        .ok_or_else(|| missing_argument(function, "array"))?;
    match value.as_array() {
        Some(array) => Ok(array),
        None => Err(ExpressionError::type_error(
            "array",
            crate::value_utils::value_type_name(value),
        )),
    }
}

/// Borrow the lambda in argument slot `index`.
fn lambda_argument<'a>(
    args: &'a [Argument<'_>],
    index: usize,
    function: &'static str,
) -> ExpressionResult<&'a crate::ast::Expr> {
    args.get(index)
        .and_then(Argument::as_lambda)
        .ok_or_else(|| missing_argument(function, "lambda"))
}

fn missing_argument(function: &'static str, argument: &'static str) -> ExpressionError {
    ExpressionError::invalid_argument(function, format!("missing {argument} argument"))
}

/// `filter(array, x => condition)` — keep elements whose predicate is truthy.
pub(crate) fn filter(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("filter", args, 2)?;
    let array = array_argument(args, "filter")?;
    let lambda = lambda_argument(args, 1, "filter")?;

    let mut result = Vec::with_capacity(array.len());
    for item in array {
        let predicate = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        if crate::value_utils::to_boolean(&predicate) {
            result.push(item.clone());
        }
    }
    Ok(RuntimeValue::Array(result.into()))
}

/// `map(array, x => transform)` — transform every element.
pub(crate) fn map(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("map", args, 2)?;
    let array = array_argument(args, "map")?;
    let lambda = lambda_argument(args, 1, "map")?;

    let output = view.output_builder(context);
    output.ensure_collection_items(array.len())?;
    let mut budget = ArrayOutputBudget::new(output)?;
    let mut result = Vec::with_capacity(array.len());
    for item in array {
        let transformed = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        budget.push(&transformed)?;
        result.push(transformed);
    }
    Ok(RuntimeValue::Array(result.into()))
}

/// `reduce(array, initial, (acc, x) => expression)` — fold left.
///
/// Single-parameter lambdas receive the element and the accumulator stays
/// available as `$acc`; multi-parameter lambdas receive it positionally.
pub(crate) fn reduce(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("reduce", args, 3)?;
    let array = array_argument(args, "reduce")?;
    let lambda = lambda_argument(args, 2, "reduce")?;
    let mut accumulator = args
        .get(1)
        .and_then(Argument::as_value)
        .cloned()
        .ok_or_else(|| missing_argument("reduce", "initial"))?;

    for item in array {
        accumulator = match lambda {
            crate::ast::Expr::Lambda { params, body } if params.len() == 1 => {
                // Single-parameter lambda: the element is bound to the
                // parameter and `$acc` carries the accumulator. The lexer
                // strips the `$` sigil, so the bound name is `acc`.
                let mut lambda_context = context.clone();
                lambda_context.set_lambda_var(params[0].as_ref(), item.clone());
                lambda_context.set_lambda_var("acc", accumulator.clone());
                view.eval_body(body, &lambda_context)?
            },
            crate::ast::Expr::Lambda { .. } => {
                view.invoke_lambda(lambda, &[accumulator.clone(), item.clone()], context)?
            },
            _ => return Err(missing_argument("reduce", "lambda")),
        };
    }
    Ok(accumulator)
}

/// `find(array, x => condition)` — first match, or nothing.
pub(crate) fn find(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("find", args, 2)?;
    let array = array_argument(args, "find")?;
    let lambda = lambda_argument(args, 1, "find")?;

    for item in array {
        let predicate = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        if crate::value_utils::to_boolean(&predicate) {
            return Ok(item.clone());
        }
    }
    Ok(RuntimeValue::Null)
}

/// `find_index(array, x => condition)` — index of the first match, or `-1`.
pub(crate) fn find_index(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("find_index", args, 2)?;
    let array = array_argument(args, "find_index")?;
    let lambda = lambda_argument(args, 1, "find_index")?;

    for (index, item) in array.iter().enumerate() {
        let predicate = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        if crate::value_utils::to_boolean(&predicate) {
            return Ok(RuntimeValue::Integer(index as i64));
        }
    }
    Ok(RuntimeValue::Integer(-1))
}

/// `every(array, x => condition)` (alias `all`) — conjunction over elements.
pub(crate) fn every(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("every", args, 2)?;
    let array = array_argument(args, "every")?;
    let lambda = lambda_argument(args, 1, "every")?;

    for item in array {
        let predicate = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        if !crate::value_utils::to_boolean(&predicate) {
            return Ok(RuntimeValue::Bool(false));
        }
    }
    Ok(RuntimeValue::Bool(true))
}

/// `some(array, x => condition)` (alias `any`) — disjunction over elements.
pub(crate) fn some(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("some", args, 2)?;
    let array = array_argument(args, "some")?;
    let lambda = lambda_argument(args, 1, "some")?;

    for item in array {
        let predicate = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        if crate::value_utils::to_boolean(&predicate) {
            return Ok(RuntimeValue::Bool(true));
        }
    }
    Ok(RuntimeValue::Bool(false))
}

/// `group_by(array, x => key)` — bucket elements by a scalar key.
pub(crate) fn group_by(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("group_by", args, 2)?;
    let array = array_argument(args, "group_by")?;
    let lambda = lambda_argument(args, 1, "group_by")?;

    let output = view.output_builder(context);
    let mut budget = GroupOutputBudget::new(output)?;
    let mut groups: BTreeMap<Arc<str>, Vec<RuntimeValue>> = BTreeMap::new();
    for item in array {
        let key_value = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        let key = match &key_value {
            RuntimeValue::String(text) => text.clone(),
            RuntimeValue::Integer(value) => Arc::from(value.to_string().as_str()),
            RuntimeValue::Unsigned(value) => Arc::from(value.to_string().as_str()),
            RuntimeValue::Float(value) => Arc::from(value.to_string().as_str()),
            RuntimeValue::Bool(value) => Arc::from(if *value { "true" } else { "false" }),
            RuntimeValue::Null | RuntimeValue::Undefined => Arc::from("null"),
            _ => {
                return Err(ExpressionError::eval_error(
                    "group_by key must be a string, number, boolean, or null",
                ));
            },
        };
        let existing_items = groups.get(&key).map_or(0, Vec::len);
        budget.push(&key, existing_items, item)?;
        groups.entry(key).or_default().push(item.clone());
    }

    let object = groups
        .into_iter()
        .map(|(key, items)| (key, RuntimeValue::Array(items.into())))
        .collect();
    Ok(RuntimeValue::Object(Arc::new(object)))
}

/// `flat_map(array, x => transform)` — map, then flatten one level.
pub(crate) fn flat_map(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("flat_map", args, 2)?;
    let array = array_argument(args, "flat_map")?;
    let lambda = lambda_argument(args, 1, "flat_map")?;

    let output = view.output_builder(context);
    let mut budget = ArrayOutputBudget::new(output)?;
    let mut result = Vec::new();
    for item in array {
        let transformed = view.invoke_lambda(lambda, std::slice::from_ref(item), context)?;
        match transformed {
            RuntimeValue::Array(inner) => {
                for value in &*inner {
                    budget.push(value)?;
                    result.push(value.clone());
                }
            },
            other => {
                budget.push(&other)?;
                result.push(other);
            },
        }
    }
    Ok(RuntimeValue::Array(result.into()))
}
