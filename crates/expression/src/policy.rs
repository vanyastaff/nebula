//! Policy controls for expression evaluation.
//!
//! Policies can constrain which builtin functions are callable and carry
//! compatibility flags such as strict mode.

use std::{collections::HashSet, num::NonZeroUsize, sync::Arc};

/// A non-zero ceiling for one builtin output dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltinOutputBound(NonZeroUsize);

impl BuiltinOutputBound {
    /// Construct an output bound, returning `None` when `limit` is zero.
    pub const fn new(limit: usize) -> Option<Self> {
        match NonZeroUsize::new(limit) {
            Some(limit) => Some(Self(limit)),
            None => None,
        }
    }

    /// Return the configured bound.
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

impl From<NonZeroUsize> for BuiltinOutputBound {
    fn from(limit: NonZeroUsize) -> Self {
        Self(limit)
    }
}

/// Finite limits applied to every registered builtin result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinOutputLimits {
    total_bytes: usize,
    string_bytes: usize,
    collection_items: usize,
    value_nodes: usize,
    value_depth: usize,
}

impl BuiltinOutputLimits {
    /// Default maximum total content bytes in one output tree.
    pub const DEFAULT_MAX_TOTAL_BYTES: usize = crate::limits::MAX_RESULT_BYTES;
    /// Default maximum UTF-8 byte length of one output string or object key.
    pub const DEFAULT_MAX_STRING_BYTES: usize = crate::limits::MAX_RESULT_BYTES;
    /// Default maximum number of direct entries in one output array or object.
    pub const DEFAULT_MAX_COLLECTION_ITEMS: usize = crate::limits::MAX_RESULT_NODES;
    /// Default maximum number of JSON values in one builtin output tree.
    pub const DEFAULT_MAX_VALUE_NODES: usize = crate::limits::MAX_RESULT_NODES;
    /// Default maximum depth of one builtin output tree.
    pub const DEFAULT_MAX_VALUE_DEPTH: usize = crate::limits::MAX_AST_DEPTH;

    /// Maximum total content bytes in one output tree.
    #[must_use]
    pub fn max_total_bytes(self) -> usize {
        self.total_bytes
    }

    /// Maximum UTF-8 byte length of one output string or object key.
    #[must_use]
    pub fn max_string_bytes(self) -> usize {
        self.string_bytes
    }

    /// Maximum number of direct entries in one output array or object.
    #[must_use]
    pub fn max_collection_items(self) -> usize {
        self.collection_items
    }

    /// Maximum number of JSON values in one output tree.
    #[must_use]
    pub fn max_value_nodes(self) -> usize {
        self.value_nodes
    }

    /// Maximum depth of one output tree, counting the root as depth one.
    #[must_use]
    pub fn max_value_depth(self) -> usize {
        self.value_depth
    }

    pub(crate) fn most_restrictive(self, other: Self) -> Self {
        Self {
            total_bytes: self.total_bytes.min(other.total_bytes),
            string_bytes: self.string_bytes.min(other.string_bytes),
            collection_items: self.collection_items.min(other.collection_items),
            value_nodes: self.value_nodes.min(other.value_nodes),
            value_depth: self.value_depth.min(other.value_depth),
        }
    }
}

impl Default for BuiltinOutputLimits {
    fn default() -> Self {
        Self {
            total_bytes: Self::DEFAULT_MAX_TOTAL_BYTES,
            string_bytes: Self::DEFAULT_MAX_STRING_BYTES,
            collection_items: Self::DEFAULT_MAX_COLLECTION_ITEMS,
            value_nodes: Self::DEFAULT_MAX_VALUE_NODES,
            value_depth: Self::DEFAULT_MAX_VALUE_DEPTH,
        }
    }
}

/// A non-zero ceiling for work performed by one expression evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvaluationStepLimit(NonZeroUsize);

impl EvaluationStepLimit {
    /// Construct a step limit, returning `None` when `max_steps` is zero.
    pub const fn new(max_steps: usize) -> Option<Self> {
        match NonZeroUsize::new(max_steps) {
            Some(max_steps) => Some(Self(max_steps)),
            None => None,
        }
    }

    /// Return the configured maximum number of evaluation steps.
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

impl From<NonZeroUsize> for EvaluationStepLimit {
    fn from(max_steps: NonZeroUsize) -> Self {
        Self(max_steps)
    }
}

/// Evaluation policy applied by the engine and optionally tightened by context.
#[derive(Debug, Clone, Default)]
pub struct EvaluationPolicy {
    allowed_functions: Option<Arc<HashSet<String>>>,
    denied_functions: Arc<HashSet<String>>,
    strict_mode: bool,
    strict_conversion_functions: bool,
    strict_numeric_comparisons: bool,
    max_json_parse_length: Option<usize>,
    max_eval_steps: Option<EvaluationStepLimit>,
    builtin_output_limits: BuiltinOutputLimits,
}

impl EvaluationPolicy {
    /// Create an empty policy (no function restrictions, strict mode off).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a policy that only allows the provided functions.
    pub fn allow_only<I, S>(allowed_functions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new().with_allowed_functions(allowed_functions)
    }

    /// Set the function allowlist.
    pub fn with_allowed_functions<I, S>(mut self, allowed_functions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let allowset: HashSet<String> = allowed_functions.into_iter().map(Into::into).collect();
        self.allowed_functions = Some(Arc::new(allowset));
        self
    }

    /// Set the function denylist.
    pub fn with_denied_functions<I, S>(mut self, denied_functions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let denyset: HashSet<String> = denied_functions.into_iter().map(Into::into).collect();
        self.denied_functions = Arc::new(denyset);
        self
    }

    /// Enable or disable strict mode.
    ///
    /// Strict mode is currently a compatibility flag for upcoming
    /// coercion-hardening behavior.
    pub fn with_strict_mode(mut self, enabled: bool) -> Self {
        self.strict_mode = enabled;
        self
    }

    /// Enable or disable strict behavior for explicit conversion builtins.
    ///
    /// When enabled, conversion builtins like `to_number` / `to_boolean`
    /// stop coercing non-native types and require native inputs.
    pub fn with_strict_conversion_functions(mut self, enabled: bool) -> Self {
        self.strict_conversion_functions = enabled;
        self
    }

    /// Enable or disable strict numeric-only relational comparisons.
    ///
    /// When enabled, relational operators (`<`, `>`, `<=`, `>=`) only accept
    /// number-vs-number operands.
    pub fn with_strict_numeric_comparisons(mut self, enabled: bool) -> Self {
        self.strict_numeric_comparisons = enabled;
        self
    }

    /// Set max JSON input size for `parse_json` (engine default: 1 MiB).
    /// A context can tighten, but cannot raise, the engine's limit.
    pub fn with_max_json_parse_length(mut self, max_bytes: usize) -> Self {
        self.max_json_parse_length = Some(max_bytes);
        self
    }

    /// Set maximum evaluation steps before aborting.
    ///
    /// AST nodes, materialized values, string bytes, and builtin work consume
    /// one shared budget across an entire compiled program. The engine default
    /// is 100,000 units. A context cannot raise the engine's effective ceiling.
    /// Exceeding the ceiling returns `ExpressionError::StepLimitExceeded`.
    ///
    pub fn with_max_eval_steps(mut self, limit: EvaluationStepLimit) -> Self {
        self.max_eval_steps = Some(limit);
        self
    }

    /// Set the maximum total content bytes in a builtin output tree.
    ///
    /// A raw integer, including zero, is intentionally not accepted:
    ///
    /// ```compile_fail
    /// use nebula_expression::EvaluationPolicy;
    /// let _ = EvaluationPolicy::new().with_max_builtin_output_bytes(0);
    /// ```
    #[must_use]
    pub fn with_max_builtin_output_bytes(mut self, max_bytes: BuiltinOutputBound) -> Self {
        self.builtin_output_limits.total_bytes = max_bytes
            .get()
            .min(BuiltinOutputLimits::DEFAULT_MAX_TOTAL_BYTES);
        self
    }

    /// Set the maximum UTF-8 byte length of any string or key returned by a builtin.
    ///
    #[must_use]
    pub fn with_max_builtin_output_string_bytes(mut self, max_bytes: BuiltinOutputBound) -> Self {
        self.builtin_output_limits.string_bytes = max_bytes
            .get()
            .min(BuiltinOutputLimits::DEFAULT_MAX_STRING_BYTES);
        self
    }

    /// Set the maximum number of direct entries in a builtin output collection.
    ///
    #[must_use]
    pub fn with_max_builtin_output_collection_items(
        mut self,
        max_items: BuiltinOutputBound,
    ) -> Self {
        self.builtin_output_limits.collection_items = max_items
            .get()
            .min(BuiltinOutputLimits::DEFAULT_MAX_COLLECTION_ITEMS);
        self
    }

    /// Set the maximum number of JSON values in one builtin output tree.
    ///
    #[must_use]
    pub fn with_max_builtin_output_nodes(mut self, max_nodes: BuiltinOutputBound) -> Self {
        self.builtin_output_limits.value_nodes = max_nodes
            .get()
            .min(BuiltinOutputLimits::DEFAULT_MAX_VALUE_NODES);
        self
    }

    /// Set the maximum depth of one builtin output tree.
    ///
    #[must_use]
    pub fn with_max_builtin_output_depth(mut self, max_depth: BuiltinOutputBound) -> Self {
        self.builtin_output_limits.value_depth = max_depth
            .get()
            .min(BuiltinOutputLimits::DEFAULT_MAX_VALUE_DEPTH);
        self
    }

    /// Return the optional allowlist.
    pub fn allowed_functions(&self) -> Option<&HashSet<String>> {
        self.allowed_functions.as_deref()
    }

    /// Return the denylist.
    pub fn denied_functions(&self) -> &HashSet<String> {
        self.denied_functions.as_ref()
    }

    /// Whether strict mode is enabled.
    pub fn strict_mode(&self) -> bool {
        self.strict_mode
    }

    /// Whether strict conversion builtins mode is enabled.
    pub fn strict_conversion_functions(&self) -> bool {
        self.strict_conversion_functions
    }

    /// Whether strict numeric-only relational comparisons are enabled.
    pub fn strict_numeric_comparisons(&self) -> bool {
        self.strict_numeric_comparisons
    }

    /// Optional override for JSON parse input size limit.
    pub fn max_json_parse_length(&self) -> Option<usize> {
        self.max_json_parse_length
    }

    /// Explicit work limit, or `None` to inherit the engine's default ceiling.
    pub fn max_eval_steps(&self) -> Option<usize> {
        self.max_eval_steps.map(EvaluationStepLimit::get)
    }

    /// Finite limits applied to results from registered builtins.
    #[must_use]
    pub fn builtin_output_limits(&self) -> BuiltinOutputLimits {
        self.builtin_output_limits
    }
}

#[cfg(test)]
mod tests {
    use super::{BuiltinOutputBound, EvaluationPolicy, EvaluationStepLimit};

    #[test]
    fn builtin_output_bound_rejects_zero() {
        assert_eq!(BuiltinOutputBound::new(0), None);
        assert_eq!(
            BuiltinOutputBound::new(1).map(BuiltinOutputBound::get),
            Some(1)
        );
    }

    #[test]
    fn test_policy_builder_sets_fields() {
        let output_bound = BuiltinOutputBound::new(64).unwrap();
        let policy = EvaluationPolicy::new()
            .with_allowed_functions(["uppercase", "length"])
            .with_denied_functions(["length"])
            .with_strict_mode(true)
            .with_strict_conversion_functions(true)
            .with_strict_numeric_comparisons(true)
            .with_max_json_parse_length(2048)
            .with_max_builtin_output_bytes(output_bound)
            .with_max_builtin_output_string_bytes(output_bound)
            .with_max_builtin_output_collection_items(output_bound)
            .with_max_builtin_output_nodes(output_bound)
            .with_max_builtin_output_depth(output_bound);

        assert!(policy.allowed_functions().unwrap().contains("uppercase"));
        assert!(policy.denied_functions().contains("length"));
        assert!(policy.strict_mode());
        assert!(policy.strict_conversion_functions());
        assert!(policy.strict_numeric_comparisons());
        assert_eq!(policy.max_json_parse_length(), Some(2048));
        let limits = policy.builtin_output_limits();
        assert_eq!(limits.max_total_bytes(), 64);
        assert_eq!(limits.max_string_bytes(), 64);
        assert_eq!(limits.max_collection_items(), 64);
        assert_eq!(limits.max_value_nodes(), 64);
        assert_eq!(limits.max_value_depth(), 64);
    }

    #[test]
    fn evaluation_step_limit_rejects_zero() {
        assert_eq!(EvaluationStepLimit::new(0), None);
    }

    #[test]
    fn with_max_eval_steps_accepts_one() {
        // Smallest legitimate budget — still useful for "every call must
        // fail-fast" testing scenarios.
        let limit = EvaluationStepLimit::new(1).unwrap();
        let policy = EvaluationPolicy::new().with_max_eval_steps(limit);
        assert_eq!(policy.max_eval_steps(), Some(1));
    }
}
