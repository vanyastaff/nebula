//! Rule trait and implementations

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use super::{RuleContext, RuleResult, RuleOutcome};
use std::fmt::{self, Debug, Display};

/// Unique identifier for a rule
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleId(String);

impl RuleId {
    /// Create a new rule ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Get as string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Rule priority (higher values = higher priority)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RulePriority(i32);

impl RulePriority {
    /// Highest priority
    pub const HIGHEST: Self = Self(i32::MAX);
    /// High priority
    pub const HIGH: Self = Self(1000);
    /// Normal priority
    pub const NORMAL: Self = Self(0);
    /// Low priority
    pub const LOW: Self = Self(-1000);
    /// Lowest priority
    pub const LOWEST: Self = Self(i32::MIN);
    
    /// Create custom priority
    pub fn new(value: i32) -> Self {
        Self(value)
    }
    
    /// Get priority value
    pub fn value(&self) -> i32 {
        self.0
    }
}

impl Default for RulePriority {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Metadata for a rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleMetadata {
    /// Rule name
    pub name: String,
    /// Rule description
    pub description: Option<String>,
    /// Rule group
    pub group: Option<String>,
    /// Rule tags
    pub tags: Vec<String>,
    /// Rule version
    pub version: String,
    /// Rule author
    pub author: Option<String>,
    /// Creation date
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last modified date
    pub modified_at: chrono::DateTime<chrono::Utc>,
    /// Whether the rule is experimental
    pub experimental: bool,
    /// Custom metadata
    pub custom: std::collections::HashMap<String, Value>,
}

impl RuleMetadata {
    /// Create new metadata
    pub fn new(name: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        Self {
            name: name.into(),
            description: None,
            group: None,
            tags: Vec::new(),
            version: "1.0.0".to_string(),
            author: None,
            created_at: now,
            modified_at: now,
            experimental: false,
            custom: std::collections::HashMap::new(),
        }
    }
    
    /// Set description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
    
    /// Set group
    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
    
    /// Add tag
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
    
    /// Set version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }
}

/// Core trait for validation rules
#[async_trait]
pub trait Rule: Send + Sync + Debug {
    /// Get rule ID
    fn id(&self) -> &RuleId;
    
    /// Get rule priority
    fn priority(&self) -> RulePriority {
        RulePriority::NORMAL
    }
    
    /// Get rule metadata
    fn metadata(&self) -> &RuleMetadata;
    
    /// Check if the rule is enabled
    fn is_enabled(&self) -> bool {
        true
    }
    
    /// Apply the rule to a value
    async fn apply(&self, value: &Value, context: &RuleContext) -> RuleResult;
    
    /// Check if the rule applies to a value (pre-condition)
    async fn applies_to(&self, value: &Value, context: &RuleContext) -> bool {
        true
    }
    
    /// Get required context keys
    fn required_context_keys(&self) -> Vec<String> {
        Vec::new()
    }
    
    /// Validate that required context is present
    fn validate_context(&self, context: &RuleContext) -> Result<(), String> {
        for key in self.required_context_keys() {
            if !context.contains_key(&key) {
                return Err(format!("Missing required context key: {}", key));
            }
        }
        Ok(())
    }
    
    /// Get dependencies on other rules
    fn dependencies(&self) -> Vec<RuleId> {
        Vec::new()
    }
    
    /// Get rules that this rule conflicts with
    fn conflicts_with(&self) -> Vec<RuleId> {
        Vec::new()
    }
}

/// Simple rule implementation
#[derive(Debug)]
pub struct SimpleRule<F> {
    id: RuleId,
    metadata: RuleMetadata,
    priority: RulePriority,
    apply_fn: F,
}

impl<F> SimpleRule<F>
where
    F: Fn(&Value, &RuleContext) -> RuleOutcome + Send + Sync,
{
    /// Create a new simple rule
    pub fn new(id: impl Into<String>, apply_fn: F) -> Self {
        let id = RuleId::new(id);
        Self {
            metadata: RuleMetadata::new(id.as_str()),
            id,
            priority: RulePriority::NORMAL,
            apply_fn,
        }
    }
    
    /// Set priority
    pub fn with_priority(mut self, priority: RulePriority) -> Self {
        self.priority = priority;
        self
    }
    
    /// Set metadata
    pub fn with_metadata(mut self, metadata: RuleMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

#[async_trait]
impl<F> Rule for SimpleRule<F>
where
    F: Fn(&Value, &RuleContext) -> RuleOutcome + Send + Sync,
{
    fn id(&self) -> &RuleId {
        &self.id
    }
    
    fn priority(&self) -> RulePriority {
        self.priority
    }
    
    fn metadata(&self) -> &RuleMetadata {
        &self.metadata
    }
    
    async fn apply(&self, value: &Value, context: &RuleContext) -> RuleResult {
        let outcome = (self.apply_fn)(value, context);
        RuleResult::from_outcome(self.id.clone(), outcome)
    }
}

/// Async rule implementation
#[derive(Debug)]
pub struct AsyncRule<F, Fut> {
    id: RuleId,
    metadata: RuleMetadata,
    priority: RulePriority,
    apply_fn: F,
    _phantom: std::marker::PhantomData<Fut>,
}

impl<F, Fut> AsyncRule<F, Fut>
where
    F: Fn(&Value, &RuleContext) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = RuleOutcome> + Send,
{
    /// Create a new async rule
    pub fn new(id: impl Into<String>, apply_fn: F) -> Self {
        let id = RuleId::new(id);
        Self {
            metadata: RuleMetadata::new(id.as_str()),
            id,
            priority: RulePriority::NORMAL,
            apply_fn,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<F, Fut> Rule for AsyncRule<F, Fut>
where
    F: Fn(&Value, &RuleContext) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = RuleOutcome> + Send,
{
    fn id(&self) -> &RuleId {
        &self.id
    }
    
    fn priority(&self) -> RulePriority {
        self.priority
    }
    
    fn metadata(&self) -> &RuleMetadata {
        &self.metadata
    }
    
    async fn apply(&self, value: &Value, context: &RuleContext) -> RuleResult {
        let outcome = (self.apply_fn)(value, context).await;
        RuleResult::from_outcome(self.id.clone(), outcome)
    }
}

/// Composite rule that combines multiple rules
#[derive(Debug)]
pub struct CompositeRule {
    id: RuleId,
    metadata: RuleMetadata,
    rules: Vec<Box<dyn Rule>>,
    combination: CombinationMode,
}

#[derive(Debug, Clone)]
pub enum CombinationMode {
    /// All rules must pass
    All,
    /// At least one rule must pass
    Any,
    /// Exactly one rule must pass
    One,
    /// Custom combination logic
    Custom(fn(&[RuleResult]) -> RuleResult),
}

impl CompositeRule {
    /// Create a new composite rule
    pub fn new(id: impl Into<String>, combination: CombinationMode) -> Self {
        let id = RuleId::new(id);
        Self {
            metadata: RuleMetadata::new(id.as_str()),
            id,
            rules: Vec::new(),
            combination,
        }
    }
    
    /// Add a rule
    pub fn add_rule<R: Rule + 'static>(mut self, rule: R) -> Self {
        self.rules.push(Box::new(rule));
        self
    }
}

#[async_trait]
impl Rule for CompositeRule {
    fn id(&self) -> &RuleId {
        &self.id
    }
    
    fn metadata(&self) -> &RuleMetadata {
        &self.metadata
    }
    
    async fn apply(&self, value: &Value, context: &RuleContext) -> RuleResult {
        let mut results = Vec::new();
        
        for rule in &self.rules {
            results.push(rule.apply(value, context).await);
        }
        
        match self.combination {
            CombinationMode::All => {
                let all_pass = results.iter().all(|r| r.passed());
                if all_pass {
                    RuleResult::pass(self.id.clone())
                } else {
                    RuleResult::fail(self.id.clone(), "Not all rules passed")
                }
            },
            CombinationMode::Any => {
                let any_pass = results.iter().any(|r| r.passed());
                if any_pass {
                    RuleResult::pass(self.id.clone())
                } else {
                    RuleResult::fail(self.id.clone(), "No rules passed")
                }
            },
            CombinationMode::One => {
                let pass_count = results.iter().filter(|r| r.passed()).count();
                if pass_count == 1 {
                    RuleResult::pass(self.id.clone())
                } else {
                    RuleResult::fail(self.id.clone(), 
                        format!("Expected exactly 1 rule to pass, got {}", pass_count))
                }
            },
            CombinationMode::Custom(combiner) => {
                combiner(&results)
            },
        }
    }
}