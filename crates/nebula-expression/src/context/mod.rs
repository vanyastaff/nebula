//! Evaluation context for expression execution
//!
//! This module provides the context in which expressions are evaluated,
//! including access to $node, $execution, $workflow, and $input variables.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Evaluation context containing variables and workflow data
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    /// Node data ($node['name'].data)
    nodes: HashMap<Arc<str>, Arc<Value>>,
    /// Execution variables ($execution.id, $execution.mode, etc.)
    execution_vars: HashMap<Arc<str>, Arc<Value>>,
    /// Workflow metadata ($workflow.id, $workflow.name, etc.)
    workflow: Arc<Value>,
    /// Input data ($input.item, $input.all, etc.)
    input: Arc<Value>,
}

impl EvaluationContext {
    /// Create a new empty evaluation context
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            execution_vars: HashMap::new(),
            workflow: Arc::new(Value::Object(serde_json::Map::new())),
            input: Arc::new(Value::Object(serde_json::Map::new())),
        }
    }

    /// Set data for a specific node
    pub fn set_node_data(&mut self, node_id: impl Into<String>, data: Value) {
        let key: Arc<str> = Arc::from(node_id.into().as_str());
        self.nodes.insert(key, Arc::new(data));
    }

    /// Get data for a specific node
    pub fn get_node_data(&self, node_id: &str) -> Option<Arc<Value>> {
        self.nodes.get(node_id).cloned()
    }

    /// Set an execution variable
    pub fn set_execution_var(&mut self, name: impl Into<String>, value: Value) {
        let key: Arc<str> = Arc::from(name.into().as_str());
        self.execution_vars.insert(key, Arc::new(value));
    }

    /// Get an execution variable
    pub fn get_execution_var(&self, name: &str) -> Option<Arc<Value>> {
        self.execution_vars.get(name).cloned()
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

    /// Resolve a variable by name
    pub fn resolve_variable(&self, name: &str) -> Option<Value> {
        // First, check for local variables (e.g., lambda parameters)
        if let Some(value) = self.execution_vars.get(name) {
            return Some((**value).clone());
        }

        match name {
            "node" => {
                // Return an object containing all nodes
                let mut obj = serde_json::Map::new();
                for (key, value) in &self.nodes {
                    obj.insert(key.to_string(), (**value).clone());
                }
                Some(Value::Object(obj))
            }
            "execution" => {
                // Return an object containing all execution variables
                let mut obj = serde_json::Map::new();
                for (key, value) in &self.execution_vars {
                    obj.insert(key.to_string(), (**value).clone());
                }
                Some(Value::Object(obj))
            }
            "workflow" => Some((*self.workflow).clone()),
            "input" => Some((*self.input).clone()),
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
}

impl EvaluationContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add node data
    pub fn node(mut self, node_id: impl Into<String>, data: Value) -> Self {
        let key: Arc<str> = Arc::from(node_id.into().as_str());
        self.nodes.insert(key, Arc::new(data));
        self
    }

    /// Add an execution variable
    pub fn execution_var(mut self, name: impl Into<String>, value: Value) -> Self {
        let key: Arc<str> = Arc::from(name.into().as_str());
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

    /// Build the evaluation context
    pub fn build(self) -> EvaluationContext {
        EvaluationContext {
            nodes: self.nodes,
            execution_vars: self.execution_vars,
            workflow: self
                .workflow
                .unwrap_or_else(|| Arc::new(Value::Object(serde_json::Map::new()))),
            input: self
                .input
                .unwrap_or_else(|| Arc::new(Value::Object(serde_json::Map::new()))),
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
        assert_eq!(ctx.get_node_data("node1").unwrap().as_str(), Some("test"));
    }

    #[test]
    fn test_builder() {
        let ctx = EvaluationContext::builder()
            .node("node1", Value::String("test".to_string()))
            .execution_var("id", Value::String("exec-123".to_string()))
            .workflow(Value::String("workflow-1".to_string()))
            .input(Value::Number(42.into()))
            .build();

        assert_eq!(ctx.get_node_data("node1").unwrap().as_str(), Some("test"));
        assert_eq!(
            ctx.get_execution_var("id").unwrap().as_str(),
            Some("exec-123")
        );
    }

    #[test]
    fn test_resolve_variable() {
        let mut ctx = EvaluationContext::new();
        ctx.set_execution_var("id", Value::String("exec-123".to_string()));

        let exec = ctx.resolve_variable("execution").unwrap();
        assert!(exec.is_object());
    }
}
