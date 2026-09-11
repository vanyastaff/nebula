//! Evaluation context for expression execution
//!
//! This module provides the context in which expressions are evaluated,
//! including access to $node, $execution, $workflow, and $input variables.

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use chrono::Utc;
use serde_json::{Map, Value};

use crate::policy::EvaluationPolicy;

/// Evaluation context containing variables and workflow data.
///
/// All maps are wrapped in `Arc<HashMap<...>>` so cloning the context is
/// O(1) — important because higher-order builtins like `map`, `filter`,
/// and `reduce` clone the context once per iteration to scope a fresh
/// lambda binding. Single-entry mutations use copy-on-write, while batch
/// node population stages a complete replacement before publishing it.
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    /// Node data (`$node['name'].data`)
    nodes: Arc<HashMap<Arc<str>, Arc<Value>>>,
    /// Execution variables ($execution.id, $execution.mode, etc.)
    execution_vars: Arc<HashMap<Arc<str>, Arc<Value>>>,
    /// Lambda-bound parameters (isolated from execution_vars to avoid name collisions)
    lambda_vars: Arc<HashMap<Arc<str>, Arc<Value>>>,
    /// Workflow metadata ($workflow.id, $workflow.name, etc.)
    workflow: Arc<Value>,
    /// Input data ($input.item, $input.all, etc.)
    input: Arc<Value>,
    /// Optional per-context restrictions, intersected with the engine policy.
    policy: Option<Arc<EvaluationPolicy>>,
    /// Lazily materialized `$node` view, invalidated on mutation.
    nodes_view: Arc<OnceLock<Arc<Value>>>,
    /// Lazily materialized `$execution` view, invalidated on mutation.
    execution_view: Arc<OnceLock<Arc<Value>>>,
}

#[inline]
fn build_view(map: &HashMap<Arc<str>, Arc<Value>>) -> Arc<Value> {
    let mut obj = Map::with_capacity(map.len());
    for (key, value) in map {
        obj.insert(key.to_string(), (**value).clone());
    }
    Arc::new(Value::Object(obj))
}

#[inline]
fn empty_object_arc() -> Arc<Value> {
    Arc::new(Value::Object(Map::new()))
}

#[inline]
fn empty_view() -> Arc<OnceLock<Arc<Value>>> {
    Arc::new(OnceLock::new())
}

#[inline]
fn empty_map_arc() -> Arc<HashMap<Arc<str>, Arc<Value>>> {
    Arc::new(HashMap::new())
}

impl EvaluationContext {
    /// Create a new empty evaluation context
    pub fn new() -> Self {
        Self {
            nodes: empty_map_arc(),
            execution_vars: empty_map_arc(),
            lambda_vars: empty_map_arc(),
            workflow: empty_object_arc(),
            input: empty_object_arc(),
            policy: None,
            nodes_view: empty_view(),
            execution_view: empty_view(),
        }
    }

    /// Validate a JSON tree before placing it in shared immutable storage.
    ///
    /// # Errors
    /// Returns a resource-limit error before cloning when the value exceeds a
    /// fixed expression result ceiling.
    pub fn try_share_value(value: &Value) -> crate::ExpressionResult<Arc<Value>> {
        crate::limits::check_value_limits(value)?;
        Ok(Arc::new(value.clone()))
    }

    /// Validate a borrowed `$node` snapshot before sharing or cloning its trees.
    ///
    /// # Errors
    /// Returns a resource-limit error as soon as the aggregate object would
    /// exceed a fixed expression result ceiling.
    pub fn validate_node_data_snapshot<'a>(
        nodes: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> crate::ExpressionResult<()> {
        crate::limits::check_object_snapshot_limits(nodes)
    }

    /// Set data for a specific node
    pub fn set_node_data(&mut self, node_key: impl AsRef<str>, data: Value) {
        let key: Arc<str> = Arc::from(node_key.as_ref());
        Arc::make_mut(&mut self.nodes).insert(key, Arc::new(data));
        self.nodes_view = empty_view();
    }

    /// Insert multiple node outputs and publish one complete `$node` view.
    ///
    /// Existing context clones retain their previous immutable snapshot. The
    /// receiving context exposes all inserted entries together after this call
    /// returns, with one view materialization regardless of batch size.
    pub fn set_node_data_batch<K, I>(&mut self, nodes: I)
    where
        K: AsRef<str>,
        I: IntoIterator<Item = (K, Value)>,
    {
        let mut updated_nodes = (*self.nodes).clone();
        updated_nodes.extend(
            nodes
                .into_iter()
                .map(|(node_key, value)| (Arc::<str>::from(node_key.as_ref()), Arc::new(value))),
        );
        self.nodes = Arc::new(updated_nodes);
        self.nodes_view = empty_view();
    }

    /// Validate and publish shared immutable node outputs without cloning their trees.
    ///
    /// # Errors
    /// Returns a resource-limit error when one output or the aggregate `$node`
    /// object exceeds a fixed expression result ceiling.
    pub fn try_set_shared_node_data_batch<K, I>(&mut self, nodes: I) -> crate::ExpressionResult<()>
    where
        K: AsRef<str>,
        I: IntoIterator<Item = (K, Arc<Value>)>,
    {
        let mut updated_nodes = (*self.nodes).clone();
        for (node_key, value) in nodes {
            crate::limits::check_value_limits(&value)?;
            if !updated_nodes.contains_key(node_key.as_ref())
                && updated_nodes.len() >= crate::limits::MAX_RESULT_NODES.saturating_sub(1)
            {
                crate::limits::check_limit(
                    "result value nodes",
                    updated_nodes.len().saturating_add(2),
                    crate::limits::MAX_RESULT_NODES,
                )?;
            }
            updated_nodes.insert(Arc::from(node_key.as_ref()), value);
        }
        crate::limits::check_object_snapshot_limits(
            updated_nodes
                .iter()
                .map(|(key, value)| (key.as_ref(), value.as_ref())),
        )?;
        self.nodes = Arc::new(updated_nodes);
        self.nodes_view = empty_view();
        Ok(())
    }

    /// Get data for a specific node
    pub fn node_data(&self, node_key: &str) -> Option<Arc<Value>> {
        self.nodes.get(node_key).cloned()
    }

    /// Set an execution variable
    pub fn set_execution_var(&mut self, name: impl AsRef<str>, value: Value) {
        let key: Arc<str> = Arc::from(name.as_ref());
        Arc::make_mut(&mut self.execution_vars).insert(key, Arc::new(value));
        self.execution_view = empty_view();
    }

    /// Get an execution variable
    pub fn get_execution_var(&self, name: &str) -> Option<Arc<Value>> {
        self.execution_vars.get(name).cloned()
    }

    /// Set a lambda-bound parameter (used exclusively for lambda scopes to avoid
    /// collisions with real execution variables)
    pub fn set_lambda_var(&mut self, name: impl AsRef<str>, value: Value) {
        let key: Arc<str> = Arc::from(name.as_ref());
        Arc::make_mut(&mut self.lambda_vars).insert(key, Arc::new(value));
    }

    /// Get a lambda-bound parameter
    pub fn get_lambda_var(&self, name: &str) -> Option<Arc<Value>> {
        self.lambda_vars.get(name).cloned()
    }

    pub(crate) fn resolve_lambda_value(&self, name: &str) -> Option<&Value> {
        self.lambda_vars.get(name).map(AsRef::as_ref)
    }

    pub(crate) fn resolve_node_value(&self, node_key: &str) -> Option<&Value> {
        self.nodes.get(node_key).map(AsRef::as_ref)
    }

    pub(crate) fn resolve_execution_value(&self, name: &str) -> Option<&Value> {
        self.execution_vars.get(name).map(AsRef::as_ref)
    }

    /// Set the workflow metadata
    pub fn set_workflow(&mut self, workflow: Value) {
        self.workflow = Arc::new(workflow);
    }

    /// Get the workflow metadata
    pub fn get_workflow(&self) -> Arc<Value> {
        Arc::clone(&self.workflow)
    }

    /// Set the input data
    pub fn set_input(&mut self, input: Value) {
        self.input = Arc::new(input);
    }

    /// Get the input data
    pub fn get_input(&self) -> Arc<Value> {
        Arc::clone(&self.input)
    }

    /// Set additional restrictions for this context. Engine limits remain binding.
    pub fn set_policy(&mut self, policy: EvaluationPolicy) {
        self.policy = Some(Arc::new(policy));
    }

    /// Get the optional context restrictions.
    pub fn policy(&self) -> Option<&EvaluationPolicy> {
        self.policy.as_deref()
    }

    /// Resolve a variable through shared ownership.
    ///
    /// Stored variables return an O(1) [`Arc`] clone, including the cached
    /// `$node` and `$execution` views. Time-derived variables allocate one new
    /// scalar value for each lookup.
    ///
    /// # Errors
    /// Returns a resource-limit error before materializing an oversized
    /// aggregate `$node` view.
    pub fn resolve_variable(&self, name: &str) -> crate::ExpressionResult<Option<Arc<Value>>> {
        if let Some(value) = self.lambda_vars.get(name) {
            return Ok(Some(Arc::clone(value)));
        }
        if let Some(value) = self.execution_vars.get(name) {
            return Ok(Some(Arc::clone(value)));
        }

        Ok(match name {
            "node" => Some(Arc::clone(self.node_view()?)),
            "execution" => Some(Arc::clone(self.execution_view()?)),
            "workflow" => Some(Arc::clone(&self.workflow)),
            "input" => Some(Arc::clone(&self.input)),
            "now" => Some(Arc::new(Value::String(Utc::now().to_rfc3339()))),
            "today" => Some(Arc::new(Value::String(
                Utc::now().format("%Y-%m-%d").to_string(),
            ))),
            _ => None,
        })
    }

    /// Borrow stored data so the evaluator can check its budget before cloning.
    pub(crate) fn resolve_variable_value(
        &self,
        name: &str,
    ) -> crate::ExpressionResult<Option<Cow<'_, Value>>> {
        // Lambda-bound parameters take priority (e.g., `x` in `filter(arr, x => x > 2)`).
        if let Some(value) = self.lambda_vars.get(name) {
            return Ok(Some(Cow::Borrowed(value)));
        }

        // Custom execution variables set via `set_execution_var` (e.g., `$obj`).
        if let Some(value) = self.execution_vars.get(name) {
            return Ok(Some(Cow::Borrowed(value)));
        }

        Ok(match name {
            "node" => Some(Cow::Borrowed(self.node_view()?)),
            "execution" => Some(Cow::Borrowed(self.execution_view()?)),
            "workflow" => Some(Cow::Borrowed(&self.workflow)),
            "input" => Some(Cow::Borrowed(&self.input)),
            "now" => {
                let now = Utc::now();
                Some(Cow::Owned(Value::String(now.to_rfc3339())))
            },
            "today" => {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                Some(Cow::Owned(Value::String(today)))
            },
            _ => None,
        })
    }

    fn node_view(&self) -> crate::ExpressionResult<&Arc<Value>> {
        if let Some(view) = self.nodes_view.get() {
            return Ok(view);
        }
        crate::limits::check_object_snapshot_limits(
            self.nodes
                .iter()
                .map(|(key, value)| (key.as_ref(), value.as_ref())),
        )?;
        Ok(self.nodes_view.get_or_init(|| build_view(&self.nodes)))
    }

    fn execution_view(&self) -> crate::ExpressionResult<&Arc<Value>> {
        if let Some(view) = self.execution_view.get() {
            return Ok(view);
        }
        crate::limits::check_object_snapshot_limits(
            self.execution_vars
                .iter()
                .map(|(key, value)| (key.as_ref(), value.as_ref())),
        )?;
        Ok(self
            .execution_view
            .get_or_init(|| build_view(&self.execution_vars)))
    }

    /// Create a builder for constructing contexts
    pub fn builder() -> EvaluationContextBuilder {
        EvaluationContextBuilder::new()
    }
}

impl Default for EvaluationContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating evaluation contexts
#[derive(Debug, Clone, Default)]
pub struct EvaluationContextBuilder {
    nodes: HashMap<Arc<str>, Arc<Value>>,
    execution_vars: HashMap<Arc<str>, Arc<Value>>,
    workflow: Option<Arc<Value>>,
    input: Option<Arc<Value>>,
    policy: Option<Arc<EvaluationPolicy>>,
}

impl EvaluationContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add node data
    pub fn node(mut self, node_key: impl AsRef<str>, data: Value) -> Self {
        let key: Arc<str> = Arc::from(node_key.as_ref());
        self.nodes.insert(key, Arc::new(data));
        self
    }

    /// Add an execution variable
    pub fn execution_var(mut self, name: impl AsRef<str>, value: Value) -> Self {
        let key: Arc<str> = Arc::from(name.as_ref());
        self.execution_vars.insert(key, Arc::new(value));
        self
    }

    /// Set workflow metadata
    pub fn workflow(mut self, workflow: Value) -> Self {
        self.workflow = Some(Arc::new(workflow));
        self
    }

    /// Set input data
    pub fn input(mut self, input: Value) -> Self {
        self.input = Some(Arc::new(input));
        self
    }

    /// Set additional restrictions for contexts created by this builder.
    pub fn policy(mut self, policy: EvaluationPolicy) -> Self {
        self.policy = Some(Arc::new(policy));
        self
    }

    /// Build the evaluation context
    pub fn build(self) -> EvaluationContext {
        EvaluationContext {
            nodes: Arc::new(self.nodes),
            execution_vars: Arc::new(self.execution_vars),
            lambda_vars: empty_map_arc(),
            workflow: self.workflow.unwrap_or_else(empty_object_arc),
            input: self.input.unwrap_or_else(empty_object_arc),
            policy: self.policy,
            nodes_view: empty_view(),
            execution_view: empty_view(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = EvaluationContext::new();
        assert!(ctx.nodes.is_empty());
        assert!(ctx.execution_vars.is_empty());
    }

    #[test]
    fn test_set_and_get_node_data() {
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("node1", Value::String("test".to_string()));
        assert_eq!(ctx.node_data("node1").unwrap().as_str(), Some("test"));
    }

    #[test]
    fn batch_node_population_publishes_complete_view() {
        let mut context = EvaluationContext::new();
        context.set_node_data("existing", Value::Number(0.into()));
        let snapshot = context.clone();

        context.set_node_data_batch([
            ("first", Value::Number(1.into())),
            ("second", Value::Number(2.into())),
        ]);

        let populated = context.resolve_variable("node").unwrap().unwrap();
        assert_eq!(populated["existing"], Value::Number(0.into()));
        assert_eq!(populated["first"], Value::Number(1.into()));
        assert_eq!(populated["second"], Value::Number(2.into()));

        let prior_snapshot = snapshot.resolve_variable("node").unwrap().unwrap();
        assert_eq!(prior_snapshot.as_object().unwrap().len(), 1);
        assert_eq!(prior_snapshot["existing"], Value::Number(0.into()));
    }

    #[test]
    fn shared_node_population_preserves_arc_identity_and_stays_lazy() {
        let mut context = EvaluationContext::new();
        let output = Arc::new(serde_json::json!({"value": 7}));

        context
            .try_set_shared_node_data_batch([("source", Arc::clone(&output))])
            .unwrap();

        let stored = context.node_data("source").unwrap();
        assert!(Arc::ptr_eq(&stored, &output));
        assert!(context.nodes_view.get().is_none());
    }

    #[test]
    fn shared_node_population_rejects_depth_transactionally_without_cloning() {
        let mut context = EvaluationContext::new();
        context.set_node_data("existing", Value::Number(1.into()));
        let mut nested = Value::Null;
        for _ in 0..300 {
            nested = Value::Array(vec![nested]);
        }
        let nested = Arc::new(nested);

        let error = context
            .try_set_shared_node_data_batch([("deep", Arc::clone(&nested))])
            .unwrap_err();

        std::assert_matches!(error, crate::ExpressionError::ResourceLimitExceeded { .. });
        assert!(context.node_data("deep").is_none());
        assert_eq!(context.node_data("existing").unwrap().as_i64(), Some(1));
        assert_eq!(Arc::strong_count(&nested), 1);
    }

    #[test]
    fn resolving_oversized_node_view_fails_before_materialization() {
        let mut context = EvaluationContext::new();
        let half_limit = crate::limits::MAX_RESULT_BYTES / 2;
        context.set_node_data("first", Value::String("a".repeat(half_limit)));
        context.set_node_data("second", Value::String("b".repeat(half_limit)));

        let error = context.resolve_variable("node").unwrap_err();

        std::assert_matches!(
            error,
            crate::ExpressionError::ResourceLimitExceeded {
                resource: "result content bytes",
                ..
            }
        );
        assert!(context.nodes_view.get().is_none());
    }

    #[test]
    fn resolving_oversized_execution_view_fails_before_materialization() {
        let mut context = EvaluationContext::new();
        let half_limit = crate::limits::MAX_RESULT_BYTES / 2;
        context.set_execution_var("first", Value::String("a".repeat(half_limit)));
        context.set_execution_var("second", Value::String("b".repeat(half_limit)));

        let error = context.resolve_variable("execution").unwrap_err();

        std::assert_matches!(
            error,
            crate::ExpressionError::ResourceLimitExceeded {
                resource: "result content bytes",
                ..
            }
        );
        assert!(context.execution_view.get().is_none());
    }

    #[test]
    fn direct_execution_property_stays_borrowed_and_lazy() {
        let mut context = EvaluationContext::new();
        context.set_execution_var("large", Value::String("x".repeat(256 * 1024)));

        let engine = crate::ExpressionEngine::new().with_policy(
            EvaluationPolicy::new().with_max_eval_steps(
                crate::EvaluationStepLimit::new(crate::limits::MAX_RESULT_BYTES).unwrap(),
            ),
        );
        let result = engine.evaluate("$execution.large", &context).unwrap();

        assert_eq!(result.as_str().map(str::len), Some(256 * 1024));
        assert!(context.execution_view.get().is_none());
    }

    #[test]
    fn batch_node_population_rolls_back_when_source_panics() {
        let mut context = EvaluationContext::new();
        context.set_node_data("existing", Value::Number(0.into()));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            context.set_node_data_batch((0..3).map(|index| {
                assert_ne!(index, 1, "source iterator failed");
                (format!("node_{index}"), Value::Number(index.into()))
            }));
        }));

        assert!(result.is_err());
        assert!(context.node_data("node_0").is_none());
        let published = context.resolve_variable("node").unwrap().unwrap();
        assert_eq!(published.as_object().unwrap().len(), 1);
        assert_eq!(published["existing"], Value::Number(0.into()));
    }

    #[test]
    fn test_builder() {
        let ctx = EvaluationContext::builder()
            .node("node1", Value::String("test".to_string()))
            .execution_var("id", Value::String("exec-123".to_string()))
            .workflow(Value::String("workflow-1".to_string()))
            .input(Value::Number(42.into()))
            .build();

        assert_eq!(ctx.node_data("node1").unwrap().as_str(), Some("test"));
        assert_eq!(
            ctx.get_execution_var("id").unwrap().as_str(),
            Some("exec-123")
        );
    }

    #[test]
    fn test_policy_override_set_and_get() {
        let policy = EvaluationPolicy::allow_only(["uppercase"]);
        let mut ctx = EvaluationContext::new();
        ctx.set_policy(policy.clone());
        assert!(ctx.policy().is_some());
        assert!(
            ctx.policy()
                .unwrap()
                .allowed_functions()
                .unwrap()
                .contains("uppercase")
        );

        let ctx2 = EvaluationContext::builder().policy(policy).build();
        assert!(ctx2.policy().is_some());
    }

    #[test]
    fn test_resolve_variable() {
        let mut ctx = EvaluationContext::new();
        ctx.set_execution_var("id", Value::String("exec-123".to_string()));

        let exec = ctx.resolve_variable("execution").unwrap().unwrap();
        assert!(exec.is_object());
    }

    #[test]
    fn nodes_view_updates_on_set_node_data() {
        // Each `set_node_data` must rebuild the materialized `$node` view
        // so that subsequent `resolve_variable("node")` reflects the new key.
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("first", Value::Number(1.into()));
        let view1 = ctx.resolve_variable("node").unwrap().unwrap();
        let obj1 = view1.as_object().unwrap();
        assert_eq!(obj1.len(), 1);
        assert!(obj1.contains_key("first"));

        ctx.set_node_data("second", Value::Number(2.into()));
        let view2 = ctx.resolve_variable("node").unwrap().unwrap();
        let obj2 = view2.as_object().unwrap();
        assert_eq!(obj2.len(), 2);
        assert!(obj2.contains_key("first"));
        assert!(obj2.contains_key("second"));
    }

    #[test]
    fn execution_view_updates_on_set_execution_var() {
        let mut ctx = EvaluationContext::new();
        ctx.set_execution_var("id", Value::String("e1".into()));
        let initial = ctx.resolve_variable("execution").unwrap().unwrap();
        assert_eq!(initial.as_object().unwrap().len(), 1);

        ctx.set_execution_var("mode", Value::String("test".into()));
        assert!(ctx.execution_view.get().is_none());

        let view = ctx.resolve_variable("execution").unwrap().unwrap();
        let obj = view.as_object().unwrap();
        assert_eq!(obj.get("id").and_then(|v| v.as_str()), Some("e1"));
        assert_eq!(obj.get("mode").and_then(|v| v.as_str()), Some("test"));
    }

    #[test]
    fn builder_initializes_views() {
        // Builder path must produce the same lazy views as the imperative API.
        let ctx = EvaluationContext::builder()
            .node("a", Value::Number(1.into()))
            .execution_var("id", Value::String("x".into()))
            .build();

        assert!(ctx.nodes_view.get().is_none());
        assert!(ctx.execution_view.get().is_none());

        let node_view = ctx.resolve_variable("node").unwrap().unwrap();
        assert_eq!(node_view.as_object().unwrap().len(), 1);

        let exec_view = ctx.resolve_variable("execution").unwrap().unwrap();
        assert!(exec_view.as_object().unwrap().contains_key("id"));
    }

    #[test]
    fn repeated_resolve_returns_consistent_data() {
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("k", Value::String("x".repeat(256 * 1024)));
        let first = ctx.resolve_variable("node").unwrap().unwrap();

        for _ in 0..10 {
            let view = ctx.resolve_variable("node").unwrap().unwrap();
            assert!(Arc::ptr_eq(&first, &view));
        }
    }

    #[test]
    fn clone_preserves_view_content() {
        // `EvaluationContext::Clone` is invoked per lambda iteration; the
        // cached view must clone with the rest of the struct, not get
        // dropped or reset to empty.
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("k", Value::Number(7.into()));
        let cloned = ctx.clone();
        let view = cloned.resolve_variable("node").unwrap().unwrap();
        assert_eq!(view.as_object().unwrap().len(), 1);
    }
}
