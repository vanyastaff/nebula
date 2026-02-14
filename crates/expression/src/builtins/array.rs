//! Array manipulation functions

use super::{check_arg_count, check_min_arg_count, get_array_arg};
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use serde_json::Value;

/// Get the length of an array
pub fn length(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    let arr = get_array_arg("length", args, 0, "array")?;
    Ok(Value::Number((arr.len() as i64).into()))
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
        .first()
        .ok_or_else(|| ExpressionError::expression_eval_error("Array is empty"))?;
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
        return Err(ExpressionError::expression_eval_error("Array is empty"));
    }
    let json_val = arr
        .get(len - 1)
        .ok_or_else(|| ExpressionError::expression_eval_error("Array is empty"))?;
    Ok(json_val.clone())
}

/// Filter array elements (stub - lambdas need special handling)
pub fn filter(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    // Note: This would require special handling in the evaluator to pass lambdas
    Err(ExpressionError::expression_eval_error(
        "filter requires lambda support in evaluator",
    ))
}

/// Map over array elements (stub - lambdas need special handling)
pub fn map(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(ExpressionError::expression_eval_error(
        "map requires lambda support in evaluator",
    ))
}

/// Reduce array elements (stub - lambdas need special handling)
pub fn reduce(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(ExpressionError::expression_eval_error(
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

    let mut elements: Vec<Value> = arr.to_vec();

    // Sort the values
    elements.sort_by(|a, b| match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let x_val = crate::value_utils::number_as_f64(x).unwrap_or(0.0);
            let y_val = crate::value_utils::number_as_f64(y).unwrap_or(0.0);
            x_val
                .partial_cmp(&y_val)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    });

    Ok(Value::Array(elements))
}

/// Reverse an array
pub fn reverse(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("reverse", args, 1)?;
    let arr = get_array_arg("reverse", args, 0, "array")?;

    let mut elements: Vec<Value> = arr.to_vec();
    elements.reverse();

    Ok(Value::Array(elements))
}

/// Join array elements into a string
pub fn join(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("join", args, 2)?;
    let arr = get_array_arg("join", args, 0, "array")?;
    let separator = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    // Convert array elements to strings and join
    let result = arr
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            _ => v.to_string(),
        })
        .collect::<Vec<_>>()
        .join(separator);

    Ok(Value::String(result))
}

/// Slice an array
pub fn slice(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("slice", args, 2)?;
    let arr = get_array_arg("slice", args, 0, "array")?;
    let start = args[1].as_i64().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "integer",
            crate::value_utils::value_type_name(&args[1]),
        )
    })? as usize;
    let end = if args.len() > 2 {
        args[2].as_i64().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "integer",
                crate::value_utils::value_type_name(&args[2]),
            )
        })? as usize
    } else {
        arr.len()
    };

    let result: Vec<_> = (start..end.min(arr.len()))
        .filter_map(|i| arr.get(i).cloned())
        .collect();
    Ok(Value::Array(result))
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
    for (i, _arg) in args.iter().enumerate() {
        let arr = get_array_arg("concat", args, i, "array")?;
        result.extend(arr.iter().cloned());
    }

    Ok(Value::Array(result))
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
                inner_arr.to_vec()
            } else {
                vec![elem.clone()]
            }
        })
        .collect();

    Ok(Value::Array(result))
}
