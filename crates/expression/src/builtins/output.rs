//! Bounded construction for registered builtin outputs.

use std::{collections::BTreeMap, fmt};

use chrono::{DateTime, FixedOffset};

use crate::{
    ExpressionError, error::ExpressionResult, policy::BuiltinOutputLimits, value::RuntimeValue,
};

/// Output dimension rejected by [`BuiltinOutputBuilder`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinOutputLimit {
    /// Total content bytes in the complete output tree.
    TotalBytes,
    /// UTF-8 bytes in one string value or object key.
    StringBytes,
    /// Direct entries in one array or object.
    CollectionItems,
    /// JSON value nodes in the complete output tree.
    ValueNodes,
    /// Nested JSON value depth, counting the root as depth one.
    ValueDepth,
}

impl fmt::Display for BuiltinOutputLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TotalBytes => formatter.write_str("total bytes"),
            Self::StringBytes => formatter.write_str("string bytes"),
            Self::CollectionItems => formatter.write_str("collection items"),
            Self::ValueNodes => formatter.write_str("value nodes"),
            Self::ValueDepth => formatter.write_str("value depth"),
        }
    }
}

/// The byte cost of an RFC 3339 date-time rendering with an offset.
const DATE_TIME_JSON_BYTES: usize = 35;

#[derive(Debug, Clone, Copy, Default)]
struct OutputSize {
    total_bytes: usize,
    max_string_bytes: usize,
    max_collection_items: usize,
    value_nodes: usize,
    max_depth: usize,
}

impl OutputSize {
    /// Seed for a container that will hold one or more children.
    ///
    /// `total_bytes` counts the opening and closing delimiters, `value_nodes`
    /// the container itself, and `max_depth` the root level. Every container
    /// budget starts from this shape before children are folded in, so the
    /// seed lives here instead of being re-written per constructor.
    const fn container() -> Self {
        Self {
            total_bytes: 2,
            value_nodes: 1,
            max_depth: 1,
            max_string_bytes: 0,
            max_collection_items: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArrayOutputBudget {
    builder: BuiltinOutputBuilder,
    size: OutputSize,
    direct_items: usize,
}

impl ArrayOutputBudget {
    pub(crate) fn new(builder: BuiltinOutputBuilder) -> ExpressionResult<Self> {
        let size = OutputSize::container();
        builder.ensure_output_size(size)?;
        Ok(Self {
            builder,
            size,
            direct_items: 0,
        })
    }

    pub(crate) fn push(&mut self, value: &RuntimeValue) -> ExpressionResult<()> {
        let child = self.builder.measure(value)?;
        let direct_items = self.direct_items.saturating_add(1);
        let mut size = self.size;
        size.total_bytes = size
            .total_bytes
            .saturating_add(usize::from(direct_items > 1))
            .saturating_add(child.total_bytes);
        size.max_string_bytes = size.max_string_bytes.max(child.max_string_bytes);
        size.max_collection_items = size
            .max_collection_items
            .max(direct_items)
            .max(child.max_collection_items);
        size.value_nodes = size.value_nodes.saturating_add(child.value_nodes);
        size.max_depth = size.max_depth.max(child.max_depth.saturating_add(1));
        self.builder.ensure_output_size(size)?;
        self.size = size;
        self.direct_items = direct_items;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GroupOutputBudget {
    builder: BuiltinOutputBuilder,
    size: OutputSize,
    groups: usize,
}

impl GroupOutputBudget {
    pub(crate) fn new(builder: BuiltinOutputBuilder) -> ExpressionResult<Self> {
        let size = OutputSize::container();
        builder.ensure_output_size(size)?;
        Ok(Self {
            builder,
            size,
            groups: 0,
        })
    }

    pub(crate) fn push(
        &mut self,
        key: &str,
        existing_items: usize,
        value: &RuntimeValue,
    ) -> ExpressionResult<()> {
        let child = self.builder.measure(value)?;
        let mut size = self.size;
        if existing_items == 0 {
            let groups = self.groups.saturating_add(1);
            size.total_bytes = size
                .total_bytes
                .saturating_add(usize::from(groups > 1))
                .saturating_add(key.len())
                .saturating_add(5)
                .saturating_add(child.total_bytes);
            size.value_nodes = size
                .value_nodes
                .saturating_add(1)
                .saturating_add(child.value_nodes);
            size.max_collection_items = size
                .max_collection_items
                .max(groups)
                .max(1)
                .max(child.max_collection_items);
            size.max_string_bytes = size
                .max_string_bytes
                .max(key.len())
                .max(child.max_string_bytes);
            size.max_depth = size.max_depth.max(child.max_depth.saturating_add(2));
            self.builder.ensure_output_size(size)?;
            self.groups = groups;
        } else {
            let group_items = existing_items.saturating_add(1);
            size.total_bytes = size
                .total_bytes
                .saturating_add(1)
                .saturating_add(child.total_bytes);
            size.value_nodes = size.value_nodes.saturating_add(child.value_nodes);
            size.max_collection_items = size
                .max_collection_items
                .max(group_items)
                .max(child.max_collection_items);
            size.max_string_bytes = size.max_string_bytes.max(child.max_string_bytes);
            size.max_depth = size.max_depth.max(child.max_depth.saturating_add(2));
            self.builder.ensure_output_size(size)?;
        }
        self.size = size;
        Ok(())
    }
}

/// Opaque, policy-bounded result from a registered builtin.
///
/// Values of this type can only be created through [`BuiltinOutputBuilder`].
/// This prevents a public extension from returning an unchecked value to the
/// evaluator.
#[must_use]
pub struct BuiltinOutput {
    value: RuntimeValue,
    size: OutputSize,
}

impl fmt::Debug for BuiltinOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltinOutput")
            .field("total_bytes", &self.size.total_bytes)
            .field("max_string_bytes", &self.size.max_string_bytes)
            .field("max_collection_items", &self.size.max_collection_items)
            .field("value_nodes", &self.size.value_nodes)
            .field("max_depth", &self.size.max_depth)
            .finish_non_exhaustive()
    }
}

impl BuiltinOutput {
    pub(crate) fn into_value(self) -> RuntimeValue {
        self.value
    }
}

/// Mandatory capability used to construct a registered builtin's result.
///
/// String and repeat constructors check their exact output size before
/// allocating. Collection constructors stop before accepting an item beyond
/// their configured bound. The raw value constructor is crate-private so
/// public extensions cannot bypass these checks.
#[must_use]
#[derive(Debug, Clone, Copy)]
pub struct BuiltinOutputBuilder {
    limits: BuiltinOutputLimits,
}

impl BuiltinOutputBuilder {
    pub(crate) fn new(limits: BuiltinOutputLimits) -> Self {
        Self { limits }
    }

    /// Construct a missing value (`Undefined`).
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] when the scalar
    /// exceeds a configured output bound.
    pub fn undefined(self) -> ExpressionResult<BuiltinOutput> {
        self.scalar(RuntimeValue::Undefined)
    }

    /// Construct JSON null.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] when the scalar
    /// exceeds a configured output bound.
    pub fn null(self) -> ExpressionResult<BuiltinOutput> {
        self.scalar(RuntimeValue::Null)
    }

    /// Construct a boolean.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] when the scalar
    /// exceeds a configured output bound.
    pub fn boolean(self, value: bool) -> ExpressionResult<BuiltinOutput> {
        self.scalar(RuntimeValue::Bool(value))
    }

    /// Construct a signed integer.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] when the scalar
    /// exceeds a configured output bound.
    pub fn signed_integer(self, value: i64) -> ExpressionResult<BuiltinOutput> {
        self.scalar(RuntimeValue::Integer(value))
    }

    /// Construct an unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] when the scalar
    /// exceeds a configured output bound.
    pub fn unsigned_integer(self, value: u64) -> ExpressionResult<BuiltinOutput> {
        self.scalar(RuntimeValue::Unsigned(value))
    }

    /// Construct a finite floating-point number.
    ///
    /// # Errors
    ///
    /// Returns an evaluation error when `value` is NaN or infinite.
    pub fn float(self, value: f64) -> ExpressionResult<BuiltinOutput> {
        if value.is_finite() {
            self.scalar(RuntimeValue::Float(value))
        } else {
            Err(ExpressionError::eval_error(
                "builtin produced a non-finite number",
            ))
        }
    }

    /// Construct a date-time value.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] when the scalar
    /// exceeds a configured output bound.
    pub fn date_time(self, value: DateTime<FixedOffset>) -> ExpressionResult<BuiltinOutput> {
        self.scalar(RuntimeValue::DateTime(value))
    }

    /// Copy a borrowed string after checking its byte length.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] before allocation
    /// when `value` exceeds the configured string bound.
    pub fn string(self, value: &str) -> ExpressionResult<BuiltinOutput> {
        let size = OutputSize {
            total_bytes: value.len(),
            max_string_bytes: value.len(),
            ..OutputSize::container()
        };
        self.ensure_output_size(size)?;
        Ok(BuiltinOutput {
            value: RuntimeValue::string(value),
            size,
        })
    }

    /// Repeat a string after checking the exact byte length of the result.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] before allocation
    /// when the repeated string would exceed the configured bound.
    pub fn repeat_string(self, value: &str, count: usize) -> ExpressionResult<BuiltinOutput> {
        let output_bytes = value.len().saturating_mul(count);
        let size = OutputSize {
            total_bytes: output_bytes,
            max_string_bytes: output_bytes,
            ..OutputSize::container()
        };
        self.ensure_output_size(size)?;
        Ok(BuiltinOutput {
            value: RuntimeValue::string(value.repeat(count)),
            size,
        })
    }

    /// Clone an existing runtime value only after validating its complete shape.
    ///
    /// # Errors
    ///
    /// Returns [`ExpressionError::BuiltinOutputLimitExceeded`] without cloning
    /// when any configured limit would be exceeded.
    pub fn clone_value(self, value: &RuntimeValue) -> ExpressionResult<BuiltinOutput> {
        let size = self.measure(value)?;
        Ok(BuiltinOutput {
            value: value.clone(),
            size,
        })
    }

    /// Construct an array from already bounded child outputs.
    ///
    /// # Errors
    ///
    /// Stops and returns [`ExpressionError::BuiltinOutputLimitExceeded`] before
    /// accepting an item beyond the collection or total-node bound.
    pub fn array(
        self,
        values: impl IntoIterator<Item = BuiltinOutput>,
    ) -> ExpressionResult<BuiltinOutput> {
        let mut array = Vec::new();
        let mut size = OutputSize::container();
        for output in values {
            let next_items = array.len().saturating_add(1);
            size.total_bytes = size
                .total_bytes
                .saturating_add(usize::from(next_items > 1))
                .saturating_add(output.size.total_bytes);
            size.max_string_bytes = size.max_string_bytes.max(output.size.max_string_bytes);
            size.max_collection_items = size
                .max_collection_items
                .max(next_items)
                .max(output.size.max_collection_items);
            size.value_nodes = size.value_nodes.saturating_add(output.size.value_nodes);
            size.max_depth = size.max_depth.max(output.size.max_depth.saturating_add(1));
            self.ensure_output_size(size)?;
            array.push(output.value);
        }
        Ok(BuiltinOutput {
            value: RuntimeValue::Array(array.into()),
            size,
        })
    }

    /// Construct an object from checked keys and bounded child outputs.
    ///
    /// # Errors
    ///
    /// Stops and returns [`ExpressionError::BuiltinOutputLimitExceeded`] before
    /// accepting an entry beyond the configured bounds.
    pub fn object<K>(
        self,
        entries: impl IntoIterator<Item = (K, BuiltinOutput)>,
    ) -> ExpressionResult<BuiltinOutput>
    where
        K: AsRef<str>,
    {
        let mut object = BTreeMap::new();
        let mut entry_sizes = BTreeMap::new();
        let mut size = object_output_size(&entry_sizes);
        self.ensure_output_size(size)?;
        for (key, output) in entries {
            let key = key.as_ref().to_owned();
            let candidate_size = if entry_sizes.contains_key(&key) {
                let mut candidate_entries = entry_sizes.clone();
                candidate_entries.insert(key.clone(), output.size);
                object_output_size(&candidate_entries)
            } else {
                object_output_size_with_entry(
                    size,
                    &key,
                    object.len().saturating_add(1),
                    output.size,
                )
            };
            self.ensure_output_size(candidate_size)?;
            size = candidate_size;
            entry_sizes.insert(key.clone(), output.size);
            object.insert(std::sync::Arc::<str>::from(key.as_str()), output.value);
        }
        Ok(BuiltinOutput {
            value: RuntimeValue::Object(std::sync::Arc::new(object)),
            size,
        })
    }

    pub(crate) fn accept(self, output: BuiltinOutput) -> ExpressionResult<BuiltinOutput> {
        self.ensure_output_size(output.size)?;
        Ok(output)
    }

    pub(crate) fn value(self, value: RuntimeValue) -> ExpressionResult<BuiltinOutput> {
        let size = self.measure(&value)?;
        Ok(BuiltinOutput { value, size })
    }

    pub(crate) fn preflight_array<'a>(
        self,
        values: impl IntoIterator<Item = &'a RuntimeValue>,
    ) -> ExpressionResult<()> {
        let mut size = OutputSize::container();
        let mut direct_items = 0usize;
        for value in values {
            let child = self.measure(value)?;
            direct_items = direct_items.saturating_add(1);
            size.total_bytes = size
                .total_bytes
                .saturating_add(usize::from(direct_items > 1))
                .saturating_add(child.total_bytes);
            size.max_string_bytes = size.max_string_bytes.max(child.max_string_bytes);
            size.max_collection_items = size
                .max_collection_items
                .max(direct_items)
                .max(child.max_collection_items);
            size.value_nodes = size.value_nodes.saturating_add(child.value_nodes);
            size.max_depth = size.max_depth.max(child.max_depth.saturating_add(1));
            self.ensure_output_size(size)?;
        }
        Ok(())
    }

    pub(crate) fn preflight_object<'a>(
        self,
        entries: impl IntoIterator<Item = (&'a str, &'a RuntimeValue)>,
    ) -> ExpressionResult<()> {
        let mut size = OutputSize::container();
        let mut direct_items = 0usize;
        for (key, value) in entries {
            let child = self.measure(value)?;
            direct_items = direct_items.saturating_add(1);
            size.total_bytes = size
                .total_bytes
                .saturating_add(usize::from(direct_items > 1))
                .saturating_add(key.len())
                .saturating_add(3)
                .saturating_add(child.total_bytes);
            size.max_string_bytes = size
                .max_string_bytes
                .max(key.len())
                .max(child.max_string_bytes);
            size.max_collection_items = size
                .max_collection_items
                .max(direct_items)
                .max(child.max_collection_items);
            size.value_nodes = size.value_nodes.saturating_add(child.value_nodes);
            size.max_depth = size.max_depth.max(child.max_depth.saturating_add(1));
            self.ensure_output_size(size)?;
        }
        Ok(())
    }

    pub(crate) fn preflight_entries(
        self,
        entries: &BTreeMap<std::sync::Arc<str>, RuntimeValue>,
    ) -> ExpressionResult<()> {
        let mut size = OutputSize {
            max_collection_items: entries.len(),
            ..OutputSize::container()
        };
        for (index, (key, value)) in entries.iter().enumerate() {
            let child = self.measure(value)?;
            size.total_bytes = size
                .total_bytes
                .saturating_add(usize::from(index > 0))
                .saturating_add(17)
                .saturating_add(key.len())
                .saturating_add(child.total_bytes);
            size.max_string_bytes = size
                .max_string_bytes
                .max(5)
                .max(key.len())
                .max(child.max_string_bytes);
            size.max_collection_items = size.max_collection_items.max(2);
            size.value_nodes = size
                .value_nodes
                .saturating_add(2)
                .saturating_add(child.value_nodes);
            size.max_depth = size.max_depth.max(3).max(child.max_depth.saturating_add(2));
            self.ensure_output_size(size)?;
        }
        Ok(())
    }

    pub(crate) fn ensure_string_bytes(self, actual: usize) -> ExpressionResult<()> {
        self.ensure(
            BuiltinOutputLimit::StringBytes,
            self.limits.max_string_bytes(),
            actual,
        )
    }

    pub(crate) fn ensure_collection_items(self, actual: usize) -> ExpressionResult<()> {
        self.ensure(
            BuiltinOutputLimit::CollectionItems,
            self.limits.max_collection_items(),
            actual,
        )
    }

    fn scalar(self, value: RuntimeValue) -> ExpressionResult<BuiltinOutput> {
        let total_bytes = match &value {
            RuntimeValue::Null | RuntimeValue::Undefined => 4,
            RuntimeValue::Bool(true) => 4,
            RuntimeValue::Bool(false) => 5,
            RuntimeValue::Integer(number) => number.to_string().len(),
            RuntimeValue::Unsigned(number) => number.to_string().len(),
            RuntimeValue::Float(number) => number.to_string().len(),
            RuntimeValue::DateTime(_) => DATE_TIME_JSON_BYTES,
            RuntimeValue::String(_) | RuntimeValue::Array(_) | RuntimeValue::Object(_) => 0,
        };
        let size = OutputSize {
            total_bytes,
            ..OutputSize::container()
        };
        self.ensure_output_size(size)?;
        Ok(BuiltinOutput { value, size })
    }

    fn measure(self, root: &RuntimeValue) -> ExpressionResult<OutputSize> {
        let mut pending = vec![(root, 1usize)];
        let mut size = OutputSize::default();
        while let Some((value, depth)) = pending.pop() {
            size.value_nodes = size.value_nodes.saturating_add(1);
            self.ensure_value_nodes(size.value_nodes)?;
            size.max_depth = size.max_depth.max(depth);
            self.ensure_value_depth(size.max_depth)?;
            let local_bytes = match value {
                RuntimeValue::Null | RuntimeValue::Undefined => 4,
                RuntimeValue::Bool(true) => 4,
                RuntimeValue::Bool(false) => 5,
                RuntimeValue::Integer(number) => number.to_string().len(),
                RuntimeValue::Unsigned(number) => number.to_string().len(),
                RuntimeValue::Float(number) => number.to_string().len(),
                RuntimeValue::DateTime(_) => DATE_TIME_JSON_BYTES,
                RuntimeValue::String(string) => {
                    size.max_string_bytes = size.max_string_bytes.max(string.len());
                    self.ensure_string_bytes(size.max_string_bytes)?;
                    string.len()
                },
                RuntimeValue::Array(values) => {
                    size.max_collection_items = size.max_collection_items.max(values.len());
                    self.ensure_collection_items(size.max_collection_items)?;
                    self.ensure_value_nodes(
                        size.value_nodes
                            .saturating_add(pending.len())
                            .saturating_add(values.len()),
                    )?;
                    pending.extend(values.iter().map(|value| (value, depth.saturating_add(1))));
                    2usize.saturating_add(values.len().saturating_sub(1))
                },
                RuntimeValue::Object(entries) => {
                    size.max_collection_items = size.max_collection_items.max(entries.len());
                    self.ensure_collection_items(size.max_collection_items)?;
                    self.ensure_value_nodes(
                        size.value_nodes
                            .saturating_add(pending.len())
                            .saturating_add(entries.len()),
                    )?;
                    let mut key_bytes = 0usize;
                    for (key, value) in entries.iter() {
                        size.max_string_bytes = size.max_string_bytes.max(key.len());
                        self.ensure_string_bytes(size.max_string_bytes)?;
                        key_bytes = key_bytes.saturating_add(key.len()).saturating_add(3);
                        pending.push((value, depth.saturating_add(1)));
                    }
                    2usize
                        .saturating_add(entries.len().saturating_sub(1))
                        .saturating_add(key_bytes)
                },
            };
            size.total_bytes = size.total_bytes.saturating_add(local_bytes);
            self.ensure_total_bytes(size.total_bytes)?;
        }
        Ok(size)
    }

    fn ensure_output_size(self, size: OutputSize) -> ExpressionResult<()> {
        self.ensure_total_bytes(size.total_bytes)?;
        self.ensure_string_bytes(size.max_string_bytes)?;
        self.ensure_collection_items(size.max_collection_items)?;
        self.ensure_value_nodes(size.value_nodes)?;
        self.ensure_value_depth(size.max_depth)
    }

    pub(crate) fn ensure_total_bytes(self, actual: usize) -> ExpressionResult<()> {
        self.ensure(
            BuiltinOutputLimit::TotalBytes,
            self.limits.max_total_bytes(),
            actual,
        )
    }

    pub(crate) fn ensure_value_nodes(self, actual: usize) -> ExpressionResult<()> {
        self.ensure(
            BuiltinOutputLimit::ValueNodes,
            self.limits.max_value_nodes(),
            actual,
        )
    }

    pub(crate) fn ensure_value_depth(self, actual: usize) -> ExpressionResult<()> {
        self.ensure(
            BuiltinOutputLimit::ValueDepth,
            self.limits.max_value_depth(),
            actual,
        )
    }

    fn ensure(
        self,
        dimension: BuiltinOutputLimit,
        limit: usize,
        actual: usize,
    ) -> ExpressionResult<()> {
        if actual <= limit {
            return Ok(());
        }
        tracing::warn!(
            target: "nebula_expression::dos",
            %dimension,
            limit,
            actual,
            "builtin output limit exceeded"
        );
        Err(ExpressionError::builtin_output_limit_exceeded(
            dimension, limit, actual,
        ))
    }
}

fn object_output_size_with_entry(
    mut size: OutputSize,
    key: &str,
    direct_items: usize,
    child: OutputSize,
) -> OutputSize {
    size.total_bytes = size
        .total_bytes
        .saturating_add(usize::from(direct_items > 1))
        .saturating_add(key.len())
        .saturating_add(3)
        .saturating_add(child.total_bytes);
    size.max_string_bytes = size
        .max_string_bytes
        .max(key.len())
        .max(child.max_string_bytes);
    size.max_collection_items = size
        .max_collection_items
        .max(direct_items)
        .max(child.max_collection_items);
    size.value_nodes = size.value_nodes.saturating_add(child.value_nodes);
    size.max_depth = size.max_depth.max(child.max_depth.saturating_add(1));
    size
}

fn object_output_size(entries: &BTreeMap<String, OutputSize>) -> OutputSize {
    let mut size = OutputSize::container();
    for (index, (key, child)) in entries.iter().enumerate() {
        size = object_output_size_with_entry(size, key, index.saturating_add(1), *child);
    }
    size
}

#[cfg(test)]
mod tests {
    use crate::{BuiltinOutputBound, EvaluationPolicy};

    use super::*;

    fn output_bound(limit: usize) -> BuiltinOutputBound {
        BuiltinOutputBound::new(limit).unwrap()
    }

    fn builder_with_limits(
        max_string_bytes: usize,
        max_collection_items: usize,
        max_value_nodes: usize,
    ) -> BuiltinOutputBuilder {
        let policy = EvaluationPolicy::new()
            .with_max_builtin_output_string_bytes(output_bound(max_string_bytes))
            .with_max_builtin_output_collection_items(output_bound(max_collection_items))
            .with_max_builtin_output_nodes(output_bound(max_value_nodes));
        BuiltinOutputBuilder::new(policy.builtin_output_limits())
    }

    #[test]
    fn array_stops_before_accepting_item_beyond_limit() {
        let output = builder_with_limits(16, 2, 8);
        let values = [
            output.signed_integer(1).unwrap(),
            output.signed_integer(2).unwrap(),
            output.signed_integer(3).unwrap(),
        ];

        let error = output.array(values).unwrap_err();
        let ExpressionError::BuiltinOutputLimitExceeded {
            dimension,
            limit,
            actual,
        } = error
        else {
            panic!("expected BuiltinOutputLimitExceeded, got {error:?}");
        };
        assert_eq!(dimension, BuiltinOutputLimit::CollectionItems);
        assert_eq!(limit, 2);
        assert_eq!(actual, 3);
    }

    #[test]
    fn cloned_value_is_measured_before_clone() {
        let output = builder_with_limits(16, 8, 3);
        let value = RuntimeValue::from_json(&serde_json::json!({"items": [1, 2]}));

        let error = output.clone_value(&value).unwrap_err();
        let ExpressionError::BuiltinOutputLimitExceeded {
            dimension,
            limit,
            actual,
        } = error
        else {
            panic!("expected BuiltinOutputLimitExceeded, got {error:?}");
        };
        assert_eq!(dimension, BuiltinOutputLimit::ValueNodes);
        assert_eq!(limit, 3);
        assert_eq!(actual, 4);
    }

    #[test]
    fn collection_revalidates_children_against_its_own_limits() {
        let permissive = builder_with_limits(16, 8, 8);
        let oversized_child = permissive.string("123456789").unwrap();
        let restrictive = builder_with_limits(8, 8, 8);

        let error = restrictive.array([oversized_child]).unwrap_err();
        let ExpressionError::BuiltinOutputLimitExceeded {
            dimension,
            limit,
            actual,
        } = error
        else {
            panic!("expected BuiltinOutputLimitExceeded, got {error:?}");
        };
        assert_eq!(dimension, BuiltinOutputLimit::StringBytes);
        assert_eq!(limit, 8);
        assert_eq!(actual, 9);
    }

    #[test]
    fn object_duplicate_keys_are_charged_after_replacement() {
        let output = builder_with_limits(16, 1, 2);
        let result = output
            .object([
                ("key", output.signed_integer(1).unwrap()),
                ("key", output.signed_integer(2).unwrap()),
            ])
            .unwrap();

        assert_eq!(result.into_value().to_json(), serde_json::json!({"key": 2}));
    }

    #[test]
    fn scalar_is_rejected_by_total_byte_limit() {
        let policy = EvaluationPolicy::new().with_max_builtin_output_bytes(output_bound(2));
        let output = BuiltinOutputBuilder::new(policy.builtin_output_limits());

        let error = output.signed_integer(123).unwrap_err();
        let ExpressionError::BuiltinOutputLimitExceeded {
            dimension,
            limit,
            actual,
        } = error
        else {
            panic!("expected BuiltinOutputLimitExceeded, got {error:?}");
        };
        assert_eq!(dimension, BuiltinOutputLimit::TotalBytes);
        assert_eq!(limit, 2);
        assert_eq!(actual, 3);
    }

    #[test]
    fn borrowed_value_is_rejected_by_depth_limit_before_clone() {
        let policy = EvaluationPolicy::new().with_max_builtin_output_depth(output_bound(2));
        let output = BuiltinOutputBuilder::new(policy.builtin_output_limits());
        let value = RuntimeValue::from_json(&serde_json::json!({"nested": {"value": 1}}));

        let error = output.clone_value(&value).unwrap_err();
        let ExpressionError::BuiltinOutputLimitExceeded {
            dimension,
            limit,
            actual,
        } = error
        else {
            panic!("expected BuiltinOutputLimitExceeded, got {error:?}");
        };
        assert_eq!(dimension, BuiltinOutputLimit::ValueDepth);
        assert_eq!(limit, 2);
        assert_eq!(actual, 3);
    }

    #[test]
    fn date_time_is_a_bounded_scalar() {
        let output = BuiltinOutputBuilder::new(BuiltinOutputLimits::default());
        let instant = DateTime::parse_from_rfc3339("2024-03-01T10:00:00+03:00").unwrap();
        let value = output.date_time(instant).unwrap().into_value();
        assert!(value.as_date_time().is_some());
    }

    #[test]
    fn debug_output_redacts_wrapped_payload() {
        const CANARY: &str = "BUILTIN_OUTPUT_SECRET_CANARY";
        let output = BuiltinOutputBuilder::new(BuiltinOutputLimits::default())
            .clone_value(&RuntimeValue::from_json(
                &serde_json::json!({CANARY: CANARY}),
            ))
            .unwrap();

        let diagnostic = format!("{output:?}");
        assert!(!diagnostic.contains(CANARY), "leaked payload: {diagnostic}");
        assert!(diagnostic.contains("total_bytes"));
        assert!(diagnostic.contains("value_nodes"));
    }

    #[test]
    fn output_limit_errors_redact_rejected_text() {
        const CANARY: &str = "BUILTIN_OUTPUT_LIMIT_SECRET_CANARY";
        let one = BuiltinOutputBound::new(1).unwrap();
        let limits = EvaluationPolicy::new()
            .with_max_builtin_output_string_bytes(one)
            .builtin_output_limits();
        let error = BuiltinOutputBuilder::new(limits)
            .string(CANARY)
            .unwrap_err();

        for diagnostic in [error.to_string(), format!("{error:?}")] {
            assert!(!diagnostic.contains(CANARY), "leaked payload: {diagnostic}");
        }
    }
}
