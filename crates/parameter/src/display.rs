use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A condition that determines parameter visibility based on another field's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "condition", rename_all = "snake_case")]
pub enum DisplayCondition {
    /// Field value equals the given value.
    Equals { value: serde_json::Value },
    /// Field value does not equal the given value.
    NotEquals { value: serde_json::Value },
    /// Field has a value (not null and not missing).
    IsSet,
    /// Field value is null.
    IsNull,
    /// Field value is an empty string or empty array.
    IsEmpty,
    /// Field value is a non-empty string or non-empty array.
    IsNotEmpty,
    /// Field value is boolean true.
    IsTrue,
    /// Field value is boolean false.
    IsFalse,
    /// Field numeric value is greater than the threshold.
    GreaterThan { value: f64 },
    /// Field numeric value is less than the threshold.
    LessThan { value: f64 },
    /// Field numeric value is within the inclusive range.
    InRange { min: f64, max: f64 },
    /// Field value (string or array) contains the given value.
    Contains { value: serde_json::Value },
    /// Field string value starts with the given prefix.
    StartsWith { prefix: String },
    /// Field string value ends with the given suffix.
    EndsWith { suffix: String },
    /// Field value is one of the given values.
    OneOf { values: Vec<serde_json::Value> },
    /// Field passes its own validation rules.
    IsValid,
}

impl DisplayCondition {
    /// Evaluate this condition against a concrete value.
    #[must_use]
    pub fn evaluate(&self, value: &serde_json::Value) -> bool {
        match self {
            Self::Equals { value: expected } => value == expected,
            Self::NotEquals { value: expected } => value != expected,
            Self::IsSet => !value.is_null(),
            Self::IsNull => value.is_null(),
            Self::IsEmpty => match value {
                serde_json::Value::String(s) => s.is_empty(),
                serde_json::Value::Array(a) => a.is_empty(),
                serde_json::Value::Null => true,
                _ => false,
            },
            Self::IsNotEmpty => match value {
                serde_json::Value::String(s) => !s.is_empty(),
                serde_json::Value::Array(a) => !a.is_empty(),
                serde_json::Value::Null => false,
                _ => true,
            },
            Self::IsTrue => value.as_bool() == Some(true),
            Self::IsFalse => value.as_bool() == Some(false),
            Self::GreaterThan { value: threshold } => {
                value.as_f64().is_some_and(|n| n > *threshold)
            }
            Self::LessThan { value: threshold } => value.as_f64().is_some_and(|n| n < *threshold),
            Self::InRange { min, max } => value.as_f64().is_some_and(|n| n >= *min && n <= *max),
            Self::Contains {
                value: search_value,
            } => match value {
                serde_json::Value::String(s) => {
                    if let Some(needle) = search_value.as_str() {
                        s.contains(needle)
                    } else {
                        false
                    }
                }
                serde_json::Value::Array(arr) => arr.contains(search_value),
                _ => false,
            },
            Self::StartsWith { prefix } => value
                .as_str()
                .is_some_and(|s| s.starts_with(prefix.as_str())),
            Self::EndsWith { suffix } => {
                value.as_str().is_some_and(|s| s.ends_with(suffix.as_str()))
            }
            Self::OneOf { values } => values.contains(value),
            // IsValid cannot be evaluated from the value alone; it requires
            // validation state from the context. Always returns false here.
            Self::IsValid => false,
        }
    }
}

/// Holds current parameter values and validation state for display rule evaluation.
#[derive(Debug, Clone, Default)]
pub struct DisplayContext {
    values: HashMap<String, serde_json::Value>,
    validation: HashMap<String, bool>,
}

impl DisplayContext {
    /// Create an empty display context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a parameter value by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.values.get(key)
    }

    /// Look up whether a parameter is currently valid.
    #[must_use]
    pub fn get_validation(&self, key: &str) -> Option<bool> {
        self.validation.get(key).copied()
    }

    /// Set a parameter value (builder-style, consuming).
    #[must_use]
    pub fn with_value(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.values.insert(key.into(), value);
        self
    }

    /// Set a parameter's validation state (builder-style, consuming).
    #[must_use]
    pub fn with_validation(mut self, key: impl Into<String>, valid: bool) -> Self {
        self.validation.insert(key.into(), valid);
        self
    }
}

/// A single display rule: check a named field against a condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayRule {
    /// The parameter key to check.
    pub field: String,
    /// The condition to evaluate against that field's value.
    pub condition: DisplayCondition,
}

impl DisplayRule {
    /// Evaluate this rule against a display context.
    #[must_use]
    pub fn evaluate(&self, context: &DisplayContext) -> bool {
        // Special case: IsValid checks validation state, not value.
        if self.condition == DisplayCondition::IsValid {
            return context.get_validation(&self.field).unwrap_or(false);
        }

        let value = context
            .get(&self.field)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        self.condition.evaluate(&value)
    }
}

/// Composable display logic: combine rules with AND, OR, NOT.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "logic", rename_all = "snake_case")]
pub enum DisplayRuleSet {
    /// A single rule.
    Single(DisplayRule),
    /// All nested rules must match.
    All { rules: Vec<DisplayRuleSet> },
    /// At least one nested rule must match.
    Any { rules: Vec<DisplayRuleSet> },
    /// Negates the nested rule.
    Not { rule: Box<DisplayRuleSet> },
}

impl DisplayRuleSet {
    /// Evaluate the rule set against a display context.
    #[must_use]
    pub fn evaluate(&self, context: &DisplayContext) -> bool {
        match self {
            Self::Single(rule) => rule.evaluate(context),
            Self::All { rules } => rules.iter().all(|r| r.evaluate(context)),
            Self::Any { rules } => rules.iter().any(|r| r.evaluate(context)),
            Self::Not { rule } => !rule.evaluate(context),
        }
    }

    /// Collect all field names referenced by this rule set.
    #[must_use]
    pub fn dependencies(&self) -> Vec<String> {
        let mut deps = Vec::new();
        self.collect_dependencies(&mut deps);
        deps.sort();
        deps.dedup();
        deps
    }

    fn collect_dependencies(&self, deps: &mut Vec<String>) {
        match self {
            Self::Single(rule) => deps.push(rule.field.clone()),
            Self::All { rules } | Self::Any { rules } => {
                for r in rules {
                    r.collect_dependencies(deps);
                }
            }
            Self::Not { rule } => rule.collect_dependencies(deps),
        }
    }
}

/// Controls when a parameter is shown or hidden in the UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterDisplay {
    /// Rules that must match for the parameter to be visible.
    /// If empty, the parameter is visible by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub show_when: Vec<DisplayRuleSet>,

    /// Rules that, if matched, hide the parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide_when: Vec<DisplayRuleSet>,
}

impl ParameterDisplay {
    /// Determine whether the parameter should be displayed given current context.
    ///
    /// Logic:
    /// - If `show_when` is empty, the parameter is visible by default.
    /// - If `show_when` is non-empty, at least one rule set must match.
    /// - If any `hide_when` rule set matches, the parameter is hidden.
    #[must_use]
    pub fn should_display(&self, context: &DisplayContext) -> bool {
        // Check hide rules first — any match hides the parameter.
        if self.hide_when.iter().any(|r| r.evaluate(context)) {
            return false;
        }

        // If no show rules, visible by default.
        if self.show_when.is_empty() {
            return true;
        }

        // At least one show rule must match.
        self.show_when.iter().any(|r| r.evaluate(context))
    }

    /// Whether this display configuration has no rules at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.show_when.is_empty() && self.hide_when.is_empty()
    }

    /// Collect all field names that this display configuration depends on.
    #[must_use]
    pub fn dependencies(&self) -> Vec<String> {
        let mut deps = Vec::new();
        for rule_set in &self.show_when {
            for dep in rule_set.dependencies() {
                deps.push(dep);
            }
        }
        for rule_set in &self.hide_when {
            for dep in rule_set.dependencies() {
                deps.push(dep);
            }
        }
        deps.sort();
        deps.dedup();
        deps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── DisplayCondition::evaluate ──

    #[test]
    fn equals_matches_same_value() {
        let cond = DisplayCondition::Equals {
            value: json!("hello"),
        };
        assert!(cond.evaluate(&json!("hello")));
        assert!(!cond.evaluate(&json!("world")));
    }

    #[test]
    fn not_equals() {
        let cond = DisplayCondition::NotEquals { value: json!(42) };
        assert!(cond.evaluate(&json!(99)));
        assert!(!cond.evaluate(&json!(42)));
    }

    #[test]
    fn is_set_and_is_null() {
        assert!(DisplayCondition::IsSet.evaluate(&json!("x")));
        assert!(!DisplayCondition::IsSet.evaluate(&json!(null)));
        assert!(DisplayCondition::IsNull.evaluate(&json!(null)));
        assert!(!DisplayCondition::IsNull.evaluate(&json!("x")));
    }

    #[test]
    fn is_empty_and_is_not_empty() {
        assert!(DisplayCondition::IsEmpty.evaluate(&json!("")));
        assert!(DisplayCondition::IsEmpty.evaluate(&json!([])));
        assert!(DisplayCondition::IsEmpty.evaluate(&json!(null)));
        assert!(!DisplayCondition::IsEmpty.evaluate(&json!("a")));
        assert!(!DisplayCondition::IsEmpty.evaluate(&json!([1])));

        assert!(DisplayCondition::IsNotEmpty.evaluate(&json!("a")));
        assert!(DisplayCondition::IsNotEmpty.evaluate(&json!([1])));
        assert!(!DisplayCondition::IsNotEmpty.evaluate(&json!("")));
        assert!(!DisplayCondition::IsNotEmpty.evaluate(&json!([])));
        assert!(!DisplayCondition::IsNotEmpty.evaluate(&json!(null)));
    }

    #[test]
    fn is_true_and_is_false() {
        assert!(DisplayCondition::IsTrue.evaluate(&json!(true)));
        assert!(!DisplayCondition::IsTrue.evaluate(&json!(false)));
        assert!(!DisplayCondition::IsTrue.evaluate(&json!(1)));

        assert!(DisplayCondition::IsFalse.evaluate(&json!(false)));
        assert!(!DisplayCondition::IsFalse.evaluate(&json!(true)));
    }

    #[test]
    fn greater_than_and_less_than() {
        let gt = DisplayCondition::GreaterThan { value: 10.0 };
        assert!(gt.evaluate(&json!(11)));
        assert!(!gt.evaluate(&json!(10)));
        assert!(!gt.evaluate(&json!(9)));
        assert!(!gt.evaluate(&json!("not a number")));

        let lt = DisplayCondition::LessThan { value: 5.0 };
        assert!(lt.evaluate(&json!(4)));
        assert!(!lt.evaluate(&json!(5)));
        assert!(!lt.evaluate(&json!(6)));
    }

    #[test]
    fn in_range() {
        let cond = DisplayCondition::InRange {
            min: 1.0,
            max: 10.0,
        };
        assert!(cond.evaluate(&json!(1)));
        assert!(cond.evaluate(&json!(5)));
        assert!(cond.evaluate(&json!(10)));
        assert!(!cond.evaluate(&json!(0)));
        assert!(!cond.evaluate(&json!(11)));
    }

    #[test]
    fn contains_string_and_array() {
        let cond = DisplayCondition::Contains {
            value: json!("world"),
        };
        assert!(cond.evaluate(&json!("hello world")));
        assert!(!cond.evaluate(&json!("hello")));

        let cond_arr = DisplayCondition::Contains { value: json!(2) };
        assert!(cond_arr.evaluate(&json!([1, 2, 3])));
        assert!(!cond_arr.evaluate(&json!([1, 3])));
    }

    #[test]
    fn starts_with_and_ends_with() {
        let sw = DisplayCondition::StartsWith {
            prefix: "http".into(),
        };
        assert!(sw.evaluate(&json!("https://example.com")));
        assert!(!sw.evaluate(&json!("ftp://example.com")));

        let ew = DisplayCondition::EndsWith {
            suffix: ".json".into(),
        };
        assert!(ew.evaluate(&json!("data.json")));
        assert!(!ew.evaluate(&json!("data.xml")));
    }

    #[test]
    fn one_of() {
        let cond = DisplayCondition::OneOf {
            values: vec![json!("a"), json!("b"), json!("c")],
        };
        assert!(cond.evaluate(&json!("b")));
        assert!(!cond.evaluate(&json!("d")));
    }

    #[test]
    fn is_valid_always_false_from_evaluate() {
        // IsValid needs context, raw evaluate always returns false.
        assert!(!DisplayCondition::IsValid.evaluate(&json!("anything")));
    }

    // ── DisplayContext ──

    #[test]
    fn context_builder() {
        let ctx = DisplayContext::new()
            .with_value("mode", json!("advanced"))
            .with_validation("email", true);

        assert_eq!(ctx.get("mode"), Some(&json!("advanced")));
        assert_eq!(ctx.get("missing"), None);
        assert_eq!(ctx.get_validation("email"), Some(true));
        assert_eq!(ctx.get_validation("missing"), None);
    }

    // ── DisplayRule ──

    #[test]
    fn rule_evaluates_against_context() {
        let rule = DisplayRule {
            field: "mode".into(),
            condition: DisplayCondition::Equals {
                value: json!("advanced"),
            },
        };
        let ctx = DisplayContext::new().with_value("mode", json!("advanced"));
        assert!(rule.evaluate(&ctx));

        let ctx2 = DisplayContext::new().with_value("mode", json!("simple"));
        assert!(!rule.evaluate(&ctx2));
    }

    #[test]
    fn rule_missing_field_uses_null() {
        let rule = DisplayRule {
            field: "missing".into(),
            condition: DisplayCondition::IsNull,
        };
        let ctx = DisplayContext::new();
        assert!(rule.evaluate(&ctx));
    }

    #[test]
    fn rule_is_valid_checks_validation_state() {
        let rule = DisplayRule {
            field: "email".into(),
            condition: DisplayCondition::IsValid,
        };
        let ctx_valid = DisplayContext::new().with_validation("email", true);
        assert!(rule.evaluate(&ctx_valid));

        let ctx_invalid = DisplayContext::new().with_validation("email", false);
        assert!(!rule.evaluate(&ctx_invalid));

        let ctx_missing = DisplayContext::new();
        assert!(!rule.evaluate(&ctx_missing));
    }

    // ── DisplayRuleSet ──

    #[test]
    fn all_requires_all_rules() {
        let rule_set = DisplayRuleSet::All {
            rules: vec![
                DisplayRuleSet::Single(DisplayRule {
                    field: "a".into(),
                    condition: DisplayCondition::IsTrue,
                }),
                DisplayRuleSet::Single(DisplayRule {
                    field: "b".into(),
                    condition: DisplayCondition::IsTrue,
                }),
            ],
        };

        let both = DisplayContext::new()
            .with_value("a", json!(true))
            .with_value("b", json!(true));
        assert!(rule_set.evaluate(&both));

        let one = DisplayContext::new()
            .with_value("a", json!(true))
            .with_value("b", json!(false));
        assert!(!rule_set.evaluate(&one));
    }

    #[test]
    fn any_requires_at_least_one() {
        let rule_set = DisplayRuleSet::Any {
            rules: vec![
                DisplayRuleSet::Single(DisplayRule {
                    field: "a".into(),
                    condition: DisplayCondition::IsTrue,
                }),
                DisplayRuleSet::Single(DisplayRule {
                    field: "b".into(),
                    condition: DisplayCondition::IsTrue,
                }),
            ],
        };

        let one = DisplayContext::new()
            .with_value("a", json!(false))
            .with_value("b", json!(true));
        assert!(rule_set.evaluate(&one));

        let none = DisplayContext::new()
            .with_value("a", json!(false))
            .with_value("b", json!(false));
        assert!(!rule_set.evaluate(&none));
    }

    #[test]
    fn not_negates() {
        let rule_set = DisplayRuleSet::Not {
            rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                field: "debug".into(),
                condition: DisplayCondition::IsTrue,
            })),
        };

        let ctx_true = DisplayContext::new().with_value("debug", json!(true));
        assert!(!rule_set.evaluate(&ctx_true));

        let ctx_false = DisplayContext::new().with_value("debug", json!(false));
        assert!(rule_set.evaluate(&ctx_false));
    }

    #[test]
    fn dependencies_collected_and_deduplicated() {
        let rule_set = DisplayRuleSet::All {
            rules: vec![
                DisplayRuleSet::Single(DisplayRule {
                    field: "mode".into(),
                    condition: DisplayCondition::IsSet,
                }),
                DisplayRuleSet::Single(DisplayRule {
                    field: "mode".into(),
                    condition: DisplayCondition::IsNotEmpty,
                }),
                DisplayRuleSet::Single(DisplayRule {
                    field: "level".into(),
                    condition: DisplayCondition::IsSet,
                }),
            ],
        };

        let deps = rule_set.dependencies();
        assert_eq!(deps, vec!["level", "mode"]);
    }

    // ── ParameterDisplay ──

    #[test]
    fn empty_display_always_visible() {
        let display = ParameterDisplay::default();
        assert!(display.is_empty());
        assert!(display.should_display(&DisplayContext::new()));
    }

    #[test]
    fn show_when_controls_visibility() {
        let display = ParameterDisplay {
            show_when: vec![DisplayRuleSet::Single(DisplayRule {
                field: "mode".into(),
                condition: DisplayCondition::Equals {
                    value: json!("advanced"),
                },
            })],
            hide_when: vec![],
        };

        let ctx_match = DisplayContext::new().with_value("mode", json!("advanced"));
        assert!(display.should_display(&ctx_match));

        let ctx_no_match = DisplayContext::new().with_value("mode", json!("simple"));
        assert!(!display.should_display(&ctx_no_match));
    }

    #[test]
    fn hide_when_overrides_show_when() {
        let display = ParameterDisplay {
            show_when: vec![DisplayRuleSet::Single(DisplayRule {
                field: "enabled".into(),
                condition: DisplayCondition::IsTrue,
            })],
            hide_when: vec![DisplayRuleSet::Single(DisplayRule {
                field: "locked".into(),
                condition: DisplayCondition::IsTrue,
            })],
        };

        let ctx = DisplayContext::new()
            .with_value("enabled", json!(true))
            .with_value("locked", json!(true));
        assert!(!display.should_display(&ctx));
    }

    #[test]
    fn display_dependencies() {
        let display = ParameterDisplay {
            show_when: vec![DisplayRuleSet::Single(DisplayRule {
                field: "mode".into(),
                condition: DisplayCondition::IsSet,
            })],
            hide_when: vec![DisplayRuleSet::Single(DisplayRule {
                field: "locked".into(),
                condition: DisplayCondition::IsTrue,
            })],
        };

        let deps = display.dependencies();
        assert_eq!(deps, vec!["locked", "mode"]);
    }

    // ── Serde round-trips ──

    #[test]
    fn condition_serde_round_trip() {
        let conditions = vec![
            DisplayCondition::Equals {
                value: json!("test"),
            },
            DisplayCondition::NotEquals { value: json!(42) },
            DisplayCondition::IsSet,
            DisplayCondition::IsNull,
            DisplayCondition::IsEmpty,
            DisplayCondition::IsNotEmpty,
            DisplayCondition::IsTrue,
            DisplayCondition::IsFalse,
            DisplayCondition::GreaterThan { value: 10.0 },
            DisplayCondition::LessThan { value: 5.0 },
            DisplayCondition::InRange {
                min: 1.0,
                max: 100.0,
            },
            DisplayCondition::Contains {
                value: json!("needle"),
            },
            DisplayCondition::StartsWith {
                prefix: "http".into(),
            },
            DisplayCondition::EndsWith {
                suffix: ".json".into(),
            },
            DisplayCondition::OneOf {
                values: vec![json!("a"), json!("b")],
            },
            DisplayCondition::IsValid,
        ];

        for cond in &conditions {
            let json = serde_json::to_string(cond).unwrap();
            let deserialized: DisplayCondition = serde_json::from_str(&json).unwrap();
            assert_eq!(*cond, deserialized, "round-trip failed for {json}");
        }
    }

    #[test]
    fn display_serde_round_trip() {
        let display = ParameterDisplay {
            show_when: vec![DisplayRuleSet::Single(DisplayRule {
                field: "mode".into(),
                condition: DisplayCondition::Equals {
                    value: json!("advanced"),
                },
            })],
            hide_when: vec![],
        };

        let json = serde_json::to_string(&display).unwrap();
        let deserialized: ParameterDisplay = serde_json::from_str(&json).unwrap();
        assert_eq!(display, deserialized);
    }

    #[test]
    fn empty_display_omits_fields_in_json() {
        let display = ParameterDisplay::default();
        let json = serde_json::to_string(&display).unwrap();
        assert!(!json.contains("show_when"));
        assert!(!json.contains("hide_when"));
    }
}
