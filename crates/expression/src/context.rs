//! Evaluation context for expression execution
//!
//! This module provides the context in which expressions are evaluated,
//! including access to $node, $execution, $workflow, and $input variables.
//!
//! Setters accept plain `serde_json::Value` — the crate boundary — and convert
//! once into [`RuntimeValue`]. Everything inside evaluation works on runtime
//! values, so typed values such as date-times survive inside stored containers.

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use chrono::Utc;

use crate::{policy::EvaluationPolicy, value::RuntimeValue};

/// Evaluation context containing variables and workflow data.
///
/// All maps are wrapped in `Arc<HashMap<...>>` so cloning the context is
/// O(1) — important because higher-order builtins like `map`, `filter`,
/// and `reduce` clone the context once per iteration to scope a fresh
/// lambda binding. Single-entry mutations use copy-on-write, while batch
/// node population stages a complete replacement before publishing it.
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    /// Node data (`$node['name'].json`)
    nodes: Arc<HashMap<Arc<str>, Arc<RuntimeValue>>>,
    /// Execution variables ($execution.id, $execution.mode, etc.)
    execution_vars: Arc<HashMap<Arc<str>, Arc<RuntimeValue>>>,
    /// Lambda-bound parameters (isolated from execution_vars to avoid name collisions)
    lambda_vars: Arc<HashMap<Arc<str>, Arc<RuntimeValue>>>,
    /// Workflow metadata ($workflow.id, $workflow.name, etc.)
    workflow: Arc<RuntimeValue>,
    /// Input data ($input.item, $input.all, etc.)
    input: Arc<RuntimeValue>,
    /// Optional per-context restrictions, intersected with the engine policy.
    policy: Option<Arc<EvaluationPolicy>>,
    /// Lazily materialized `$node` view, invalidated on mutation.
    nodes_view: Arc<OnceLock<Arc<RuntimeValue>>>,
    /// Lazily materialized `$execution` view, invalidated on mutation.
    execution_view: Arc<OnceLock<Arc<RuntimeValue>>>,
}

#[inline]
fn build_view(map: &HashMap<Arc<str>, Arc<RuntimeValue>>) -> Arc<RuntimeValue> {
    let mut object = std::collections::BTreeMap::new();
    for (key, value) in map {
        object.insert(Arc::clone(key), (**value).clone());
    }
    Arc::new(RuntimeValue::Object(Arc::new(object)))
}

#[inline]
fn empty_object_arc() -> Arc<RuntimeValue> {
    Arc::new(RuntimeValue::Object(Arc::new(
        std::collections::BTreeMap::new(),
    )))
}

#[inline]
fn empty_view() -> Arc<OnceLock<Arc<RuntimeValue>>> {
    Arc::new(OnceLock::new())
}

#[inline]
fn empty_map_arc() -> Arc<HashMap<Arc<str>, Arc<RuntimeValue>>> {
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

    /// Validate a JSON tree and convert it to a shared runtime value.
    ///
    /// # Errors
    /// Returns a resource-limit error before cloning when the value exceeds a
    /// fixed expression result ceiling.
    pub fn try_share_value(
        value: &serde_json::Value,
    ) -> crate::ExpressionResult<Arc<RuntimeValue>> {
        let converted = RuntimeValue::from_json(value);
        crate::limits::check_value_limits(&converted)?;
        Ok(Arc::new(converted))
    }

    /// Validate a borrowed `$node` snapshot before sharing or cloning its trees.
    ///
    /// # Errors
    /// Returns a resource-limit error as soon as the aggregate object would
    /// exceed a fixed expression result ceiling.
    pub fn validate_node_data_snapshot<'a>(
        nodes: impl IntoIterator<Item = (&'a str, &'a serde_json::Value)>,
    ) -> crate::ExpressionResult<()> {
        let converted = nodes
            .into_iter()
            .map(|(key, value)| (key, RuntimeValue::from_json(value)))
            .collect::<Vec<_>>();
        crate::limits::check_object_snapshot_limits(
            converted.iter().map(|(key, value)| (*key, value)),
        )
    }

    /// Set data for a specific node
    pub fn set_node_data(&mut self, node_key: impl AsRef<str>, data: serde_json::Value) {
        let key: Arc<str> = Arc::from(node_key.as_ref());
        Arc::make_mut(&mut self.nodes).insert(key, Arc::new(RuntimeValue::from_json(&data)));
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
        I: IntoIterator<Item = (K, serde_json::Value)>,
    {
        let mut updated_nodes = (*self.nodes).clone();
        updated_nodes.extend(nodes.into_iter().map(|(node_key, value)| {
            (
                Arc::<str>::from(node_key.as_ref()),
                Arc::new(RuntimeValue::from_json(&value)),
            )
        }));
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
        I: IntoIterator<Item = (K, Arc<RuntimeValue>)>,
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
    pub fn node_data(&self, node_key: &str) -> Option<Arc<RuntimeValue>> {
        self.nodes.get(node_key).cloned()
    }

    /// Set an execution variable
    pub fn set_execution_var(&mut self, name: impl AsRef<str>, value: serde_json::Value) {
        let key: Arc<str> = Arc::from(name.as_ref());
        Arc::make_mut(&mut self.execution_vars)
            .insert(key, Arc::new(RuntimeValue::from_json(&value)));
        self.execution_view = empty_view();
    }

    /// Get an execution variable
    pub fn get_execution_var(&self, name: &str) -> Option<Arc<RuntimeValue>> {
        self.execution_vars.get(name).cloned()
    }

    /// Set a lambda-bound parameter (used exclusively for lambda scopes to avoid
    /// collisions with real execution variables)
    pub fn set_lambda_var(&mut self, name: impl AsRef<str>, value: RuntimeValue) {
        let key: Arc<str> = Arc::from(name.as_ref());
        Arc::make_mut(&mut self.lambda_vars).insert(key, Arc::new(value));
    }

    /// Get a lambda-bound parameter
    pub fn get_lambda_var(&self, name: &str) -> Option<Arc<RuntimeValue>> {
        self.lambda_vars.get(name).cloned()
    }

    pub(crate) fn resolve_lambda_value(&self, name: &str) -> Option<&RuntimeValue> {
        self.lambda_vars.get(name).map(AsRef::as_ref)
    }

    pub(crate) fn resolve_node_value(&self, node_key: &str) -> Option<&RuntimeValue> {
        self.nodes.get(node_key).map(AsRef::as_ref)
    }

    pub(crate) fn resolve_execution_value(&self, name: &str) -> Option<&RuntimeValue> {
        self.execution_vars.get(name).map(AsRef::as_ref)
    }

    /// Set the workflow metadata
    pub fn set_workflow(&mut self, workflow: serde_json::Value) {
        self.workflow = Arc::new(RuntimeValue::from_json(&workflow));
    }

    /// Get the workflow metadata
    pub fn get_workflow(&self) -> Arc<RuntimeValue> {
        Arc::clone(&self.workflow)
    }

    /// Set the input data
    pub fn set_input(&mut self, input: serde_json::Value) {
        self.input = Arc::new(RuntimeValue::from_json(&input));
    }

    /// Get the input data
    pub fn get_input(&self) -> Arc<RuntimeValue> {
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
    /// [`Self::resolve_variable`] and [`Self::resolve_variable_value`] share
    /// this name table; keep them in sync.
    ///
    /// # Errors
    /// Returns a resource-limit error before materializing an oversized
    /// aggregate `$node` view.
    pub fn resolve_variable(
        &self,
        name: &str,
    ) -> crate::ExpressionResult<Option<Arc<RuntimeValue>>> {
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
            // `$json` is the n8n spelling of the current item's data. This
            // crate resolves one item at a time, so it is exactly `$input`.
            "input" | "json" => Some(Arc::clone(&self.input)),
            "now" => Some(Arc::new(RuntimeValue::date_time_utc(Utc::now()))),
            "today" => Some(Arc::new(RuntimeValue::date_time_utc(
                Utc::now()
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .map(|naive| chrono::DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
                    .unwrap_or_else(Utc::now),
            ))),
            _ => None,
        })
    }

    /// Borrow stored data so the evaluator can check its budget before cloning.
    ///
    /// Name table shared with [`Self::resolve_variable`]; keep them in sync.
    pub(crate) fn resolve_variable_value(
        &self,
        name: &str,
    ) -> crate::ExpressionResult<Option<Cow<'_, RuntimeValue>>> {
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
            // See `resolve_variable`: `$json` aliases the current item.
            "input" | "json" => Some(Cow::Borrowed(&self.input)),
            "now" => Some(Cow::Owned(RuntimeValue::date_time_utc(Utc::now()))),
            "today" => Some(Cow::Owned(RuntimeValue::date_time_utc(
                Utc::now()
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .map(|naive| chrono::DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
                    .unwrap_or_else(Utc::now),
            ))),
            _ => None,
        })
    }

    fn node_view(&self) -> crate::ExpressionResult<&Arc<RuntimeValue>> {
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

    fn execution_view(&self) -> crate::ExpressionResult<&Arc<RuntimeValue>> {
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
    nodes: HashMap<Arc<str>, Arc<RuntimeValue>>,
    execution_vars: HashMap<Arc<str>, Arc<RuntimeValue>>,
    workflow: Option<Arc<RuntimeValue>>,
    input: Option<Arc<RuntimeValue>>,
    policy: Option<Arc<EvaluationPolicy>>,
}

impl EvaluationContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add node data
    pub fn node(mut self, node_key: impl AsRef<str>, data: serde_json::Value) -> Self {
        let key: Arc<str> = Arc::from(node_key.as_ref());
        self.nodes
            .insert(key, Arc::new(RuntimeValue::from_json(&data)));
        self
    }

    /// Add an execution variable
    pub fn execution_var(mut self, name: impl AsRef<str>, value: serde_json::Value) -> Self {
        let key: Arc<str> = Arc::from(name.as_ref());
        self.execution_vars
            .insert(key, Arc::new(RuntimeValue::from_json(&value)));
        self
    }

    /// Set workflow metadata
    pub fn workflow(mut self, workflow: serde_json::Value) -> Self {
        self.workflow = Some(Arc::new(RuntimeValue::from_json(&workflow)));
        self
    }

    /// Set input data
    pub fn input(mut self, input: serde_json::Value) -> Self {
        self.input = Some(Arc::new(RuntimeValue::from_json(&input)));
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
    use serde_json::Value;

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
        assert_eq!(populated.to_json()["existing"], Value::Number(0.into()));
        assert_eq!(populated.to_json()["first"], Value::Number(1.into()));
        assert_eq!(populated.to_json()["second"], Value::Number(2.into()));

        let prior_snapshot = snapshot.resolve_variable("node").unwrap().unwrap();
        assert_eq!(prior_snapshot.to_json().as_object().unwrap().len(), 1);
        assert_eq!(
            prior_snapshot.to_json()["existing"],
            Value::Number(0.into())
        );
    }

    #[test]
    fn shared_node_population_preserves_arc_identity_and_stays_lazy() {
        let mut context = EvaluationContext::new();
        let output = Arc::new(RuntimeValue::object(Default::default()));

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
        let mut nested = RuntimeValue::Null;
        for _ in 0..300 {
            nested = RuntimeValue::array(vec![nested]);
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
        assert_eq!(published.to_json().as_object().unwrap().len(), 1);
        assert_eq!(published.to_json()["existing"], Value::Number(0.into()));
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
        assert!(exec.as_object().is_some());
    }

    #[test]
    fn nodes_view_updates_on_set_node_data() {
        // Each `set_node_data` must rebuild the materialized `$node` view
        // so that subsequent `resolve_variable("node")` reflects the new key.
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("first", Value::Number(1.into()));
        let view1 = ctx.resolve_variable("node").unwrap().unwrap();
        assert_eq!(view1.as_object().unwrap().len(), 1);
        assert!(view1.as_object().unwrap().contains_key("first"));

        ctx.set_node_data("second", Value::Number(2.into()));
        let view2 = ctx.resolve_variable("node").unwrap().unwrap();
        assert_eq!(view2.as_object().unwrap().len(), 2);
        assert!(view2.as_object().unwrap().contains_key("first"));
        assert!(view2.as_object().unwrap().contains_key("second"));
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
        let object = view.to_json();
        assert_eq!(
            object.get("id").and_then(|value| value.as_str()),
            Some("e1")
        );
        assert_eq!(
            object.get("mode").and_then(|value| value.as_str()),
            Some("test")
        );
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

    #[test]
    fn now_and_today_resolve_as_date_time_values() {
        let context = EvaluationContext::new();
        let now = context.resolve_variable("now").unwrap().unwrap();
        assert!(now.as_date_time().is_some(), "`$now` must be a date value");
        let today = context.resolve_variable("today").unwrap().unwrap();
        let today = today.as_date_time().expect("`$today` must be a date value");
        assert_eq!(today.format("%H:%M:%S").to_string(), "00:00:00");
    }

    #[test]
    fn json_and_input_resolve_to_the_same_shared_value() {
        // `$json` is the n8n spelling of the current item and must not be a
        // second copy that can drift from `$input`.
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("item".into()));

        let from_input = context.resolve_variable("input").unwrap().unwrap();
        let from_json = context.resolve_variable("json").unwrap().unwrap();
        assert!(Arc::ptr_eq(&from_input, &from_json));

        let borrowed = context.resolve_variable_value("json").unwrap().unwrap();
        assert_eq!(borrowed.as_ref(), from_input.as_ref());
    }
}
