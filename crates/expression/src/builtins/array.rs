//! Array manipulation functions

use super::{check_arg_count, check_min_arg_count, get_array_arg};
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::error::{ExpressionErrorExt, ExpressionResult};
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

/// Remove duplicate elements from an array, preserving order
///
/// Uses string representation for equality comparison.
/// Example: `unique([1,2,2,3,1])` returns `[1,2,3]`
pub fn unique(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("unique", args, 1)?;
    let arr = get_array_arg("unique", args, 0, "array")?;

    let mut seen = std::collections::HashSet::new();
    let result: Vec<Value> = arr
        .iter()
        .filter(|item| {
            // Use JSON serialization for stable equality comparison
            let key = item.to_string();
            seen.insert(key)
        })
        .cloned()
        .collect();

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

/// Returns true if any element equals value, or if any element has a truthy field
///
/// With two arguments: if the second argument is a string, it is treated as a field
/// name and each element is checked for a truthy value at that field. Otherwise,
/// each element is compared for equality with the second argument.
///
/// # Examples
/// - `some([1,2,3], 2)` → `true`
/// - `some([{"active": true}, {"active": false}], "active")` → `true`
/// - `some([1,3,5], 2)` → `false`
pub fn some(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("some", args, 2)?;
    let arr = get_array_arg("some", args, 0, "array")?;
    let criterion = &args[1];

    let result = if let Some(key) = criterion.as_str() {
        arr.iter()
            .any(|elem| elem.get(key).is_some_and(is_truthy))
    } else {
        arr.iter().any(|elem| elem == criterion)
    };

    Ok(Value::Bool(result))
}

/// Returns true if ALL elements equal value, or all have a truthy field
///
/// With two arguments: if the second argument is a string, it is treated as a field
/// name and every element must have a truthy value at that field. Otherwise,
/// every element must equal the second argument.
///
/// An empty array returns `true` (vacuous truth).
///
/// # Examples
/// - `every([1,1,1], 1)` → `true`
/// - `every([{"ok": true}, {"ok": true}], "ok")` → `true`
/// - `every([{"ok": true}, {"ok": false}], "ok")` → `false`
pub fn every(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("every", args, 2)?;
    let arr = get_array_arg("every", args, 0, "array")?;
    let criterion = &args[1];

    let result = if let Some(key) = criterion.as_str() {
        arr.iter()
            .all(|elem| elem.get(key).is_some_and(is_truthy))
    } else {
        arr.iter().all(|elem| elem == criterion)
    };

    Ok(Value::Bool(result))
}

/// Returns the first element where `element[key] == value`, or null if not found
///
/// # Examples
/// - `find([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}], "id", 2)` → `{"id": 2, "name": "b"}`
/// - `find([{"id": 1}], "id", 99)` → `null`
pub fn find(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("find", args, 3)?;
    let arr = get_array_arg("find", args, 0, "array")?;
    let key = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;
    let target = &args[2];

    let found = arr
        .iter()
        .find(|elem| elem.get(key).is_some_and(|v| v == target));

    Ok(found.cloned().unwrap_or(Value::Null))
}

/// Returns the index of the first element where `element[key] == value`, or -1 if not found
///
/// # Examples
/// - `find_index([{"id": 1}, {"id": 2}], "id", 2)` → `1`
/// - `find_index([{"id": 1}], "id", 99)` → `-1`
pub fn find_index(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("find_index", args, 3)?;
    let arr = get_array_arg("find_index", args, 0, "array")?;
    let key = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;
    let target = &args[2];

    let index = arr
        .iter()
        .position(|elem| elem.get(key).is_some_and(|v| v == target));

    let result = index.map_or(-1_i64, |i| i as i64);
    Ok(Value::Number(result.into()))
}

/// Groups array elements by the value of a field
///
/// Returns an object where each key is a distinct field value (as a string) and
/// the corresponding value is an array of elements that have that field value.
///
/// # Examples
/// - `group_by([{"type": "a", "v": 1}, {"type": "b", "v": 2}, {"type": "a", "v": 3}], "type")`
///   → `{"a": [{"type": "a", "v": 1}, {"type": "a", "v": 3}], "b": [{"type": "b", "v": 2}]}`
pub fn group_by(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("group_by", args, 2)?;
    let arr = get_array_arg("group_by", args, 0, "array")?;
    let key = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    let mut groups: serde_json::Map<String, Value> = serde_json::Map::new();

    for elem in arr {
        let group_key = match elem.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => continue,
        };

        groups
            .entry(group_key)
            .and_modify(|bucket| {
                if let Value::Array(vec) = bucket {
                    vec.push(elem.clone());
                }
            })
            .or_insert_with(|| Value::Array(vec![elem.clone()]));
    }

    Ok(Value::Object(groups))
}

/// Extracts an array field from each element and flattens the results
///
/// For each element in the array, retrieves the value at `key` (which should be
/// an array) and concatenates all such arrays into a single flat array. Elements
/// where `key` is missing or not an array are skipped.
///
/// # Examples
/// - `flat_map([{"tags": ["a","b"]}, {"tags": ["c"]}], "tags")` → `["a","b","c"]`
pub fn flat_map(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("flat_map", args, 2)?;
    let arr = get_array_arg("flat_map", args, 0, "array")?;
    let key = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    let result: Vec<Value> = arr
        .iter()
        .filter_map(|elem| elem.get(key).and_then(|v| v.as_array()).cloned())
        .flatten()
        .collect();

    Ok(Value::Array(result))
}

/// Returns whether a JSON value is truthy (non-empty, non-zero, non-null, non-false)
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::BuiltinRegistry;
    use crate::context::EvaluationContext;
    use crate::eval::Evaluator;
    use serde_json::json;
    use std::sync::Arc;

    fn eval() -> Evaluator {
        Evaluator::new(Arc::new(BuiltinRegistry::new()))
    }

    fn ctx() -> EvaluationContext {
        EvaluationContext::new()
    }

    // --- some ---

    #[test]
    fn some_returns_true_when_element_equals_value() {
        let args = vec![json!([1, 2, 3]), json!(2)];
        assert_eq!(some(&args, &eval(), &ctx()).unwrap(), json!(true));
    }

    #[test]
    fn some_returns_false_when_no_element_equals_value() {
        let args = vec![json!([1, 3, 5]), json!(2)];
        assert_eq!(some(&args, &eval(), &ctx()).unwrap(), json!(false));
    }

    #[test]
    fn some_returns_true_when_any_element_has_truthy_field() {
        let args = vec![
            json!([{"active": true}, {"active": false}]),
            json!("active"),
        ];
        assert_eq!(some(&args, &eval(), &ctx()).unwrap(), json!(true));
    }

    #[test]
    fn some_returns_false_when_no_element_has_truthy_field() {
        let args = vec![
            json!([{"active": false}, {"active": false}]),
            json!("active"),
        ];
        assert_eq!(some(&args, &eval(), &ctx()).unwrap(), json!(false));
    }

    #[test]
    fn some_returns_false_on_empty_array() {
        let args = vec![json!([]), json!(1)];
        assert_eq!(some(&args, &eval(), &ctx()).unwrap(), json!(false));
    }

    // --- every ---

    #[test]
    fn every_returns_true_when_all_elements_equal_value() {
        let args = vec![json!([1, 1, 1]), json!(1)];
        assert_eq!(every(&args, &eval(), &ctx()).unwrap(), json!(true));
    }

    #[test]
    fn every_returns_false_when_not_all_elements_equal_value() {
        let args = vec![json!([1, 1, 2]), json!(1)];
        assert_eq!(every(&args, &eval(), &ctx()).unwrap(), json!(false));
    }

    #[test]
    fn every_returns_true_when_all_elements_have_truthy_field() {
        let args = vec![json!([{"ok": true}, {"ok": true}]), json!("ok")];
        assert_eq!(every(&args, &eval(), &ctx()).unwrap(), json!(true));
    }

    #[test]
    fn every_returns_false_when_any_element_has_falsy_field() {
        let args = vec![json!([{"ok": true}, {"ok": false}]), json!("ok")];
        assert_eq!(every(&args, &eval(), &ctx()).unwrap(), json!(false));
    }

    #[test]
    fn every_returns_true_on_empty_array() {
        let args = vec![json!([]), json!(1)];
        assert_eq!(every(&args, &eval(), &ctx()).unwrap(), json!(true));
    }

    // --- find ---

    #[test]
    fn find_returns_first_matching_element() {
        let args = vec![
            json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]),
            json!("id"),
            json!(2),
        ];
        assert_eq!(
            find(&args, &eval(), &ctx()).unwrap(),
            json!({"id": 2, "name": "b"})
        );
    }

    #[test]
    fn find_returns_null_when_no_match() {
        let args = vec![json!([{"id": 1}]), json!("id"), json!(99)];
        assert_eq!(find(&args, &eval(), &ctx()).unwrap(), json!(null));
    }

    #[test]
    fn find_returns_null_on_empty_array() {
        let args = vec![json!([]), json!("id"), json!(1)];
        assert_eq!(find(&args, &eval(), &ctx()).unwrap(), json!(null));
    }

    #[test]
    fn find_rejects_non_string_key() {
        let args = vec![json!([{"id": 1}]), json!(42), json!(1)];
        assert!(find(&args, &eval(), &ctx()).is_err());
    }

    // --- find_index ---

    #[test]
    fn find_index_returns_correct_index() {
        let args = vec![json!([{"id": 1}, {"id": 2}]), json!("id"), json!(2)];
        assert_eq!(find_index(&args, &eval(), &ctx()).unwrap(), json!(1));
    }

    #[test]
    fn find_index_returns_negative_one_when_not_found() {
        let args = vec![json!([{"id": 1}]), json!("id"), json!(99)];
        assert_eq!(find_index(&args, &eval(), &ctx()).unwrap(), json!(-1));
    }

    #[test]
    fn find_index_returns_negative_one_on_empty_array() {
        let args = vec![json!([]), json!("id"), json!(1)];
        assert_eq!(find_index(&args, &eval(), &ctx()).unwrap(), json!(-1));
    }

    // --- group_by ---

    #[test]
    fn group_by_groups_elements_by_field() {
        let args = vec![
            json!([{"type": "a", "v": 1}, {"type": "b", "v": 2}, {"type": "a", "v": 3}]),
            json!("type"),
        ];
        let result = group_by(&args, &eval(), &ctx()).unwrap();
        assert_eq!(
            result["a"],
            json!([{"type": "a", "v": 1}, {"type": "a", "v": 3}])
        );
        assert_eq!(result["b"], json!([{"type": "b", "v": 2}]));
    }

    #[test]
    fn group_by_returns_empty_object_for_empty_array() {
        let args = vec![json!([]), json!("type")];
        assert_eq!(group_by(&args, &eval(), &ctx()).unwrap(), json!({}));
    }

    #[test]
    fn group_by_skips_elements_missing_the_key() {
        let args = vec![json!([{"type": "a"}, {"other": "x"}]), json!("type")];
        let result = group_by(&args, &eval(), &ctx()).unwrap();
        assert_eq!(result["a"], json!([{"type": "a"}]));
        assert!(result.get("other").is_none());
    }

    // --- flat_map ---

    #[test]
    fn flat_map_extracts_and_flattens_field_arrays() {
        let args = vec![
            json!([{"tags": ["a", "b"]}, {"tags": ["c"]}]),
            json!("tags"),
        ];
        assert_eq!(
            flat_map(&args, &eval(), &ctx()).unwrap(),
            json!(["a", "b", "c"])
        );
    }

    #[test]
    fn flat_map_returns_empty_array_for_empty_input() {
        let args = vec![json!([]), json!("tags")];
        assert_eq!(flat_map(&args, &eval(), &ctx()).unwrap(), json!([]));
    }

    #[test]
    fn flat_map_skips_elements_with_missing_or_non_array_field() {
        let args = vec![
            json!([{"tags": ["a"]}, {"tags": "not-an-array"}, {"other": [1]}]),
            json!("tags"),
        ];
        assert_eq!(flat_map(&args, &eval(), &ctx()).unwrap(), json!(["a"]));
    }
}
