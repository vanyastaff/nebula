//! Array manipulation functions

use super::{check_arg_count, check_min_arg_count, get_array_arg};
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Get the length of an array
pub fn length(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    let arr = get_array_arg("length", args, 0, "array")?;
    Ok(Value::integer(arr.len() as i64))
}

/// Get the first element of an array
pub fn first(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("first", args, 1)?;
    let arr = get_array_arg("first", args, 0, "array")?;
    let json_val = arr
        .get(0)
        .ok_or_else(|| NebulaError::expression_eval_error("Array is empty"))?;
    Ok(json_val.clone())
}

/// Get the last element of an array
pub fn last(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("last", args, 1)?;
    let arr = get_array_arg("last", args, 0, "array")?;
    let len = arr.len();
    if len == 0 {
        return Err(NebulaError::expression_eval_error("Array is empty"));
    }
    let json_val = arr
        .get(len - 1)
        .ok_or_else(|| NebulaError::expression_eval_error("Array is empty"))?;
    Ok(json_val.clone())
}

/// Filter array elements (stub - lambdas need special handling)
pub fn filter(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    // Note: This would require special handling in the evaluator to pass lambdas
    Err(NebulaError::expression_eval_error(
        "filter requires lambda support in evaluator",
    ))
}

/// Map over array elements (stub - lambdas need special handling)
pub fn map(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(NebulaError::expression_eval_error(
        "map requires lambda support in evaluator",
    ))
}

/// Reduce array elements (stub - lambdas need special handling)
pub fn reduce(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(NebulaError::expression_eval_error(
        "reduce requires lambda support in evaluator",
    ))
}

/// Sort an array
pub fn sort(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("sort", args, 1)?;
    let arr = get_array_arg("sort", args, 0, "array")?;

    let mut elements: Vec<Value> = arr.iter().cloned().collect();

    // Sort the values
    elements.sort_by(|a, b| match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => {
            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    });

    Ok(Value::Array(nebula_value::Array::from_vec(elements)))
}

/// Reverse an array
pub fn reverse(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("reverse", args, 1)?;
    let arr = get_array_arg("reverse", args, 0, "array")?;

    let mut elements: Vec<Value> = arr.iter().cloned().collect();
    elements.reverse();

    Ok(Value::Array(nebula_value::Array::from_vec(elements)))
}

/// Join array elements into a string
pub fn join(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("join", args, 2)?;
    let arr = get_array_arg("join", args, 0, "array")?;
    let separator = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    // Use iterator directly without intermediate Vec allocation
    let result = arr
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(separator);

    Ok(Value::text(result))
}

/// Slice an array
pub fn slice(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("slice", args, 2)?;
    let arr = get_array_arg("slice", args, 0, "array")?;
    let start = args[1].to_integer()? as usize;
    let end = if args.len() > 2 {
        args[2].to_integer()? as usize
    } else {
        arr.len()
    };

    let result: Vec<_> = (start..end.min(arr.len()))
        .filter_map(|i| arr.get(i).cloned())
        .collect();
    Ok(Value::Array(nebula_value::Array::from_vec(result)))
}

/// Concatenate arrays
pub fn concat(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("concat", args, 1)?;

    // Calculate total size to pre-allocate
    let total_size: usize = args
        .iter()
        .filter_map(|arg| arg.as_array().map(|arr| arr.len()))
        .sum();

    let mut result = Vec::with_capacity(total_size);
    for (i, arg) in args.iter().enumerate() {
        let arr = get_array_arg("concat", args, i, "array")?;
        result.extend(arr.iter().cloned());
    }

    Ok(Value::Array(nebula_value::Array::from_vec(result)))
}

/// Flatten a nested array
pub fn flatten(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("flatten", args, 1)?;
    let arr = get_array_arg("flatten", args, 0, "array")?;

    // Use iterator + flat_map for more efficient flattening
    let result: Vec<_> = arr
        .iter()
        .flat_map(|elem| {
            if let Some(inner_arr) = elem.as_array() {
                inner_arr.iter().cloned().collect::<Vec<_>>()
            } else {
                vec![elem.clone()]
            }
        })
        .collect();

    Ok(Value::Array(nebula_value::Array::from_vec(result)))
}
