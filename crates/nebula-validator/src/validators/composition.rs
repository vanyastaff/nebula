//! Rule composition system for nebula-validator
//! 
//! This module provides advanced rule composition capabilities including
//! dependency management, topological sorting, and complex rule chains.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

use crate::traits::Validatable;
use crate::types::{ValidationResult, ValidationError, ValidatorMetadata, ValidationComplexity, ErrorCode};
use crate::context::ValidationContext;

// ==================== Composed Rule ====================

/// Composed rule with metadata and dependencies
pub struct ComposedRule {
    id: String,
    validator: Box<dyn Validatable>,
    dependencies: Vec<String>,
    priority: i32,
    cache_result: bool,
}

impl ComposedRule {
    /// Create new composed rule
    pub fn new(
        id: impl Into<String>,
        validator: Box<dyn Validatable>,
    ) -> Self {
        Self {
            id: id.into(),
            validator,
            dependencies: Vec::new(),
            priority: 0,
            cache_result: false,
        }
    }
    
    /// Add dependency
    pub fn depends_on(mut self, dependency: impl Into<String>) -> Self {
        self.dependencies.push(dependency.into());
        self
    }
    
    /// Set priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
    
    /// Enable result caching
    pub fn cache_result(mut self) -> Self {
        self.cache_result = true;
        self
    }
}

// ==================== Rule Composer ====================

/// Builder for composing complex rules
pub struct RuleComposer {
    rules: Vec<ComposedRule>,
    context: ValidationContext,
}

impl RuleComposer {
    /// Create new rule composer
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            context: ValidationContext::new(),
        }
    }
    
    /// Add rule
    pub fn rule<V: Validatable + 'static>(
        mut self,
        id: impl Into<String>,
        validator: V,
    ) -> Self {
        self.rules.push(ComposedRule::new(
            id,
            Box::new(validator),
        ));
        self
    }
    
    /// Add rule with dependencies
    pub fn dependent_rule<V: Validatable + 'static>(
        mut self,
        id: impl Into<String>,
        validator: V,
        dependencies: Vec<String>,
    ) -> Self {
        let mut rule = ComposedRule::new(id, Box::new(validator));
        rule.dependencies = dependencies;
        self.rules.push(rule);
        self
    }
    
    /// Add rule with full configuration
    pub fn configured_rule(
        mut self,
        rule: ComposedRule,
    ) -> Self {
        self.rules.push(rule);
        self
    }
    
    /// Create dependency graph and execute validation
    pub async fn validate(&self, value: &Value) -> ValidationResult<()> {
        let execution_order = self.topological_sort()?;
        let mut results = HashMap::new();
        let mut errors = Vec::new();
        
        for rule_id in execution_order {
            let rule = self.rules.iter()
                .find(|r| r.id == rule_id)
                .unwrap();
            
            // Check dependencies
            let deps_satisfied = rule.dependencies.iter()
                .all(|dep| results.get(dep).map_or(false, |r: &bool| *r));
            
            if !deps_satisfied {
                continue; // Skip if dependencies not satisfied
            }
            
            match rule.validator.validate(value).await {
                Ok(_) => {
                    results.insert(rule.id.clone(), true);
                },
                Err(e) => {
                    results.insert(rule.id.clone(), false);
                    errors.push(e);
                }
            }
        }
        
        if errors.is_empty() {
            ValidationResult::success(())
        } else {
            ValidationResult::failure(errors)
        }
    }
    
    /// Topological sort to determine execution order
    fn topological_sort(&self) -> Result<Vec<String>, ValidationError> {
        // Implementation of Kahn's algorithm
        let mut in_degree = HashMap::new();
        let mut graph = HashMap::new();
        
        // Build graph
        for rule in &self.rules {
            in_degree.entry(rule.id.clone()).or_insert(0);
            graph.entry(rule.id.clone()).or_insert_with(Vec::new);
            
            for dep in &rule.dependencies {
                graph.entry(dep.clone())
                    .or_insert_with(Vec::new)
                    .push(rule.id.clone());
                *in_degree.entry(rule.id.clone()).or_insert(0) += 1;
            }
        }
        
        // Find nodes with no incoming edges
        let mut queue = VecDeque::new();
        for (node, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(node.clone());
            }
        }
        
        let mut result = Vec::new();
        while let Some(node) = queue.pop_front() {
            result.push(node.clone());
            
            if let Some(neighbors) = graph.get(&node) {
                for neighbor in neighbors {
                    let degree = in_degree.get_mut(neighbor).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }
        
        if result.len() != self.rules.len() {
            Err(ValidationError::new(
                ErrorCode::new("circular_dependency"),
                "Circular dependency detected in validation rules"
            ))
        } else {
            Ok(result)
        }
    }
    
    /// Get rule by ID
    pub fn get_rule(&self, id: &str) -> Option<&ComposedRule> {
        self.rules.iter().find(|r| r.id == id)
    }
    
    /// Get all rules
    pub fn rules(&self) -> &[ComposedRule] {
        &self.rules
    }
    
    /// Check if rule exists
    pub fn has_rule(&self, id: &str) -> bool {
        self.rules.iter().any(|r| r.id == id)
    }
    
    /// Remove rule
    pub fn remove_rule(&mut self, id: &str) -> bool {
        if let Some(pos) = self.rules.iter().position(|r| r.id == id) {
            self.rules.remove(pos);
            true
        } else {
            false
        }
    }
    
    /// Clear all rules
    pub fn clear(&mut self) {
        self.rules.clear();
    }
    
    /// Get rule count
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

impl Default for RuleComposer {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Rule Chain ====================

/// Chain of rules for sequential execution
pub struct RuleChain {
    rules: Vec<ComposedRule>,
    stop_on_error: bool,
    collect_all_errors: bool,
}

impl RuleChain {
    /// Create new rule chain
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            stop_on_error: true,
            collect_all_errors: false,
        }
    }
    
    /// Add rule to chain
    pub fn add_rule(mut self, rule: ComposedRule) -> Self {
        self.rules.push(rule);
        self
    }
    
    /// Add rule with builder pattern
    pub fn rule<V: Validatable + 'static>(
        mut self,
        id: impl Into<String>,
        validator: V,
    ) -> Self {
        self.rules.push(ComposedRule::new(
            id,
            Box::new(validator),
        ));
        self
    }
    
    /// Continue on error
    pub fn continue_on_error(mut self) -> Self {
        self.stop_on_error = false;
        self
    }
    
    /// Collect all errors
    pub fn collect_all_errors(mut self) -> Self {
        self.collect_all_errors = true;
        self
    }
    
    /// Execute chain
    pub async fn execute(&self, value: &Value) -> ValidationResult<()> {
        let mut errors = Vec::new();
        
        for rule in &self.rules {
            match rule.validator.validate(value).await {
                Ok(_) => {},
                Err(e) => {
                    if self.collect_all_errors {
                        errors.extend(e);
                    } else {
                        errors = e;
                    }
                    
                    if self.stop_on_error {
                        break;
                    }
                }
            }
        }
        
        if errors.is_empty() {
            ValidationResult::success(())
        } else {
            ValidationResult::failure(errors)
        }
    }
}

impl Default for RuleChain {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Rule Group ====================

/// Group of related rules
pub struct RuleGroup {
    name: String,
    rules: Vec<ComposedRule>,
    shared_context: ValidationContext,
}

impl RuleGroup {
    /// Create new rule group
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rules: Vec::new(),
            shared_context: ValidationContext::new(),
        }
    }
    
    /// Add rule to group
    pub fn add_rule(mut self, rule: ComposedRule) -> Self {
        self.rules.push(rule);
        self
    }
    
    /// Add rule with builder pattern
    pub fn rule<V: Validatable + 'static>(
        mut self,
        id: impl Into<String>,
        validator: V,
    ) -> Self {
        self.rules.push(ComposedRule::new(
            id,
            Box::new(validator),
        ));
        self
    }
    
    /// Execute all rules in group
    pub async fn execute(&self, value: &Value) -> ValidationResult<()> {
        let mut errors = Vec::new();
        
        for rule in &self.rules {
            match rule.validator.validate(value).await {
                Ok(_) => {},
                Err(e) => errors.extend(e),
            }
        }
        
        if errors.is_empty() {
            ValidationResult::success(())
        } else {
            ValidationResult::failure(errors)
        }
    }
    
    /// Get group name
    pub fn name(&self) -> &str {
        &self.name
    }
    
    /// Get rule count
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

// ==================== Builder Functions ====================

/// Create new rule composer
pub fn rule_composer() -> RuleComposer {
    RuleComposer::new()
}

/// Create new rule chain
pub fn rule_chain() -> RuleChain {
    RuleChain::new()
}

/// Create new rule group
pub fn rule_group(name: impl Into<String>) -> RuleGroup {
    RuleGroup::new(name)
}

/// Create composed rule
pub fn composed_rule(
    id: impl Into<String>,
    validator: impl Validatable + 'static,
) -> ComposedRule {
    ComposedRule::new(id, Box::new(validator))
}
