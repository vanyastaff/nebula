//! Policy controls for expression evaluation.
//!
//! Policies can constrain which builtin functions are callable and carry
//! compatibility flags such as strict mode.

use std::{collections::HashSet, sync::Arc};

/// Evaluation policy applied by the engine and optionally overridden by context.
#[derive(Debug, Clone, Default)]
pub struct EvaluationPolicy {
    allowed_functions: Option<Arc<HashSet<String>>>,
    denied_functions: Arc<HashSet<String>>,
    strict_mode: bool,
    strict_conversion_functions: bool,
    strict_numeric_comparisons: bool,
    max_json_parse_length: Option<usize>,
    max_eval_steps: Option<usize>,
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

    /// Set max JSON input size for `parse_json`.
    pub fn with_max_json_parse_length(mut self, max_bytes: usize) -> Self {
        self.max_json_parse_length = Some(max_bytes);
        self
    }

    /// Set maximum evaluation steps before aborting.
    ///
    /// Each AST node evaluation counts as one step. When the limit is
    /// exceeded, the evaluator returns an `EvalError`. `None` means
    /// unlimited (the default).
    pub fn with_max_eval_steps(mut self, max: usize) -> Self {
        self.max_eval_steps = Some(max);
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

    /// Maximum evaluation steps. `None` means unlimited.
    pub fn max_eval_steps(&self) -> Option<usize> {
        self.max_eval_steps
    }
}

#[cfg(test)]
mod tests {
    use super::EvaluationPolicy;

    #[test]
    fn test_policy_builder_sets_fields() {
        let policy = EvaluationPolicy::new()
            .with_allowed_functions(["uppercase", "length"])
            .with_denied_functions(["length"])
            .with_strict_mode(true)
            .with_strict_conversion_functions(true)
            .with_strict_numeric_comparisons(true)
            .with_max_json_parse_length(2048);

        assert!(policy.allowed_functions().unwrap().contains("uppercase"));
        assert!(policy.denied_functions().contains("length"));
        assert!(policy.strict_mode());
        assert!(policy.strict_conversion_functions());
        assert!(policy.strict_numeric_comparisons());
        assert_eq!(policy.max_json_parse_length(), Some(2048));
    }
}
