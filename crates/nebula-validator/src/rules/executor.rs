//! Rule executor for running rules

use super::{Rule, RuleContext, RuleResult, RuleId};
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

/// Rule execution strategy
#[derive(Debug, Clone)]
pub enum ExecutionStrategy {
    /// Execute rules sequentially
    Sequential,
    /// Execute rules in parallel
    Parallel,
    /// Execute rules in parallel with limited concurrency
    ParallelLimited(usize),
    /// Execute rules by priority
    PriorityBased,
    /// Custom execution strategy
    Custom(Box<dyn ExecutionStrategyTrait>),
}

/// Trait for custom execution strategies
pub trait ExecutionStrategyTrait: Send + Sync {
    /// Execute rules with this strategy
    fn execute(
        &self,
        rules: &[&dyn Rule],
        value: &Value,
        context: &RuleContext,
    ) -> impl std::future::Future<Output = Vec<RuleResult>> + Send;
}

/// Execution mode
#[derive(Debug, Clone)]
pub enum ExecutionMode {
    /// Continue executing all rules
    ContinueAll,
    /// Stop on first failure
    StopOnFirstFailure,
    /// Stop on first success
    StopOnFirstSuccess,
}

/// Rule executor
#[derive(Debug)]
pub struct RuleExecutor {
    strategy: ExecutionStrategy,
    mode: ExecutionMode,
    timeout: Option<Duration>,
}

impl RuleExecutor {
    /// Create a new executor
    pub fn new(strategy: ExecutionStrategy) -> Self {
        Self {
            strategy,
            mode: ExecutionMode::ContinueAll,
            timeout: None,
        }
    }
    
    /// Set execution mode
    pub fn with_mode(mut self, mode: ExecutionMode) -> Self {
        self.mode = mode;
        self
    }
    
    /// Set timeout for rule execution
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    
    /// Execute multiple rules
    pub async fn execute_rules(
        &self,
        rules: &[&dyn Rule],
        value: &Value,
        context: &RuleContext,
    ) -> RuleResult {
        if rules.is_empty() {
            return RuleResult::pass(RuleId::new("no_rules"));
        }
        
        let results = match &self.strategy {
            ExecutionStrategy::Sequential => {
                self.execute_sequential(rules, value, context).await
            },
            ExecutionStrategy::Parallel => {
                self.execute_parallel(rules, value, context, None).await
            },
            ExecutionStrategy::ParallelLimited(limit) => {
                self.execute_parallel(rules, value, context, Some(*limit)).await
            },
            ExecutionStrategy::PriorityBased => {
                self.execute_priority(rules, value, context).await
            },
            ExecutionStrategy::Custom(strategy) => {
                strategy.execute(rules, value, context).await
            },
        };
        
        // Combine results
        RuleResult::combine(results)
    }
    
    /// Execute a single rule
    pub async fn execute_single(
        &self,
        rule: &dyn Rule,
        value: &Value,
        context: &RuleContext,
    ) -> RuleResult {
        // Check if rule applies
        if !rule.applies_to(value, context).await {
            return RuleResult::skip(rule.id().clone(), "Rule does not apply");
        }
        
        // Validate context
        if let Err(msg) = rule.validate_context(context) {
            return RuleResult::error(rule.id().clone(), msg);
        }
        
        // Execute with timeout if configured
        if let Some(timeout_duration) = self.timeout {
            match timeout(timeout_duration, rule.apply(value, context)).await {
                Ok(result) => result,
                Err(_) => RuleResult::error(rule.id().clone(), "Rule execution timed out"),
            }
        } else {
            rule.apply(value, context).await
        }
    }
    
    /// Execute rules sequentially
    async fn execute_sequential(
        &self,
        rules: &[&dyn Rule],
        value: &Value,
        context: &RuleContext,
    ) -> Vec<RuleResult> {
        let mut results = Vec::new();
        
        for rule in rules {
            let result = self.execute_single(*rule, value, context).await;
            let should_stop = match self.mode {
                ExecutionMode::StopOnFirstFailure => !result.passed(),
                ExecutionMode::StopOnFirstSuccess => result.passed(),
                ExecutionMode::ContinueAll => false,
            };
            
            results.push(result);
            
            if should_stop {
                break;
            }
        }
        
        results
    }
    
    /// Execute rules in parallel
    async fn execute_parallel(
        &self,
        rules: &[&dyn Rule],
        value: &Value,
        context: &RuleContext,
        max_concurrency: Option<usize>,
    ) -> Vec<RuleResult> {
        use futures::stream::{self, StreamExt};
        
        let futures = rules.iter().map(|rule| {
            self.execute_single(*rule, value, context)
        });
        
        let stream = stream::iter(futures);
        
        let results: Vec<_> = if let Some(limit) = max_concurrency {
            stream.buffer_unordered(limit).collect().await
        } else {
            stream.buffer_unordered(rules.len()).collect().await
        };
        
        results
    }
    
    /// Execute rules by priority
    async fn execute_priority(
        &self,
        rules: &[&dyn Rule],
        value: &Value,
        context: &RuleContext,
    ) -> Vec<RuleResult> {
        // Sort rules by priority
        let mut sorted_rules = rules.to_vec();
        sorted_rules.sort_by_key(|r| std::cmp::Reverse(r.priority()));
        
        self.execute_sequential(&sorted_rules, value, context).await
    }
}

/// Default executor
impl Default for RuleExecutor {
    fn default() -> Self {
        Self::new(ExecutionStrategy::Sequential)
    }
}