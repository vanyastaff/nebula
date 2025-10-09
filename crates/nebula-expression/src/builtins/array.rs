//! Array manipulation functions

use super::{check_arg_count, check_min_arg_count};
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;
use nebula_value::{JsonValueExt, ValueRefExt};

/// Get the length of an array
pub fn length(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;
    Ok(Value::integer(arr.len() as i64))
}

/// Get the first element of an array
pub fn first(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("first", args, 1)?;
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;
    let json_val = arr
        .get(0)
        .ok_or_else(|| NebulaError::expression_eval_error("Array is empty"))?;
    Ok(json_val.to_nebula_value_or_null())
}

/// Get the last element of an array
pub fn last(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("last", args, 1)?;
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;
    let len = arr.len();
    if len == 0 {
        return Err(NebulaError::expression_eval_error("Array is empty"));
    }
    let json_val = arr
        .get(len - 1)
        .ok_or_else(|| NebulaError::expression_eval_error("Array is empty"))?;
    Ok(json_val.to_nebula_value_or_null())
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
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;

    let mut elements: Vec<Value> = arr
        .iter()
        .map(|json_val| json_val.to_nebula_value_or_null())
        .collect();
    elements.sort_by(|a, b| match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.value().cmp(&y.value()),
        (Value::Float(x), Value::Float(y)) => x
            .value()
            .partial_cmp(&y.value())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => x.as_str().cmp(y.as_str()),
        _ => std::cmp::Ordering::Equal,
    });

    let mut result = nebula_value::Array::new();
    for elem in elements {
        result = result.push(elem.to_json());
    }
    Ok(Value::Array(result))
}

/// Reverse an array
pub fn reverse(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("reverse", args, 1)?;
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;

    let mut elements: Vec<Value> = arr
        .iter()
        .map(|json_val| json_val.to_nebula_value_or_null())
        .collect();
    elements.reverse();

    let mut result = nebula_value::Array::new();
    for elem in elements {
        result = result.push(elem.to_json());
    }
    Ok(Value::Array(result))
}

/// Join array elements into a string
pub fn join(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("join", args, 2)?;
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;
    let separator = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    // Use iterator directly without intermediate Vec allocation
    let result = arr.iter()
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
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;
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
    let total_size: usize = args.iter()
        .filter_map(|arg| arg.as_array().map(|arr| arr.len()))
        .sum();

    let mut result = Vec::with_capacity(total_size);
    for arg in args {
        let arr = arg
            .as_array()
            .ok_or_else(|| NebulaError::expression_type_error("array", arg.kind().name()))?;
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
    let arr = args[0]
        .as_array()
        .ok_or_else(|| NebulaError::expression_type_error("array", args[0].kind().name()))?;

    // Use iterator + flat_map for more efficient flattening
    let result: Vec<_> = arr.iter()
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
