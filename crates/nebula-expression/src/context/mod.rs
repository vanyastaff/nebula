//! Evaluation context for expression execution
//!
//! This module provides the context in which expressions are evaluated,
//! including access to $node, $execution, $workflow, and $input variables.

use nebula_value::ValueRefExt;
use nebula_value::Value;
use std::collections::HashMap;

/// Evaluation context containing variables and workflow data
#[derive(Debug, Clone)]
pub struct EvaluationContext {
    /// Node data ($node['name'].data)
    nodes: HashMap<String, Value>,
    /// Execution variables ($execution.id, $execution.mode, etc.)
    execution_vars: HashMap<String, Value>,
    /// Workflow metadata ($workflow.id, $workflow.name, etc.)
    workflow: Value,
    /// Input data ($input.item, $input.all, etc.)
    input: Value,
}

impl EvaluationContext {
    /// Create a new empty evaluation context
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            execution_vars: HashMap::new(),
            workflow: Value::object_empty(),
            input: Value::object_empty(),
        }
    }

    /// Set data for a specific node
    pub fn set_node_data(&mut self, node_id: impl Into<String>, data: Value) {
        self.nodes.insert(node_id.into(), data);
    }

    /// Get data for a specific node
    pub fn get_node_data(&self, node_id: &str) -> Option<&Value> {
        self.nodes.get(node_id)
    }

    /// Set an execution variable
    pub fn set_execution_var(&mut self, name: impl Into<String>, value: Value) {
        self.execution_vars.insert(name.into(), value);
    }

    /// Get an execution variable
    pub fn get_execution_var(&self, name: &str) -> Option<&Value> {
        self.execution_vars.get(name)
    }

    /// Set the workflow metadata
    pub fn set_workflow(&mut self, workflow: Value) {
        self.workflow = workflow;
    }

    /// Get the workflow metadata
    pub fn get_workflow(&self) -> &Value {
        &self.workflow
    }

    /// Set the input data
    pub fn set_input(&mut self, input: Value) {
        self.input = input;
    }

    /// Get the input data
    pub fn get_input(&self) -> &Value {
        &self.input
    }

    /// Resolve a variable by name
    pub fn resolve_variable(&self, name: &str) -> Option<Value> {
        match name {
            "node" => {
                // Return an object containing all nodes
                let mut obj = nebula_value::Object::new();
                for (key, value) in &self.nodes {
                    obj = obj.insert(key.clone(), value.to_json());
                }
                Some(Value::Object(obj))
            }
            "execution" => {
                // Return an object containing all execution variables
                let mut obj = nebula_value::Object::new();
                for (key, value) in &self.execution_vars {
                    obj = obj.insert(key.clone(), value.to_json());
                }
                Some(Value::Object(obj))
            }
            "workflow" => Some(self.workflow.clone()),
            "input" => Some(self.input.clone()),
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
    nodes: HashMap<String, Value>,
    execution_vars: HashMap<String, Value>,
    workflow: Option<Value>,
    input: Option<Value>,
}

impl EvaluationContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add node data
    pub fn node(mut self, node_id: impl Into<String>, data: Value) -> Self {
        self.nodes.insert(node_id.into(), data);
        self
    }

    /// Add an execution variable
    pub fn execution_var(mut self, name: impl Into<String>, value: Value) -> Self {
        self.execution_vars.insert(name.into(), value);
        self
    }

    /// Set workflow metadata
    pub fn workflow(mut self, workflow: Value) -> Self {
        self.workflow = Some(workflow);
        self
    }

    /// Set input data
    pub fn input(mut self, input: Value) -> Self {
        self.input = Some(input);
        self
    }

    /// Build the evaluation context
    pub fn build(self) -> EvaluationContext {
        EvaluationContext {
            nodes: self.nodes,
            execution_vars: self.execution_vars,
            workflow: self.workflow.unwrap_or_else(Value::object_empty),
            input: self.input.unwrap_or_else(Value::object_empty),
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
        ctx.set_node_data("node1", Value::text("test"));
        assert_eq!(
            ctx.get_node_data("node1").unwrap().as_str(),
            Some("test")
        );
    }

    #[test]
    fn test_builder() {
        let ctx = EvaluationContext::builder()
            .node("node1", Value::text("test"))
            .execution_var("id", Value::text("exec-123"))
            .workflow(Value::text("workflow-1"))
            .input(Value::integer(42))
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
        ctx.set_execution_var("id", Value::text("exec-123"));

        let exec = ctx.resolve_variable("execution").unwrap();
        assert!(exec.is_object());
    }
}
