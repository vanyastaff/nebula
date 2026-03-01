//! Policy controls for expression evaluation.
//!
//! Policies can constrain which builtin functions are callable and carry
//! compatibility flags such as strict mode.

use std::collections::HashSet;
use std::sync::Arc;

/// Evaluation policy applied by the engine and optionally overridden by context.
#[derive(Debug, Clone, Default)]
pub struct EvaluationPolicy {
    allowed_functions: Option<Arc<HashSet<String>>>,
    denied_functions: Arc<HashSet<String>>,
    strict_mode: bool,
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
}

#[cfg(test)]
mod tests {
    use super::EvaluationPolicy;

    #[test]
    fn test_policy_builder_sets_fields() {
        let policy = EvaluationPolicy::new()
            .with_allowed_functions(["uppercase", "length"])
            .with_denied_functions(["length"])
            .with_strict_mode(true);

        assert!(policy.allowed_functions().unwrap().contains("uppercase"));
        assert!(policy.denied_functions().contains("length"));
        assert!(policy.strict_mode());
    }
}

