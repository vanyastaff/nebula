//! Deferred rules — evaluated against the whole value in full mode only.
//! Skipped at schema-validation time.

use std::cmp::Ordering;

use serde::Serialize;
use serde_json::Value;

use crate::{
    foundation::{FieldPath, ValidationError, error::codes, validatable::json_type_name},
    rule::context::PredicateContext,
};

/// Rule that the static pass defers to full evaluation.
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeferredRule {
    /// Each array item must have a unique value at the given sub-path.
    ///
    /// Items without a value at the sub-path are not compared. The root
    /// path compares whole items.
    UniqueBy(FieldPath),
}

impl std::fmt::Debug for DeferredRule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UniqueBy(_) => "DeferredRule::UniqueBy(<protected>)",
        })
    }
}

impl DeferredRule {
    /// Evaluates the rule against the whole input.
    ///
    /// The static rule pass defers these rules before reaching this method.
    ///
    /// # Errors
    ///
    /// `type_mismatch` when the input is not an array, and `unique_by`
    /// (params `path` and `index`, the position of the later duplicate)
    /// when two items share a value at the sub-path.
    pub fn validate(
        &self,
        input: &Value,
        _ctx: Option<&PredicateContext>,
    ) -> Result<(), ValidationError> {
        match self {
            Self::UniqueBy(path) => unique_by(path, input),
        }
    }
}

fn unique_by(path: &FieldPath, input: &Value) -> Result<(), ValidationError> {
    let Value::Array(items) = input else {
        return Err(ValidationError::type_mismatch(
            "",
            "array",
            json_type_name(input),
        ));
    };
    let mut keys: Vec<(usize, &Value)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| lookup(item, path).map(|key| (index, key)))
        .collect();
    // Stable sort keeps equal keys in input order, so the second of an
    // adjacent equal pair is the later duplicate.
    keys.sort_by(|left, right| total_cmp(left.1, right.1));
    let duplicate = keys
        .windows(2)
        .filter(|pair| total_cmp(pair[0].1, pair[1].1) == Ordering::Equal)
        .map(|pair| pair[1].0)
        .min();
    match duplicate {
        None => Ok(()),
        Some(index) => Err(ValidationError::new(
            codes::UNIQUE_BY,
            "array items must be unique at the given path",
        )
        .with_param("path", path.as_str().to_owned())
        .with_param("index", index.to_string())),
    }
}

fn lookup<'a>(item: &'a Value, path: &FieldPath) -> Option<&'a Value> {
    path.segments()
        .try_fold(item, |value, segment| match value {
            Value::Object(map) => map.get(segment.as_ref()),
            Value::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index)),
            _ => None,
        })
}

/// A total order over JSON values that agrees with `Value`'s equality.
fn total_cmp(left: &Value, right: &Value) -> Ordering {
    fn rank(value: &Value) -> u8 {
        match value {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::Number(left), Value::Number(right)) => {
            let as_float = |number: &serde_json::Number| number.as_f64().unwrap_or(f64::NAN);
            as_float(left)
                .total_cmp(&as_float(right))
                .then_with(|| left.to_string().cmp(&right.to_string()))
        },
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Array(left), Value::Array(right)) => left
            .iter()
            .zip(right)
            .map(|(left, right)| total_cmp(left, right))
            .find(|ordering| ordering.is_ne())
            .unwrap_or_else(|| left.len().cmp(&right.len())),
        (Value::Object(left), Value::Object(right)) => {
            let (left_entries, right_entries) = (sorted_entries(left), sorted_entries(right));
            left_entries
                .iter()
                .zip(&right_entries)
                .map(|(left, right)| left.0.cmp(right.0).then_with(|| total_cmp(left.1, right.1)))
                .find(|ordering| ordering.is_ne())
                .unwrap_or_else(|| left_entries.len().cmp(&right_entries.len()))
        },
        _ => rank(left).cmp(&rank(right)),
    }
}

fn sorted_entries(map: &serde_json::Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    entries
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rule(path: &str) -> DeferredRule {
        DeferredRule::UniqueBy(FieldPath::parse(path).unwrap())
    }

    #[test]
    fn unique_by_roundtrip() {
        let r = rule("name");
        let back: DeferredRule = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn unique_by_accepts_distinct_keys() {
        let input = json!([{"id": 1}, {"id": 2}, {"name": "no id"}, {"name": "no id"}]);
        assert!(rule("id").validate(&input, None).is_ok());
    }

    #[test]
    fn unique_by_reports_the_later_duplicate() {
        let input = json!([{"id": "a"}, {"id": "b"}, {"id": "c"}, {"id": "b"}, {"id": "a"}]);
        let error = rule("id").validate(&input, None).unwrap_err();
        assert_eq!(error.code, codes::UNIQUE_BY);
        assert_eq!(error.param("index"), Some("3"));
        assert_eq!(error.param("path"), Some("/id"));
    }

    #[test]
    fn unique_by_compares_nested_and_structured_keys() {
        let nested = json!([{"a": {"b": 1}}, {"a": {"b": 1.0}}]);
        assert!(rule("a.b").validate(&nested, None).is_ok());
        let structured = json!([{"k": {"x": 1, "y": [2]}}, {"k": {"y": [2], "x": 1}}]);
        assert!(rule("k").validate(&structured, None).is_err());
    }

    #[test]
    fn unique_by_rejects_non_array_input() {
        let error = rule("id").validate(&json!({"id": 1}), None).unwrap_err();
        assert_eq!(error.code, codes::TYPE_MISMATCH);
    }
}
