use nebula_parameter::display::{
    DisplayCondition, DisplayContext, DisplayRule, DisplayRuleSet, ParameterDisplay,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// 1. Complex display conditions (nested All/Any/Not)
// ---------------------------------------------------------------------------

#[test]
fn deeply_nested_all_any_not() {
    // Structure: All[
    //   enabled == true,
    //   Any[
    //     mode == "advanced",
    //     Not(tier == "free")
    //   ]
    // ]
    let rule_set = DisplayRuleSet::All {
        rules: vec![
            DisplayRuleSet::Single(DisplayRule {
                field: "enabled".into(),
                condition: DisplayCondition::IsTrue,
            }),
            DisplayRuleSet::Any {
                rules: vec![
                    DisplayRuleSet::Single(DisplayRule {
                        field: "mode".into(),
                        condition: DisplayCondition::Equals {
                            value: json!("advanced"),
                        },
                    }),
                    DisplayRuleSet::Not {
                        rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                            field: "tier".into(),
                            condition: DisplayCondition::Equals {
                                value: json!("free"),
                            },
                        })),
                    },
                ],
            },
        ],
    };

    // enabled=true, mode=advanced -> true (All conditions met)
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(true))
        .with_value("mode", json!("advanced"))
        .with_value("tier", json!("free"));
    assert!(rule_set.evaluate(&ctx));

    // enabled=true, mode=basic, tier=pro -> true (Not(free) passes)
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(true))
        .with_value("mode", json!("basic"))
        .with_value("tier", json!("pro"));
    assert!(rule_set.evaluate(&ctx));

    // enabled=false -> false (All fails on first condition)
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(false))
        .with_value("mode", json!("advanced"))
        .with_value("tier", json!("pro"));
    assert!(!rule_set.evaluate(&ctx));

    // enabled=true, mode=basic, tier=free -> false (Any: neither branch passes)
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(true))
        .with_value("mode", json!("basic"))
        .with_value("tier", json!("free"));
    assert!(!rule_set.evaluate(&ctx));
}

#[test]
fn triple_nested_not() {
    // Not(Not(Not(field == "yes"))) should be equivalent to Not(field == "yes")
    let rule_set = DisplayRuleSet::Not {
        rule: Box::new(DisplayRuleSet::Not {
            rule: Box::new(DisplayRuleSet::Not {
                rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                    field: "answer".into(),
                    condition: DisplayCondition::Equals {
                        value: json!("yes"),
                    },
                })),
            }),
        }),
    };

    let ctx_yes = DisplayContext::new().with_value("answer", json!("yes"));
    assert!(!rule_set.evaluate(&ctx_yes));

    let ctx_no = DisplayContext::new().with_value("answer", json!("no"));
    assert!(rule_set.evaluate(&ctx_no));
}

#[test]
fn empty_all_evaluates_true() {
    let rule_set = DisplayRuleSet::All { rules: vec![] };
    let ctx = DisplayContext::new();
    assert!(rule_set.evaluate(&ctx));
}

#[test]
fn empty_any_evaluates_false() {
    let rule_set = DisplayRuleSet::Any { rules: vec![] };
    let ctx = DisplayContext::new();
    assert!(!rule_set.evaluate(&ctx));
}

// ---------------------------------------------------------------------------
// 2. should_display with show_when and hide_when together
// ---------------------------------------------------------------------------

#[test]
fn show_when_only_controls_visibility() {
    let display = ParameterDisplay {
        show_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "mode".into(),
            condition: DisplayCondition::Equals {
                value: json!("advanced"),
            },
        })],
        hide_when: vec![],
    };

    let ctx_advanced = DisplayContext::new().with_value("mode", json!("advanced"));
    assert!(display.should_display(&ctx_advanced));

    let ctx_simple = DisplayContext::new().with_value("mode", json!("simple"));
    assert!(!display.should_display(&ctx_simple));

    let ctx_empty = DisplayContext::new();
    assert!(!display.should_display(&ctx_empty));
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

    // show passes, hide does not -> visible
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(true))
        .with_value("locked", json!(false));
    assert!(display.should_display(&ctx));

    // show passes, hide also passes -> hidden (hide wins)
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(true))
        .with_value("locked", json!(true));
    assert!(!display.should_display(&ctx));

    // show fails -> hidden regardless of hide_when
    let ctx = DisplayContext::new()
        .with_value("enabled", json!(false))
        .with_value("locked", json!(false));
    assert!(!display.should_display(&ctx));
}

#[test]
fn no_rules_means_always_visible() {
    let display = ParameterDisplay::default();
    assert!(display.is_empty());
    assert!(display.should_display(&DisplayContext::new()));
    assert!(display.should_display(&DisplayContext::new().with_value("anything", json!(42))));
}

#[test]
fn hide_when_only_hides_from_default_visible() {
    let display = ParameterDisplay {
        show_when: vec![],
        hide_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "maintenance".into(),
            condition: DisplayCondition::IsTrue,
        })],
    };

    // No show_when -> visible by default, but hide_when can suppress.
    let ctx_normal = DisplayContext::new().with_value("maintenance", json!(false));
    assert!(display.should_display(&ctx_normal));

    let ctx_maintenance = DisplayContext::new().with_value("maintenance", json!(true));
    assert!(!display.should_display(&ctx_maintenance));
}

#[test]
fn multiple_show_when_any_match_suffices() {
    let display = ParameterDisplay {
        show_when: vec![
            DisplayRuleSet::Single(DisplayRule {
                field: "role".into(),
                condition: DisplayCondition::Equals {
                    value: json!("admin"),
                },
            }),
            DisplayRuleSet::Single(DisplayRule {
                field: "role".into(),
                condition: DisplayCondition::Equals {
                    value: json!("superadmin"),
                },
            }),
        ],
        hide_when: vec![],
    };

    let ctx_admin = DisplayContext::new().with_value("role", json!("admin"));
    assert!(display.should_display(&ctx_admin));

    let ctx_super = DisplayContext::new().with_value("role", json!("superadmin"));
    assert!(display.should_display(&ctx_super));

    let ctx_user = DisplayContext::new().with_value("role", json!("user"));
    assert!(!display.should_display(&ctx_user));
}

#[test]
fn multiple_hide_when_any_match_hides() {
    let display = ParameterDisplay {
        show_when: vec![],
        hide_when: vec![
            DisplayRuleSet::Single(DisplayRule {
                field: "disabled".into(),
                condition: DisplayCondition::IsTrue,
            }),
            DisplayRuleSet::Single(DisplayRule {
                field: "maintenance".into(),
                condition: DisplayCondition::IsTrue,
            }),
        ],
    };

    let ctx_both_false = DisplayContext::new()
        .with_value("disabled", json!(false))
        .with_value("maintenance", json!(false));
    assert!(display.should_display(&ctx_both_false));

    let ctx_disabled = DisplayContext::new()
        .with_value("disabled", json!(true))
        .with_value("maintenance", json!(false));
    assert!(!display.should_display(&ctx_disabled));

    let ctx_maintenance = DisplayContext::new()
        .with_value("disabled", json!(false))
        .with_value("maintenance", json!(true));
    assert!(!display.should_display(&ctx_maintenance));
}

// ---------------------------------------------------------------------------
// 3. dependencies() returns all referenced field keys
// ---------------------------------------------------------------------------

#[test]
fn single_rule_dependencies() {
    let rule_set = DisplayRuleSet::Single(DisplayRule {
        field: "mode".into(),
        condition: DisplayCondition::IsSet,
    });

    assert_eq!(rule_set.dependencies(), vec!["mode"]);
}

#[test]
fn nested_dependencies_deduplicated_and_sorted() {
    let rule_set = DisplayRuleSet::All {
        rules: vec![
            DisplayRuleSet::Single(DisplayRule {
                field: "z_field".into(),
                condition: DisplayCondition::IsTrue,
            }),
            DisplayRuleSet::Single(DisplayRule {
                field: "a_field".into(),
                condition: DisplayCondition::IsSet,
            }),
            DisplayRuleSet::Not {
                rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                    field: "z_field".into(),
                    condition: DisplayCondition::IsFalse,
                })),
            },
            DisplayRuleSet::Any {
                rules: vec![
                    DisplayRuleSet::Single(DisplayRule {
                        field: "m_field".into(),
                        condition: DisplayCondition::IsNotEmpty,
                    }),
                    DisplayRuleSet::Single(DisplayRule {
                        field: "a_field".into(),
                        condition: DisplayCondition::Equals {
                            value: json!("yes"),
                        },
                    }),
                ],
            },
        ],
    };

    let deps = rule_set.dependencies();
    assert_eq!(deps, vec!["a_field", "m_field", "z_field"]);
}

#[test]
fn parameter_display_dependencies_from_show_and_hide() {
    let display = ParameterDisplay {
        show_when: vec![
            DisplayRuleSet::Single(DisplayRule {
                field: "mode".into(),
                condition: DisplayCondition::IsSet,
            }),
            DisplayRuleSet::Single(DisplayRule {
                field: "enabled".into(),
                condition: DisplayCondition::IsTrue,
            }),
        ],
        hide_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "locked".into(),
            condition: DisplayCondition::IsTrue,
        })],
    };

    let deps = display.dependencies();
    assert_eq!(deps, vec!["enabled", "locked", "mode"]);
}

#[test]
fn empty_display_has_no_dependencies() {
    let display = ParameterDisplay::default();
    assert!(display.dependencies().is_empty());
}

// ---------------------------------------------------------------------------
// 4. Display conditions with different value types
// ---------------------------------------------------------------------------

#[test]
fn condition_with_string_value() {
    let rule = DisplayRule {
        field: "format".into(),
        condition: DisplayCondition::Equals {
            value: json!("json"),
        },
    };

    let ctx_match = DisplayContext::new().with_value("format", json!("json"));
    assert!(rule.evaluate(&ctx_match));

    let ctx_no_match = DisplayContext::new().with_value("format", json!("xml"));
    assert!(!rule.evaluate(&ctx_no_match));
}

#[test]
fn condition_with_number_value() {
    let rule = DisplayRule {
        field: "count".into(),
        condition: DisplayCondition::GreaterThan { value: 5.0 },
    };

    let ctx_above = DisplayContext::new().with_value("count", json!(10));
    assert!(rule.evaluate(&ctx_above));

    let ctx_below = DisplayContext::new().with_value("count", json!(3));
    assert!(!rule.evaluate(&ctx_below));

    let ctx_equal = DisplayContext::new().with_value("count", json!(5));
    assert!(!rule.evaluate(&ctx_equal));
}

#[test]
fn condition_with_bool_value() {
    let rule_true = DisplayRule {
        field: "active".into(),
        condition: DisplayCondition::IsTrue,
    };
    let rule_false = DisplayRule {
        field: "active".into(),
        condition: DisplayCondition::IsFalse,
    };

    let ctx_true = DisplayContext::new().with_value("active", json!(true));
    assert!(rule_true.evaluate(&ctx_true));
    assert!(!rule_false.evaluate(&ctx_true));

    let ctx_false = DisplayContext::new().with_value("active", json!(false));
    assert!(!rule_true.evaluate(&ctx_false));
    assert!(rule_false.evaluate(&ctx_false));
}

#[test]
fn condition_with_array_value() {
    let rule_contains = DisplayRule {
        field: "tags".into(),
        condition: DisplayCondition::Contains {
            value: json!("important"),
        },
    };

    let ctx_has = DisplayContext::new().with_value("tags", json!(["important", "urgent"]));
    assert!(rule_contains.evaluate(&ctx_has));

    let ctx_missing = DisplayContext::new().with_value("tags", json!(["normal"]));
    assert!(!rule_contains.evaluate(&ctx_missing));

    let rule_not_empty = DisplayRule {
        field: "tags".into(),
        condition: DisplayCondition::IsNotEmpty,
    };

    let ctx_non_empty = DisplayContext::new().with_value("tags", json!(["a"]));
    assert!(rule_not_empty.evaluate(&ctx_non_empty));

    let ctx_empty = DisplayContext::new().with_value("tags", json!([]));
    assert!(!rule_not_empty.evaluate(&ctx_empty));
}

#[test]
fn condition_with_null_value() {
    let ctx_null = DisplayContext::new().with_value("field", json!(null));
    let ctx_missing = DisplayContext::new();

    let rule_is_null = DisplayRule {
        field: "field".into(),
        condition: DisplayCondition::IsNull,
    };
    assert!(rule_is_null.evaluate(&ctx_null));
    // Missing field treated as null.
    assert!(rule_is_null.evaluate(&ctx_missing));

    let rule_is_set = DisplayRule {
        field: "field".into(),
        condition: DisplayCondition::IsSet,
    };
    assert!(!rule_is_set.evaluate(&ctx_null));
    assert!(!rule_is_set.evaluate(&ctx_missing));
}

#[test]
fn in_range_with_boundary_values() {
    let rule = DisplayRule {
        field: "rating".into(),
        condition: DisplayCondition::InRange { min: 1.0, max: 5.0 },
    };

    assert!(rule.evaluate(&DisplayContext::new().with_value("rating", json!(1))));
    assert!(rule.evaluate(&DisplayContext::new().with_value("rating", json!(3))));
    assert!(rule.evaluate(&DisplayContext::new().with_value("rating", json!(5))));
    assert!(!rule.evaluate(&DisplayContext::new().with_value("rating", json!(0))));
    assert!(!rule.evaluate(&DisplayContext::new().with_value("rating", json!(6))));
    assert!(!rule.evaluate(&DisplayContext::new().with_value("rating", json!("not a number"))));
}

#[test]
fn one_of_with_mixed_types() {
    let rule = DisplayRule {
        field: "value".into(),
        condition: DisplayCondition::OneOf {
            values: vec![json!("auto"), json!(0), json!(false)],
        },
    };

    assert!(rule.evaluate(&DisplayContext::new().with_value("value", json!("auto"))));
    assert!(rule.evaluate(&DisplayContext::new().with_value("value", json!(0))));
    assert!(rule.evaluate(&DisplayContext::new().with_value("value", json!(false))));
    assert!(!rule.evaluate(&DisplayContext::new().with_value("value", json!("manual"))));
    assert!(!rule.evaluate(&DisplayContext::new().with_value("value", json!(1))));
}

#[test]
fn starts_with_and_ends_with() {
    let rule_starts = DisplayRule {
        field: "url".into(),
        condition: DisplayCondition::StartsWith {
            prefix: "https://".into(),
        },
    };
    let rule_ends = DisplayRule {
        field: "file".into(),
        condition: DisplayCondition::EndsWith {
            suffix: ".json".into(),
        },
    };

    assert!(
        rule_starts
            .evaluate(&DisplayContext::new().with_value("url", json!("https://example.com")))
    );
    assert!(
        !rule_starts
            .evaluate(&DisplayContext::new().with_value("url", json!("http://example.com")))
    );

    assert!(rule_ends.evaluate(&DisplayContext::new().with_value("file", json!("config.json"))));
    assert!(!rule_ends.evaluate(&DisplayContext::new().with_value("file", json!("config.yaml"))));
}

#[test]
fn is_valid_uses_validation_state() {
    let rule = DisplayRule {
        field: "email".into(),
        condition: DisplayCondition::IsValid,
    };

    let ctx_valid = DisplayContext::new().with_validation("email", true);
    assert!(rule.evaluate(&ctx_valid));

    let ctx_invalid = DisplayContext::new().with_validation("email", false);
    assert!(!rule.evaluate(&ctx_invalid));

    let ctx_unknown = DisplayContext::new();
    assert!(!rule.evaluate(&ctx_unknown));
}

// ---------------------------------------------------------------------------
// 5. Serde round-trip of display rules
// ---------------------------------------------------------------------------

#[test]
fn complex_display_rules_serde_round_trip() {
    let display = ParameterDisplay {
        show_when: vec![DisplayRuleSet::All {
            rules: vec![
                DisplayRuleSet::Single(DisplayRule {
                    field: "enabled".into(),
                    condition: DisplayCondition::IsTrue,
                }),
                DisplayRuleSet::Any {
                    rules: vec![
                        DisplayRuleSet::Single(DisplayRule {
                            field: "count".into(),
                            condition: DisplayCondition::GreaterThan { value: 10.0 },
                        }),
                        DisplayRuleSet::Not {
                            rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                                field: "mode".into(),
                                condition: DisplayCondition::Equals {
                                    value: json!("disabled"),
                                },
                            })),
                        },
                    ],
                },
            ],
        }],
        hide_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "hidden".into(),
            condition: DisplayCondition::IsTrue,
        })],
    };

    let json_str = serde_json::to_string_pretty(&display).unwrap();
    let restored: ParameterDisplay = serde_json::from_str(&json_str).unwrap();

    assert_eq!(display, restored);
}
