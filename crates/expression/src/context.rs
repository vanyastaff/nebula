//! Evaluation context for expression execution
//!
//! This module provides the context in which expressions are evaluated,
//! including access to $node, $execution, $workflow, and $input variables.

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use serde_json::{Map, Value};

use crate::policy::EvaluationPolicy;

/// Evaluation context containing variables and workflow data
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    /// Node data ($node['name'].data)
    nodes: HashMap<Arc<str>, Arc<Value>>,
    /// Execution variables ($execution.id, $execution.mode, etc.)
    execution_vars: HashMap<Arc<str>, Arc<Value>>,
    /// Lambda-bound parameters (isolated from execution_vars to avoid name collisions)
    lambda_vars: HashMap<Arc<str>, Arc<Value>>,
    /// Workflow metadata ($workflow.id, $workflow.name, etc.)
    workflow: Arc<Value>,
    /// Input data ($input.item, $input.all, etc.)
    input: Arc<Value>,
    /// Optional per-context evaluation policy override.
    policy: Option<Arc<EvaluationPolicy>>,
    /// Pre-materialized `$node` view, rebuilt only on mutation.
    ///
    /// `resolve_variable("node")` was rebuilding a fresh `Map` from `nodes`
    /// on every call, which made `{{ $node.a + $node.b + $node.c }}` cost
    /// O(N×M) with N nodes and M references. Caching here trades a small
    /// allocation per *mutation* for O(1) access on the read hot path.
    nodes_view: Arc<Value>,
    /// Pre-materialized `$execution` view (same rationale as `nodes_view`).
    execution_view: Arc<Value>,
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

impl EvaluationContext {
    /// Create a new empty evaluation context
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            execution_vars: HashMap::new(),
            lambda_vars: HashMap::new(),
            workflow: empty_object_arc(),
            input: empty_object_arc(),
            policy: None,
            nodes_view: empty_object_arc(),
            execution_view: empty_object_arc(),
        }
    }

    /// Set data for a specific node
    pub fn set_node_data(&mut self, node_key: impl AsRef<str>, data: Value) {
        let key: Arc<str> = Arc::from(node_key.as_ref());
        self.nodes.insert(key, Arc::new(data));
        self.nodes_view = build_view(&self.nodes);
    }

    /// Get data for a specific node
    pub fn node_data(&self, node_key: &str) -> Option<Arc<Value>> {
        self.nodes.get(node_key).cloned()
    }

    /// Set an execution variable
    pub fn set_execution_var(&mut self, name: impl AsRef<str>, value: Value) {
        let key: Arc<str> = Arc::from(name.as_ref());
        self.execution_vars.insert(key, Arc::new(value));
        self.execution_view = build_view(&self.execution_vars);
    }

    /// Get an execution variable
    pub fn get_execution_var(&self, name: &str) -> Option<Arc<Value>> {
        self.execution_vars.get(name).cloned()
    }

    /// Set a lambda-bound parameter (used exclusively for lambda scopes to avoid
    /// collisions with real execution variables)
    pub fn set_lambda_var(&mut self, name: impl AsRef<str>, value: Value) {
        let key: Arc<str> = Arc::from(name.as_ref());
        self.lambda_vars.insert(key, Arc::new(value));
    }

    /// Get a lambda-bound parameter
    pub fn get_lambda_var(&self, name: &str) -> Option<Arc<Value>> {
        self.lambda_vars.get(name).cloned()
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

    /// Set an optional policy override for this context.
    pub fn set_policy(&mut self, policy: EvaluationPolicy) {
        self.policy = Some(Arc::new(policy));
    }

    /// Get the optional policy override.
    pub fn policy(&self) -> Option<&EvaluationPolicy> {
        self.policy.as_deref()
    }

    /// Resolve a variable by name.
    ///
    /// `$node` and `$execution` are served from pre-materialized views (see
    /// `nodes_view` / `execution_view`); the cost of a single resolve is one
    /// `Value` clone of an already-built object, not a full HashMap rebuild.
    pub fn resolve_variable(&self, name: &str) -> Option<Value> {
        // Lambda-bound parameters take priority (e.g., `x` in `filter(arr, x => x > 2)`).
        if let Some(value) = self.lambda_vars.get(name) {
            return Some((**value).clone());
        }

        // Custom execution variables set via `set_execution_var` (e.g., `$obj`).
        if let Some(value) = self.execution_vars.get(name) {
            return Some((**value).clone());
        }

        match name {
            "node" => Some((*self.nodes_view).clone()),
            "execution" => Some((*self.execution_view).clone()),
            "workflow" => Some((*self.workflow).clone()),
            "input" => Some((*self.input).clone()),
            "now" => {
                let now = Utc::now();
                Some(Value::String(now.to_rfc3339()))
            },
            "today" => {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                Some(Value::String(today))
            },
            _ => None,
        }
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

    /// Set a policy override for contexts created by this builder.
    pub fn policy(mut self, policy: EvaluationPolicy) -> Self {
        self.policy = Some(Arc::new(policy));
        self
    }

    /// Build the evaluation context
    pub fn build(self) -> EvaluationContext {
        let nodes_view = build_view(&self.nodes);
        let execution_view = build_view(&self.execution_vars);
        EvaluationContext {
            nodes: self.nodes,
            execution_vars: self.execution_vars,
            lambda_vars: HashMap::new(),
            workflow: self.workflow.unwrap_or_else(empty_object_arc),
            input: self.input.unwrap_or_else(empty_object_arc),
            policy: self.policy,
            nodes_view,
            execution_view,
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

        let exec = ctx.resolve_variable("execution").unwrap();
        assert!(exec.is_object());
    }

    #[test]
    fn nodes_view_updates_on_set_node_data() {
        // Each `set_node_data` must rebuild the materialized `$node` view
        // so that subsequent `resolve_variable("node")` reflects the new key.
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("first", Value::Number(1.into()));
        let view1 = ctx.resolve_variable("node").unwrap();
        let obj1 = view1.as_object().unwrap();
        assert_eq!(obj1.len(), 1);
        assert!(obj1.contains_key("first"));

        ctx.set_node_data("second", Value::Number(2.into()));
        let view2 = ctx.resolve_variable("node").unwrap();
        let obj2 = view2.as_object().unwrap();
        assert_eq!(obj2.len(), 2);
        assert!(obj2.contains_key("first"));
        assert!(obj2.contains_key("second"));
    }

    #[test]
    fn execution_view_updates_on_set_execution_var() {
        let mut ctx = EvaluationContext::new();
        ctx.set_execution_var("id", Value::String("e1".into()));
        ctx.set_execution_var("mode", Value::String("test".into()));

        let view = ctx.resolve_variable("execution").unwrap();
        let obj = view.as_object().unwrap();
        assert_eq!(obj.get("id").and_then(|v| v.as_str()), Some("e1"));
        assert_eq!(obj.get("mode").and_then(|v| v.as_str()), Some("test"));
    }

    #[test]
    fn builder_initializes_views() {
        // Builder path must produce the same materialized views as the
        // imperative API, otherwise `EvaluationContextBuilder::build`
        // contexts would silently see empty `$node`/`$execution`.
        let ctx = EvaluationContext::builder()
            .node("a", Value::Number(1.into()))
            .execution_var("id", Value::String("x".into()))
            .build();

        let node_view = ctx.resolve_variable("node").unwrap();
        assert_eq!(node_view.as_object().unwrap().len(), 1);

        let exec_view = ctx.resolve_variable("execution").unwrap();
        assert!(exec_view.as_object().unwrap().contains_key("id"));
    }

    #[test]
    fn repeated_resolve_returns_consistent_data() {
        // Hot-path regression: ten resolves of `$node` must all return the
        // same materialized object — caching must not produce stale views.
        let mut ctx = EvaluationContext::new();
        ctx.set_node_data("k", Value::Number(7.into()));

        for _ in 0..10 {
            let view = ctx.resolve_variable("node").unwrap();
            assert_eq!(
                view.as_object().unwrap().get("k").and_then(Value::as_i64),
                Some(7)
            );
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
        let view = cloned.resolve_variable("node").unwrap();
        assert_eq!(view.as_object().unwrap().len(), 1);
    }
}
