//! Hard allocation bounds and the default per-call work allowance.

use crate::{ExpressionError, ExpressionResult, value::RuntimeValue};

pub(crate) const MAX_SOURCE_BYTES: usize = 1_048_576;
pub(crate) const MAX_TOKENS: usize = 65_536;
pub(crate) const MAX_AST_NODES: usize = 16_384;
pub(crate) const MAX_AST_DEPTH: usize = 256;
pub(crate) const MAX_RESULT_BYTES: usize = 1_048_576;
pub(crate) const MAX_RESULT_NODES: usize = 65_536;
pub(crate) const DEFAULT_MAX_EVAL_STEPS: usize = 100_000;

/// Upper bound of an RFC 3339 rendering with nanoseconds and an offset:
/// `2024-03-01T10:00:00.123456789+03:00` is 35 bytes.
const DATE_TIME_JSON_BYTES: usize = 35;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ValueSize {
    pub(crate) content_bytes: usize,
    pub(crate) nodes: usize,
    pub(crate) depth: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AggregateValueBudget {
    size: ValueSize,
    entries: usize,
}

impl AggregateValueBudget {
    pub(crate) fn new() -> ExpressionResult<Self> {
        let size = ValueSize {
            content_bytes: 2,
            nodes: 1,
            depth: 1,
        };
        check_value_size(size)?;
        Ok(Self { size, entries: 0 })
    }

    pub(crate) fn push_array_value(&mut self, value: &RuntimeValue) -> ExpressionResult<()> {
        let child = measure_value(value)?;
        let entries = self.entries.saturating_add(1);
        let size = ValueSize {
            content_bytes: self
                .size
                .content_bytes
                .saturating_add(usize::from(entries > 1))
                .saturating_add(child.content_bytes),
            nodes: self.size.nodes.saturating_add(child.nodes),
            depth: self.size.depth.max(child.depth.saturating_add(1)),
        };
        check_value_size(size)?;
        self.size = size;
        self.entries = entries;
        Ok(())
    }

    pub(crate) fn insert_object_value(
        &mut self,
        key: &str,
        value: &RuntimeValue,
        replaced: Option<&RuntimeValue>,
    ) -> ExpressionResult<()> {
        let child = measure_value(value)?;
        let replaced = replaced.map(measure_value).transpose()?;
        let is_new = replaced.is_none();
        let entries = self.entries.saturating_add(usize::from(is_new));
        let replaced_bytes = replaced.map_or(0, |size| size.content_bytes);
        let replaced_nodes = replaced.map_or(0, |size| size.nodes);
        let entry_bytes = if is_new {
            usize::from(entries > 1)
                .saturating_add(key.len())
                .saturating_add(3)
        } else {
            0
        };
        let size = ValueSize {
            content_bytes: self
                .size
                .content_bytes
                .saturating_sub(replaced_bytes)
                .saturating_add(entry_bytes)
                .saturating_add(child.content_bytes),
            nodes: self
                .size
                .nodes
                .saturating_sub(replaced_nodes)
                .saturating_add(child.nodes),
            // A stale maximum after replacement is safe: the old depth was
            // already valid and cannot make a valid replacement fail.
            depth: self.size.depth.max(child.depth.saturating_add(1)),
        };
        check_value_size(size)?;
        self.size = size;
        self.entries = entries;
        Ok(())
    }
}

pub(crate) fn check_limit(
    resource: &'static str,
    actual: usize,
    limit: usize,
) -> ExpressionResult<()> {
    if actual > limit {
        tracing::warn!(target: "nebula_expression::dos", resource, limit, actual, "resource limit exceeded");
        return Err(ExpressionError::ResourceLimitExceeded {
            resource,
            limit,
            actual,
        });
    }
    Ok(())
}

pub(crate) fn check_value_limits(value: &RuntimeValue) -> ExpressionResult<()> {
    measure_value(value).map(|_| ())
}

fn check_value_size(size: ValueSize) -> ExpressionResult<()> {
    check_limit("result content bytes", size.content_bytes, MAX_RESULT_BYTES)?;
    check_limit("result value nodes", size.nodes, MAX_RESULT_NODES)?;
    check_limit("result value depth", size.depth, MAX_AST_DEPTH)
}

/// The byte cost of one value's local representation, excluding children.
fn local_value_bytes(value: &RuntimeValue) -> usize {
    match value {
        RuntimeValue::Null | RuntimeValue::Undefined => 4,
        RuntimeValue::Bool(true) => 4,
        RuntimeValue::Bool(false) => 5,
        RuntimeValue::Integer(number) => number.to_string().len(),
        RuntimeValue::Unsigned(number) => number.to_string().len(),
        RuntimeValue::Float(number) => number.to_string().len(),
        RuntimeValue::String(text) => text.len(),
        RuntimeValue::DateTime(_) => DATE_TIME_JSON_BYTES,
        RuntimeValue::Array(values) => 2_usize.saturating_add(values.len().saturating_sub(1)),
        RuntimeValue::Object(values) => {
            let key_bytes = values.keys().fold(0_usize, |bytes, key| {
                bytes.saturating_add(key.len()).saturating_add(3)
            });
            2_usize
                .saturating_add(values.len().saturating_sub(1))
                .saturating_add(key_bytes)
        },
    }
}

pub(crate) fn measure_value(value: &RuntimeValue) -> ExpressionResult<ValueSize> {
    let mut nodes = 0_usize;
    let mut content_bytes = 0_usize;
    let mut max_depth = 0_usize;
    let mut pending = vec![(value, 1_usize)];

    while let Some((value, depth)) = pending.pop() {
        check_limit("result value depth", depth, MAX_AST_DEPTH)?;
        max_depth = max_depth.max(depth);
        nodes = nodes.saturating_add(1);
        check_limit("result value nodes", nodes, MAX_RESULT_NODES)?;

        match value {
            RuntimeValue::Array(values) => {
                check_limit(
                    "result value nodes",
                    nodes
                        .saturating_add(pending.len())
                        .saturating_add(values.len()),
                    MAX_RESULT_NODES,
                )?;
                pending.extend(values.iter().map(|child| (child, depth + 1)));
            },
            RuntimeValue::Object(values) => {
                check_limit(
                    "result value nodes",
                    nodes
                        .saturating_add(pending.len())
                        .saturating_add(values.len()),
                    MAX_RESULT_NODES,
                )?;
                pending.extend(values.values().map(|child| (child, depth + 1)));
            },
            _ => {},
        }

        content_bytes = content_bytes.saturating_add(local_value_bytes(value));
        check_limit("result content bytes", content_bytes, MAX_RESULT_BYTES)?;
    }

    Ok(ValueSize {
        content_bytes,
        nodes,
        depth: max_depth,
    })
}

pub(crate) fn check_object_snapshot_limits<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a RuntimeValue)>,
) -> ExpressionResult<()> {
    let mut budget = AggregateValueBudget::new()?;
    for (key, value) in entries {
        budget.insert_object_value(key, value, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn array_budget_rejects_aggregate_before_accepting_oversized_child() {
        let child = RuntimeValue::string("x".repeat(MAX_RESULT_BYTES / 2));
        let mut budget = AggregateValueBudget::new().unwrap();

        budget.push_array_value(&child).unwrap();
        assert_matches!(
            budget.push_array_value(&child),
            Err(ExpressionError::ResourceLimitExceeded {
                resource: "result content bytes",
                ..
            })
        );
    }

    #[test]
    fn object_budget_replacement_does_not_accumulate_discarded_values() {
        let child = RuntimeValue::string("x".repeat(MAX_RESULT_BYTES / 2));
        let mut budget = AggregateValueBudget::new().unwrap();

        budget.insert_object_value("key", &child, None).unwrap();
        budget
            .insert_object_value("key", &child, Some(&child))
            .unwrap();
    }

    #[test]
    fn date_time_inside_a_container_is_measured_and_accepted() {
        let instant = chrono::DateTime::parse_from_rfc3339("2024-03-01T10:00:00+03:00").unwrap();
        let value = RuntimeValue::array(vec![RuntimeValue::DateTime(instant)]);
        let size = measure_value(&value).unwrap();
        assert_eq!(size.nodes, 2);
        assert_eq!(size.depth, 2);
        assert!(size.content_bytes >= DATE_TIME_JSON_BYTES);
    }

    #[test]
    fn undefined_costs_the_same_as_null() {
        assert_eq!(
            measure_value(&RuntimeValue::Undefined)
                .unwrap()
                .content_bytes,
            measure_value(&RuntimeValue::Null).unwrap().content_bytes
        );
    }
}
