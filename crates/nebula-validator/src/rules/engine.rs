//! Rule engine implementation

use super::{Rule, RuleContext, RuleExecutor, RuleResult, RuleId, ExecutionStrategy};
use crate::types::{ValidationResult, ValidationError};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use async_trait::async_trait;

/// Rule engine for managing and executing validation rules
#[derive(Debug)]
pub struct RuleEngine {
    /// Registered rules
    rules: Arc<RwLock<HashMap<RuleId, Box<dyn Rule>>>>,
    /// Rule groups for organization
    groups: Arc<RwLock<HashMap<String, Vec<RuleId>>>>,
    /// Disabled rules
    disabled_rules: Arc<RwLock<HashSet<RuleId>>>,
    /// Engine configuration
    config: EngineConfig,
    /// Rule executor
    executor: RuleExecutor,
    /// Global context
    global_context: Arc<RwLock<RuleContext>>,
}

impl RuleEngine {
    /// Create a new rule engine
    pub fn new(config: EngineConfig) -> Self {
        Self {
            rules: Arc::new(RwLock::new(HashMap::new())),
            groups: Arc::new(RwLock::new(HashMap::new())),
            disabled_rules: Arc::new(RwLock::new(HashSet::new())),
            executor: RuleExecutor::new(config.execution_strategy.clone()),
            config,
            global_context: Arc::new(RwLock::new(RuleContext::new())),
        }
    }
    
    /// Create a builder for the engine
    pub fn builder() -> RuleEngineBuilder {
        RuleEngineBuilder::new()
    }
    
    /// Register a new rule
    pub async fn register_rule<R>(&self, rule: R) -> Result<(), EngineError>
    where
        R: Rule + 'static,
    {
        let rule_id = rule.id().clone();
        let mut rules = self.rules.write().await;
        
        if rules.contains_key(&rule_id) && !self.config.allow_overwrite {
            return Err(EngineError::DuplicateRule(rule_id));
        }
        
        rules.insert(rule_id.clone(), Box::new(rule));
        
        // Update groups if specified
        if let Some(group) = rule.metadata().group.as_ref() {
            let mut groups = self.groups.write().await;
            groups.entry(group.clone())
                .or_insert_with(Vec::new)
                .push(rule_id);
        }
        
        Ok(())
    }
    
    /// Unregister a rule
    pub async fn unregister_rule(&self, rule_id: &RuleId) -> Result<(), EngineError> {
        let mut rules = self.rules.write().await;
        
        if rules.remove(rule_id).is_none() {
            return Err(EngineError::RuleNotFound(rule_id.clone()));
        }
        
        // Remove from groups
        let mut groups = self.groups.write().await;
        for group_rules in groups.values_mut() {
            group_rules.retain(|id| id != rule_id);
        }
        
        // Remove from disabled set
        let mut disabled = self.disabled_rules.write().await;
        disabled.remove(rule_id);
        
        Ok(())
    }
    
    /// Enable a rule
    pub async fn enable_rule(&self, rule_id: &RuleId) -> Result<(), EngineError> {
        let rules = self.rules.read().await;
        if !rules.contains_key(rule_id) {
            return Err(EngineError::RuleNotFound(rule_id.clone()));
        }
        
        let mut disabled = self.disabled_rules.write().await;
        disabled.remove(rule_id);
        Ok(())
    }
    
    /// Disable a rule
    pub async fn disable_rule(&self, rule_id: &RuleId) -> Result<(), EngineError> {
        let rules = self.rules.read().await;
        if !rules.contains_key(rule_id) {
            return Err(EngineError::RuleNotFound(rule_id.clone()));
        }
        
        let mut disabled = self.disabled_rules.write().await;
        disabled.insert(rule_id.clone());
        Ok(())
    }
    
    /// Execute all enabled rules
    pub async fn execute(&self, value: &Value) -> RuleResult {
        let context = self.global_context.read().await.clone();
        self.execute_with_context(value, context).await
    }
    
    /// Execute with custom context
    pub async fn execute_with_context(
        &self,
        value: &Value,
        mut context: RuleContext,
    ) -> RuleResult {
        let rules = self.rules.read().await;
        let disabled = self.disabled_rules.read().await;
        
        // Filter enabled rules
        let enabled_rules: Vec<_> = rules
            .iter()
            .filter(|(id, _)| !disabled.contains(id))
            .map(|(_, rule)| rule.as_ref())
            .collect();
        
        // Add engine metadata to context
        context.set("engine.rule_count", enabled_rules.len() as i64);
        context.set("engine.execution_time", chrono::Utc::now().to_rfc3339());
        
        self.executor.execute_rules(&enabled_rules, value, &context).await
    }
    
    /// Execute rules by group
    pub async fn execute_group(
        &self,
        group: &str,
        value: &Value,
    ) -> Result<RuleResult, EngineError> {
        let groups = self.groups.read().await;
        let rule_ids = groups.get(group)
            .ok_or_else(|| EngineError::GroupNotFound(group.to_string()))?;
        
        let rules = self.rules.read().await;
        let disabled = self.disabled_rules.read().await;
        let context = self.global_context.read().await.clone();
        
        let group_rules: Vec<_> = rule_ids
            .iter()
            .filter(|id| !disabled.contains(id))
            .filter_map(|id| rules.get(id))
            .map(|rule| rule.as_ref())
            .collect();
        
        self.executor.execute_rules(&group_rules, value, &context).await
    }
    
    /// Execute a single rule
    pub async fn execute_rule(
        &self,
        rule_id: &RuleId,
        value: &Value,
    ) -> Result<RuleResult, EngineError> {
        let rules = self.rules.read().await;
        let rule = rules.get(rule_id)
            .ok_or_else(|| EngineError::RuleNotFound(rule_id.clone()))?;
        
        let context = self.global_context.read().await.clone();
        self.executor.execute_single(rule.as_ref(), value, &context).await
    }
    
    /// Get all registered rules
    pub async fn list_rules(&self) -> Vec<RuleId> {
        self.rules.read().await.keys().cloned().collect()
    }
    
    /// Get rules by group
    pub async fn list_group_rules(&self, group: &str) -> Option<Vec<RuleId>> {
        self.groups.read().await.get(group).cloned()
    }
    
    /// Get rule metadata
    pub async fn get_rule_metadata(&self, rule_id: &RuleId) -> Option<RuleMetadata> {
        self.rules.read().await
            .get(rule_id)
            .map(|rule| rule.metadata().clone())
    }
    
    /// Update global context
    pub async fn set_context_value(&self, key: impl Into<String>, value: impl Into<ContextValue>) {
        let mut context = self.global_context.write().await;
        context.set(key, value);
    }
    
    /// Clear all rules
    pub async fn clear(&self) {
        let mut rules = self.rules.write().await;
        let mut groups = self.groups.write().await;
        let mut disabled = self.disabled_rules.write().await;
        
        rules.clear();
        groups.clear();
        disabled.clear();
    }
}

/// Engine configuration
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Execution strategy
    pub execution_strategy: ExecutionStrategy,
    /// Whether to allow rule overwriting
    pub allow_overwrite: bool,
    /// Whether to stop on first failure
    pub stop_on_first_failure: bool,
    /// Whether to collect all violations
    pub collect_all_violations: bool,
    /// Maximum concurrent rule executions
    pub max_concurrency: usize,
    /// Rule execution timeout
    pub rule_timeout: std::time::Duration,
    /// Whether to cache rule results
    pub enable_caching: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            execution_strategy: ExecutionStrategy::Sequential,
            allow_overwrite: false,
            stop_on_first_failure: false,
            collect_all_violations: true,
            max_concurrency: 10,
            rule_timeout: std::time::Duration::from_secs(5),
            enable_caching: true,
        }
    }
}

/// Builder for RuleEngine
pub struct RuleEngineBuilder {
    config: EngineConfig,
    initial_rules: Vec<Box<dyn Rule>>,
}

impl RuleEngineBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: EngineConfig::default(),
            initial_rules: Vec::new(),
        }
    }
    
    /// Set execution strategy
    pub fn execution_strategy(mut self, strategy: ExecutionStrategy) -> Self {
        self.config.execution_strategy = strategy;
        self
    }
    
    /// Allow rule overwriting
    pub fn allow_overwrite(mut self, allow: bool) -> Self {
        self.config.allow_overwrite = allow;
        self
    }
    
    /// Stop on first failure
    pub fn stop_on_first_failure(mut self, stop: bool) -> Self {
        self.config.stop_on_first_failure = stop;
        self
    }
    
    /// Set max concurrency
    pub fn max_concurrency(mut self, max: usize) -> Self {
        self.config.max_concurrency = max;
        self
    }
    
    /// Add initial rule
    pub fn with_rule<R: Rule + 'static>(mut self, rule: R) -> Self {
        self.initial_rules.push(Box::new(rule));
        self
    }
    
    /// Build the engine
    pub async fn build(self) -> RuleEngine {
        let engine = RuleEngine::new(self.config);
        
        // Register initial rules
        for rule in self.initial_rules {
            let _ = engine.register_rule(rule).await;
        }
        
        engine
    }
}

impl Default for RuleEngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Engine errors
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Duplicate rule: {0}")]
    DuplicateRule(RuleId),
    
    #[error("Rule not found: {0}")]
    RuleNotFound(RuleId),
    
    #[error("Group not found: {0}")]
    GroupNotFound(String),
    
    #[error("Execution error: {0}")]
    ExecutionError(String),
    
    #[error("Context error: {0}")]
    ContextError(String),
}

use super::RuleMetadata;
use super::ContextValue;