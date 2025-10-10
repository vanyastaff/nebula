//! Display system for conditional parameter visibility
//!
//! This module provides an idiomatic Rust API for controlling when parameters
//! should be displayed based on the state of other parameters. It leverages
//! `nebula-validator` for all validation logic, treating display conditions
//! as cross-field validations.
//!
//! # Architecture
//!
//! - `DisplayContext` - Contains resolved values of all parameters
//! - `DisplayRule` - A validator that checks parameter state
//! - `DisplayRuleSet` - Composition of rules (AND/OR/NOT)
//! - `ParameterDisplay` - Configuration for when to show/hide a parameter
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//! use nebula_value::Value;
//!
//! // Create context from array
//! let ctx = DisplayContext::from([
//!     ("auth_type", Value::text("api_key")),
//!     ("level", Value::integer(15)),
//! ]);
//!
//! // Build display rules with operators
//! let display = ParameterDisplay::builder()
//!     .show_when(
//!         DisplayRule::when_equals("auth_type", Value::text("api_key"))
//!         & DisplayRule::when_greater_than("level", 10.0)
//!     )
//!     .build();
//!
//! assert!(display.should_display_sync(&ctx));
//! ```

use nebula_core::ParameterKey;
// TODO: Update display system to work with new nebula-validator API
// use nebula_validator::core::{AsyncValidator, ValidationContext};
use nebula_value::Value;
use std::collections::HashMap;
use std::ops::{BitAnd, BitOr, Not};

// ============================================================================
// DisplayContext - holds resolved parameter values
// ============================================================================

/// Context containing resolved values of all parameters for display evaluation
///
/// This is the input to display rule evaluation. All parameter values should
/// be resolved (expressions evaluated) before being placed in this context.
///
/// # Examples
///
/// ```rust
/// # use nebula_parameter::DisplayContext;
/// # use nebula_value::Value;
/// // Multiple ways to create
/// let ctx = DisplayContext::new()
///     .with_value("auth_type", Value::text("api_key"))
///     .with_value("level", Value::integer(15));
///
/// // From array (const generic)
/// let ctx: DisplayContext = [
///     ("auth_type", Value::text("api_key")),
///     ("level", Value::integer(15)),
/// ].into();
///
/// // Index access
/// let auth = &ctx["auth_type"];
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayContext {
    values: HashMap<ParameterKey, Value>,
}

impl DisplayContext {
    /// Create a new empty context
    #[inline]
    pub const fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Get a parameter value by key
    #[inline]
    pub fn get(&self, key: &ParameterKey) -> Option<&Value> {
        self.values.get(key)
    }

    /// Set a parameter value
    #[inline]
    pub fn set(&mut self, key: ParameterKey, value: Value) {
        self.values.insert(key, value);
    }

    /// Insert a parameter value (returns old value if present)
    #[inline]
    pub fn insert(&mut self, key: impl Into<ParameterKey>, value: impl Into<Value>) -> Option<Value> {
        self.values.insert(key.into(), value.into())
    }

    /// Remove a parameter value
    #[inline]
    pub fn remove(&mut self, key: &ParameterKey) -> Option<Value> {
        self.values.remove(key)
    }

    /// Check if a parameter exists
    #[inline]
    pub fn contains_key(&self, key: &ParameterKey) -> bool {
        self.values.contains_key(key)
    }

    /// Get the number of parameters
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if context is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Iterate over parameter key-value pairs
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&ParameterKey, &Value)> {
        self.values.iter()
    }

    /// Builder pattern: add a value and return self
    #[inline]
    #[must_use = "builder methods must be chained or built"]
    pub fn with_value(mut self, key: impl Into<ParameterKey>, value: impl Into<Value>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }

    /// Clear all values
    #[inline]
    pub fn clear(&mut self) {
        self.values.clear();
    }
}

// ============================================================================
// From/Into implementations for DisplayContext
// ============================================================================

impl From<HashMap<ParameterKey, Value>> for DisplayContext {
    fn from(values: HashMap<ParameterKey, Value>) -> Self {
        Self { values }
    }
}

impl From<DisplayContext> for HashMap<ParameterKey, Value> {
    fn from(ctx: DisplayContext) -> Self {
        ctx.values
    }
}

impl From<Vec<(ParameterKey, Value)>> for DisplayContext {
    fn from(pairs: Vec<(ParameterKey, Value)>) -> Self {
        Self {
            values: pairs.into_iter().collect(),
        }
    }
}

/// Create from array of pairs (const generic - works with any size)
impl<const N: usize> From<[(ParameterKey, Value); N]> for DisplayContext {
    fn from(pairs: [(ParameterKey, Value); N]) -> Self {
        Self {
            values: pairs.into_iter().collect(),
        }
    }
}

/// Create from array of (&str, Value) pairs
impl<const N: usize> From<[(&str, Value); N]> for DisplayContext {
    fn from(pairs: [(&str, Value); N]) -> Self {
        Self {
            values: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v))
                .collect(),
        }
    }
}

/// Convert to ValidationContext for nebula-validator
impl From<&DisplayContext> for ValidationContext {
    fn from(ctx: &DisplayContext) -> Self {
        let root = Value::Object(
            ctx.values
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.clone()))
                .collect()
        );
        ValidationContext::simple(root)
    }
}

/// Convert to Value::Object
impl From<&DisplayContext> for Value {
    fn from(ctx: &DisplayContext) -> Self {
        Value::Object(
            ctx.values
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.clone()))
                .collect()
        )
    }
}

impl From<DisplayContext> for Value {
    fn from(ctx: DisplayContext) -> Self {
        Value::Object(
            ctx.values
                .into_iter()
                .map(|(k, v)| (k.as_str().to_string(), v))
                .collect()
        )
    }
}

// ============================================================================
// Collection traits for DisplayContext
// ============================================================================

impl FromIterator<(ParameterKey, Value)> for DisplayContext {
    fn from_iter<T: IntoIterator<Item = (ParameterKey, Value)>>(iter: T) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

impl<'a> FromIterator<(&'a str, Value)> for DisplayContext {
    fn from_iter<T: IntoIterator<Item = (&'a str, Value)>>(iter: T) -> Self {
        Self {
            values: iter
                .into_iter()
                .map(|(k, v)| (k.into(), v))
                .collect(),
        }
    }
}

impl std::ops::Index<&ParameterKey> for DisplayContext {
    type Output = Value;

    fn index(&self, key: &ParameterKey) -> &Self::Output {
        &self.values[key]
    }
}

impl std::ops::Index<&str> for DisplayContext {
    type Output = Value;

    fn index(&self, key: &str) -> &Self::Output {
        &self.values[&ParameterKey::from(key)]
    }
}

impl Extend<(ParameterKey, Value)> for DisplayContext {
    fn extend<T: IntoIterator<Item = (ParameterKey, Value)>>(&mut self, iter: T) {
        self.values.extend(iter);
    }
}

impl IntoIterator for DisplayContext {
    type Item = (ParameterKey, Value);
    type IntoIter = std::collections::hash_map::IntoIter<ParameterKey, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'a> IntoIterator for &'a DisplayContext {
    type Item = (&'a ParameterKey, &'a Value);
    type IntoIter = std::collections::hash_map::Iter<'a, ParameterKey, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

// ============================================================================
// DisplayRule - wraps a validator for display conditions
// ============================================================================

/// A display rule that determines if a parameter should be shown
///
/// This wraps a `nebula-validator` Validator and uses it to check the state
/// of parameters. If validation passes, the condition is met.
///
/// # Examples
///
/// ```rust
/// # use nebula_parameter::{DisplayRule, DisplayContext};
/// # use nebula_value::Value;
/// // Single parameter check
/// let rule = DisplayRule::when_equals("auth_type", Value::text("api_key"));
///
/// // Cross-field check
/// let rule = DisplayRule::when_fields_equal("password", "confirm_password");
///
/// // Combine with operators
/// let combined = rule1 & rule2;  // AND
/// let either = rule1 | rule2;    // OR
/// let negated = !rule1;          // NOT
/// ```
#[derive(Clone)]
pub struct DisplayRule {
    validator: Box<dyn Validator>,
    dependencies: Option<Vec<ParameterKey>>,
    description: Option<String>,
}

impl DisplayRule {
    /// Create a new display rule with a validator
    pub fn new(validator: Box<dyn Validator>) -> Self {
        Self {
            validator,
            dependencies: None,
            description: None,
        }
    }

    /// Specify which parameters this rule depends on (for optimization)
    #[must_use = "builder methods must be chained or built"]
    pub fn with_dependencies(mut self, deps: impl IntoIterator<Item = impl Into<ParameterKey>>) -> Self {
        self.dependencies = Some(deps.into_iter().map(Into::into).collect());
        self
    }

    /// Add a description for debugging/documentation
    #[must_use = "builder methods must be chained or built"]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Evaluate this rule against a context (async)
    pub async fn evaluate(&self, ctx: &DisplayContext) -> bool {
        let validation_ctx = ValidationContext::from(ctx);
        let root = Value::from(ctx);

        self.validator
            .validate(&root, Some(&validation_ctx))
            .await
            .is_ok()
    }

    /// Evaluate this rule against a context (sync)
    pub fn evaluate_sync(&self, ctx: &DisplayContext) -> bool {
        futures::executor::block_on(self.evaluate(ctx))
    }

    /// Get the parameter dependencies
    pub fn dependencies(&self) -> Option<&[ParameterKey]> {
        self.dependencies.as_deref()
    }

    /// Get the description
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

impl std::fmt::Debug for DisplayRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayRule")
            .field("validator", &self.validator.name())
            .field("dependencies", &self.dependencies)
            .field("description", &self.description)
            .finish()
    }
}

// ============================================================================
// Bitwise operators for DisplayRule (idiomatic Rust!)
// ============================================================================

/// AND operator: `rule1 & rule2`
impl BitAnd for DisplayRule {
    type Output = DisplayRuleSet;

    fn bitand(self, rhs: Self) -> Self::Output {
        DisplayRuleSet::All(vec![
            DisplayRuleSet::Single(self),
            DisplayRuleSet::Single(rhs),
        ])
    }
}

/// OR operator: `rule1 | rule2`
impl BitOr for DisplayRule {
    type Output = DisplayRuleSet;

    fn bitor(self, rhs: Self) -> Self::Output {
        DisplayRuleSet::Any(vec![
            DisplayRuleSet::Single(self),
            DisplayRuleSet::Single(rhs),
        ])
    }
}

/// NOT operator: `!rule`
impl Not for DisplayRule {
    type Output = DisplayRuleSet;

    fn not(self) -> Self::Output {
        DisplayRuleSet::Not(Box::new(DisplayRuleSet::Single(self)))
    }
}

// ============================================================================
// DisplayRuleSet - composition of rules
// ============================================================================

/// A set of display rules combined with logical operators
///
/// # Examples
///
/// ```rust
/// # use nebula_parameter::{DisplayRule, DisplayRuleSet};
/// # use nebula_value::Value;
/// let rule1 = DisplayRule::when_true("advanced");
/// let rule2 = DisplayRule::when_greater_than("level", 10.0);
/// let rule3 = DisplayRule::when_equals("role", Value::text("admin"));
///
/// // Combine with operators
/// let ruleset = (rule1 & rule2) | rule3;
///
/// // Or explicitly
/// let ruleset = DisplayRuleSet::any([
///     DisplayRuleSet::all([rule1, rule2]),
///     DisplayRuleSet::Single(rule3),
/// ]);
/// ```
#[derive(Debug, Clone)]
pub enum DisplayRuleSet {
    /// A single rule
    Single(DisplayRule),
    /// All rules must pass (AND)
    All(Vec<DisplayRuleSet>),
    /// Any rule must pass (OR)
    Any(Vec<DisplayRuleSet>),
    /// Rule must not pass (NOT)
    Not(Box<DisplayRuleSet>),
}

impl DisplayRuleSet {
    /// Evaluate this ruleset (async)
    pub async fn evaluate(&self, ctx: &DisplayContext) -> bool {
        match self {
            DisplayRuleSet::Single(rule) => rule.evaluate(ctx).await,
            DisplayRuleSet::All(rules) => {
                for rule in rules {
                    if !rule.evaluate(ctx).await {
                        return false;
                    }
                }
                true
            }
            DisplayRuleSet::Any(rules) => {
                for rule in rules {
                    if rule.evaluate(ctx).await {
                        return true;
                    }
                }
                false
            }
            DisplayRuleSet::Not(rule) => !rule.evaluate(ctx).await,
        }
    }

    /// Evaluate this ruleset (sync)
    pub fn evaluate_sync(&self, ctx: &DisplayContext) -> bool {
        futures::executor::block_on(self.evaluate(ctx))
    }

    /// Get all parameter dependencies from this ruleset
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();
        self.collect_dependencies(&mut deps);
        deps.sort();
        deps.dedup();
        deps
    }

    fn collect_dependencies(&self, deps: &mut Vec<ParameterKey>) {
        match self {
            DisplayRuleSet::Single(rule) => {
                if let Some(rule_deps) = rule.dependencies() {
                    deps.extend_from_slice(rule_deps);
                }
            }
            DisplayRuleSet::All(rules) | DisplayRuleSet::Any(rules) => {
                for rule in rules {
                    rule.collect_dependencies(deps);
                }
            }
            DisplayRuleSet::Not(rule) => {
                rule.collect_dependencies(deps);
            }
        }
    }

    /// Create an ALL ruleset from an iterator
    pub fn all(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        DisplayRuleSet::All(rules.into_iter().map(Into::into).collect())
    }

    /// Create an ANY ruleset from an iterator
    pub fn any(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        DisplayRuleSet::Any(rules.into_iter().map(Into::into).collect())
    }

    /// Create a NOT ruleset
    pub fn not(rule: impl Into<DisplayRuleSet>) -> Self {
        DisplayRuleSet::Not(Box::new(rule.into()))
    }
}

// ============================================================================
// From implementations for DisplayRuleSet
// ============================================================================

impl From<DisplayRule> for DisplayRuleSet {
    fn from(rule: DisplayRule) -> Self {
        DisplayRuleSet::Single(rule)
    }
}

impl From<Vec<DisplayRule>> for DisplayRuleSet {
    fn from(rules: Vec<DisplayRule>) -> Self {
        if rules.len() == 1 {
            DisplayRuleSet::Single(rules.into_iter().next().unwrap())
        } else {
            DisplayRuleSet::All(rules.into_iter().map(Into::into).collect())
        }
    }
}

impl<const N: usize> From<[DisplayRule; N]> for DisplayRuleSet {
    fn from(rules: [DisplayRule; N]) -> Self {
        DisplayRuleSet::All(rules.into_iter().map(Into::into).collect())
    }
}

// ============================================================================
// Bitwise operators for DisplayRuleSet
// ============================================================================

impl BitAnd for DisplayRuleSet {
    type Output = DisplayRuleSet;

    fn bitand(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (DisplayRuleSet::All(mut left), DisplayRuleSet::All(right)) => {
                left.extend(right);
                DisplayRuleSet::All(left)
            }
            (DisplayRuleSet::All(mut rules), other) | (other, DisplayRuleSet::All(mut rules)) => {
                rules.push(other);
                DisplayRuleSet::All(rules)
            }
            (left, right) => DisplayRuleSet::All(vec![left, right]),
        }
    }
}

impl BitOr for DisplayRuleSet {
    type Output = DisplayRuleSet;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (DisplayRuleSet::Any(mut left), DisplayRuleSet::Any(right)) => {
                left.extend(right);
                DisplayRuleSet::Any(left)
            }
            (DisplayRuleSet::Any(mut rules), other) | (other, DisplayRuleSet::Any(mut rules)) => {
                rules.push(other);
                DisplayRuleSet::Any(rules)
            }
            (left, right) => DisplayRuleSet::Any(vec![left, right]),
        }
    }
}

impl Not for DisplayRuleSet {
    type Output = DisplayRuleSet;

    fn not(self) -> Self::Output {
        DisplayRuleSet::Not(Box::new(self))
    }
}

// ============================================================================
// ParameterDisplay - configuration for parameter visibility
// ============================================================================

/// Configuration determining when a parameter should be displayed
///
/// # Examples
///
/// ```rust
/// # use nebula_parameter::{ParameterDisplay, DisplayRule};
/// # use nebula_value::Value;
/// let display = ParameterDisplay::builder()
///     .show_when(
///         DisplayRule::when_equals("auth_type", Value::text("api_key"))
///         & DisplayRule::when_set("api_key")
///     )
///     .hide_when_false("enabled")
///     .build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct ParameterDisplay {
    show_when: Option<DisplayRuleSet>,
    hide_when: Option<DisplayRuleSet>,
}

impl ParameterDisplay {
    /// Create a new empty display configuration
    #[inline]
    pub const fn new() -> Self {
        Self {
            show_when: None,
            hide_when: None,
        }
    }

    /// Check if parameter should be displayed (async)
    pub async fn should_display(&self, ctx: &DisplayContext) -> bool {
        // Priority: hide_when is checked first
        if let Some(hide_rules) = &self.hide_when {
            if hide_rules.evaluate(ctx).await {
                return false;
            }
        }

        // Then check show_when
        if let Some(show_rules) = &self.show_when {
            return show_rules.evaluate(ctx).await;
        }

        // Default: show
        true
    }

    /// Check if parameter should be displayed (sync)
    #[inline]
    pub fn should_display_sync(&self, ctx: &DisplayContext) -> bool {
        futures::executor::block_on(self.should_display(ctx))
    }

    /// Get all parameter dependencies
    pub fn dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();

        if let Some(show) = &self.show_when {
            deps.extend(show.dependencies());
        }

        if let Some(hide) = &self.hide_when {
            deps.extend(hide.dependencies());
        }

        deps.sort();
        deps.dedup();
        deps
    }

    /// Create a builder
    #[inline]
    pub fn builder() -> ParameterDisplayBuilder {
        ParameterDisplayBuilder::new()
    }

    /// Check if this display has no conditions
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.show_when.is_none() && self.hide_when.is_none()
    }
}

// ============================================================================
// From implementations for ParameterDisplay
// ============================================================================

impl From<DisplayRule> for ParameterDisplay {
    fn from(rule: DisplayRule) -> Self {
        Self {
            show_when: Some(DisplayRuleSet::Single(rule)),
            hide_when: None,
        }
    }
}

impl From<DisplayRuleSet> for ParameterDisplay {
    fn from(ruleset: DisplayRuleSet) -> Self {
        Self {
            show_when: Some(ruleset),
            hide_when: None,
        }
    }
}

// ============================================================================
// ParameterDisplayBuilder - fluent API for building display config
// ============================================================================

/// Builder for creating `ParameterDisplay` with a fluent API
///
/// # Examples
///
/// ```rust
/// # use nebula_parameter::{ParameterDisplay, DisplayRule, DisplayRuleSet};
/// # use nebula_value::Value;
/// let display = ParameterDisplay::builder()
///     .show_when_equals("mode", Value::text("advanced"))
///     .show_when_all([
///         DisplayRule::when_greater_than("level", 10.0),
///         DisplayRule::when_true("enabled"),
///     ])
///     .hide_when_equals("env", Value::text("production"))
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct ParameterDisplayBuilder {
    show_rules: Vec<DisplayRuleSet>,
    hide_rules: Vec<DisplayRuleSet>,
}

impl ParameterDisplayBuilder {
    /// Create a new builder
    #[inline]
    pub const fn new() -> Self {
        Self {
            show_rules: Vec::new(),
            hide_rules: Vec::new(),
        }
    }

    /// Add a show condition
    #[must_use = "builder methods must be chained or built"]
    pub fn show_when(mut self, rule: impl Into<DisplayRuleSet>) -> Self {
        self.show_rules.push(rule.into());
        self
    }

    /// Add a hide condition
    #[must_use = "builder methods must be chained or built"]
    pub fn hide_when(mut self, rule: impl Into<DisplayRuleSet>) -> Self {
        self.hide_rules.push(rule.into());
        self
    }

    /// Add multiple show conditions (all must pass)
    #[must_use = "builder methods must be chained or built"]
    pub fn show_when_all(mut self, rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        self.show_rules.push(DisplayRuleSet::all(rules));
        self
    }

    /// Add multiple show conditions (any must pass)
    #[must_use = "builder methods must be chained or built"]
    pub fn show_when_any(mut self, rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        self.show_rules.push(DisplayRuleSet::any(rules));
        self
    }

    /// Add multiple hide conditions (all must pass)
    #[must_use = "builder methods must be chained or built"]
    pub fn hide_when_all(mut self, rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        self.hide_rules.push(DisplayRuleSet::all(rules));
        self
    }

    /// Add multiple hide conditions (any must pass)
    #[must_use = "builder methods must be chained or built"]
    pub fn hide_when_any(mut self, rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self {
        self.hide_rules.push(DisplayRuleSet::any(rules));
        self
    }

    // Convenience methods for common cases

    /// Show when parameter equals value
    #[must_use = "builder methods must be chained or built"]
    pub fn show_when_equals(self, parameter: impl Into<ParameterKey>, value: impl Into<Value>) -> Self {
        self.show_when(DisplayRule::when_equals(parameter, value.into()))
    }

    /// Hide when parameter equals value
    #[must_use = "builder methods must be chained or built"]
    pub fn hide_when_equals(self, parameter: impl Into<ParameterKey>, value: impl Into<Value>) -> Self {
        self.hide_when(DisplayRule::when_equals(parameter, value.into()))
    }

    /// Show when parameter is set (not null)
    #[must_use = "builder methods must be chained or built"]
    pub fn show_when_set(self, parameter: impl Into<ParameterKey>) -> Self {
        self.show_when(DisplayRule::when_set(parameter))
    }

    /// Show when parameter is true
    #[must_use = "builder methods must be chained or built"]
    pub fn show_when_true(self, parameter: impl Into<ParameterKey>) -> Self {
        self.show_when(DisplayRule::when_true(parameter))
    }

    /// Hide when parameter is false
    #[must_use = "builder methods must be chained or built"]
    pub fn hide_when_false(self, parameter: impl Into<ParameterKey>) -> Self {
        self.hide_when(DisplayRule::when_false(parameter))
    }

    /// Build the final ParameterDisplay
    pub fn build(self) -> ParameterDisplay {
        ParameterDisplay {
            show_when: match self.show_rules.len() {
                0 => None,
                1 => Some(self.show_rules.into_iter().next().unwrap()),
                _ => Some(DisplayRuleSet::All(self.show_rules)),
            },
            hide_when: match self.hide_rules.len() {
                0 => None,
                1 => Some(self.hide_rules.into_iter().next().unwrap()),
                _ => Some(DisplayRuleSet::Any(self.hide_rules)),
            },
        }
    }
}

// ============================================================================
// Convenience constructors for DisplayRule
// ============================================================================

impl DisplayRule {
    /// Parameter equals value
    pub fn when_equals(parameter: impl Into<ParameterKey>, value: Value) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(equals(value)))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter is true
    pub fn when_true(parameter: impl Into<ParameterKey>) -> Self {
        Self::when_equals(parameter, Value::boolean(true))
    }

    /// Parameter is false
    pub fn when_false(parameter: impl Into<ParameterKey>) -> Self {
        Self::when_equals(parameter, Value::boolean(false))
    }

    /// Parameter is set (not null)
    pub fn when_set(parameter: impl Into<ParameterKey>) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(not_null()))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter is not empty
    pub fn when_not_empty(parameter: impl Into<ParameterKey>) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(not_empty()))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter is greater than value
    pub fn when_greater_than(parameter: impl Into<ParameterKey>, value: f64) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(greater_than(value)))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter is less than value
    pub fn when_less_than(parameter: impl Into<ParameterKey>, value: f64) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(less_than(value)))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter is in range
    pub fn when_between(parameter: impl Into<ParameterKey>, min: f64, max: f64) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(range(min, max)))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter contains substring
    pub fn when_contains(parameter: impl Into<ParameterKey>, substring: String) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(string_contains(substring)))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter starts with prefix
    pub fn when_starts_with(parameter: impl Into<ParameterKey>, prefix: String) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(string_starts_with(prefix)))
        ))
        .with_dependencies([param_key])
    }

    /// Parameter is in list of values
    pub fn when_in(parameter: impl Into<ParameterKey>, values: Vec<Value>) -> Self {
        use nebula_validator::*;

        let param_key = parameter.into();
        Self::new(Box::new(
            WhenFieldValidator::new(param_key.as_str().to_string(), Box::new(one_of(values)))
        ))
        .with_dependencies([param_key])
    }

    /// Two fields are equal (cross-field validation)
    pub fn when_fields_equal(field1: &str, field2: &str) -> Self {
        Self::new(Box::new(FieldsEqualValidator::new(field1, field2)))
            .with_dependencies([field1, field2])
            .with_description(format!("Fields '{}' and '{}' must be equal", field1, field2))
    }

    /// Two fields are different (cross-field validation)
    pub fn when_fields_different(field1: &str, field2: &str) -> Self {
        use nebula_validator::*;

        Self::new(Box::new(different_from_str(field2)))
            .with_dependencies([field1, field2])
            .with_description(format!("Fields '{}' and '{}' must be different", field1, field2))
    }

    /// Field is required if another field has a specific value
    pub fn when_required_if(field: &str, other_field: &str, other_value: Value) -> Self {
        use nebula_validator::*;

        Self::new(Box::new(required_if_field_str(other_field, other_value)))
            .with_dependencies([field, other_field])
            .with_description(format!("Field '{}' is required when '{}' has specific value", field, other_field))
    }

    /// Custom validator
    pub fn custom(validator: Box<dyn Validator>) -> Self {
        Self::new(validator)
    }

    /// Custom validator with explicit dependencies
    pub fn custom_with_deps(
        validator: Box<dyn Validator>,
        dependencies: impl IntoIterator<Item = impl Into<ParameterKey>>,
    ) -> Self {
        Self::new(validator).with_dependencies(dependencies)
    }
}

// ============================================================================
// Errors
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_context_from_array() {
        let ctx: DisplayContext = [
            ("auth_type", Value::text("api_key")),
            ("level", Value::integer(15)),
        ].into();

        assert_eq!(ctx.len(), 2);
        assert_eq!(ctx["auth_type"], Value::text("api_key"));
    }

    #[test]
    fn test_display_rule_operators() {
        let rule1 = DisplayRule::when_true("enabled");
        let rule2 = DisplayRule::when_greater_than("level", 10.0);

        let _and = rule1.clone() & rule2.clone();
        let _or = rule1.clone() | rule2.clone();
        let _not = !rule1;
    }

    #[test]
    fn test_display_builder() {
        let display = ParameterDisplay::builder()
            .show_when_equals("auth_type", Value::text("api_key"))
            .hide_when_false("enabled")
            .build();

        assert!(!display.is_empty());
    }

    #[tokio::test]
    async fn test_should_display() {
        let ctx = DisplayContext::from([
            ("auth_type", Value::text("api_key")),
            ("enabled", Value::boolean(true)),
        ]);

        let display = ParameterDisplay::builder()
            .show_when_equals("auth_type", Value::text("api_key"))
            .hide_when_false("enabled")
            .build();

        assert!(display.should_display(&ctx).await);
    }
}