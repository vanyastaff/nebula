use nebula_validator::{
    DeferredRule, DiagnosticDisclosure, ExecutionMode, MAX_RULE_DEPTH, MAX_RULE_JSON_DEPTH,
    MAX_RULE_JSON_NODES, MAX_RULE_NODES, MAX_RULE_OPERANDS, MAX_RULE_TEXT_BYTES, Predicate,
    PredicateContext, Rule, RuleBuildError, RuleOperands, RulePattern, RuleView, ValueRule,
    foundation::{FieldPath, Validate},
};
use serde_json::{Value, json};

fn not_chain(depth: usize) -> Rule {
    let mut rule = Rule::email();
    for _ in 1..depth {
        rule = Rule::not(rule).unwrap();
    }
    rule
}

fn nested_not_json(depth: usize) -> String {
    let mut wire = String::with_capacity(depth.saturating_mul(8));
    for _ in 1..depth {
        wire.push_str("{\"not\":");
    }
    wire.push_str("\"email\"");
    for _ in 1..depth {
        wire.push('}');
    }
    wire
}

fn nested_json_value(depth: usize) -> Value {
    let mut value = Value::Null;
    for _ in 1..depth {
        value = Value::Array(vec![value]);
    }
    value
}

fn nested_json_wire(depth: usize) -> String {
    let mut wire = String::from("{\"one_of\":[");
    for _ in 1..depth {
        wire.push('[');
    }
    wire.push_str("null");
    for _ in 1..depth {
        wire.push(']');
    }
    wire.push_str("]}");
    wire
}

fn large_numeric_array_wire(nodes: usize) -> String {
    let mut wire = String::from("{\"one_of\":[[");
    for index in 0..nodes {
        if index != 0 {
            wire.push(',');
        }
        wire.push('0');
    }
    wire.push_str("]]}");
    wire
}

fn large_numeric_in_wire(nodes: usize) -> String {
    let mut wire = String::from("{\"in\":[\"/candidate\",[[");
    for index in 0..nodes {
        if index != 0 {
            wire.push(',');
        }
        wire.push('0');
    }
    wire.push_str("]]]}");
    wire
}

fn large_numeric_object_wire(nodes: usize) -> String {
    let mut wire = String::from("{\"one_of\":[{");
    for index in 0..nodes {
        if index != 0 {
            wire.push(',');
        }
        wire.push('"');
        wire.push_str(&index.to_string());
        wire.push_str("\":0");
    }
    wire.push_str("}]}");
    wire
}

#[test]
fn depth_limit_accepts_boundary_and_rejects_one_more() {
    let boundary = not_chain(MAX_RULE_DEPTH);
    assert_eq!(boundary.check_limits(), Ok(()));
    assert_eq!(
        Rule::not(boundary).unwrap_err(),
        RuleBuildError::DepthLimit {
            limit: MAX_RULE_DEPTH,
        }
    );
}

#[test]
fn node_limit_accepts_boundary_and_rejects_one_more() {
    let boundary = Rule::all(vec![Rule::email(); MAX_RULE_NODES - 1]).unwrap();
    assert_eq!(boundary.check_limits(), Ok(()));
    assert_eq!(
        Rule::all(vec![Rule::email(); MAX_RULE_NODES]).unwrap_err(),
        RuleBuildError::NodeLimit {
            limit: MAX_RULE_NODES,
        }
    );
}

#[test]
fn operand_limit_rejects_large_value_set() {
    let values = vec![json!(false); MAX_RULE_OPERANDS + 1];
    let error = Rule::one_of(values).unwrap_err();
    assert_eq!(
        error,
        RuleBuildError::OperandLimit {
            limit: MAX_RULE_OPERANDS,
        }
    );
}

#[test]
fn text_limit_is_byte_exact_and_redacted() {
    let accepted = Rule::custom("x".repeat(MAX_RULE_TEXT_BYTES));
    assert!(accepted.is_ok());

    let secret = "s".repeat(MAX_RULE_TEXT_BYTES + 1);
    let error = Rule::custom(secret.clone()).unwrap_err();
    assert_eq!(
        error,
        RuleBuildError::TextLimit {
            limit: MAX_RULE_TEXT_BYTES,
        }
    );
    assert!(!error.to_string().contains(&secret));
    assert!(!format!("{error:?}").contains(&secret));
}

#[test]
fn recursive_constructors_reject_before_building_an_overdeep_rule() {
    let inner = not_chain(MAX_RULE_DEPTH);
    assert_eq!(
        Rule::not(inner).unwrap_err(),
        RuleBuildError::DepthLimit {
            limit: MAX_RULE_DEPTH,
        }
    );
}

#[test]
fn hostile_deserialization_stops_at_rule_depth_limit() {
    let error = serde_json::from_str::<Rule>(&nested_not_json(MAX_RULE_DEPTH + 1)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("rule depth limit"), "{message}");
    assert!(message.contains(&MAX_RULE_DEPTH.to_string()), "{message}");
}

#[test]
fn deserialization_rejects_aggregate_text_and_operands() {
    let text_wire = serde_json::to_string(&json!({
        "custom": "x".repeat(MAX_RULE_TEXT_BYTES + 1)
    }))
    .unwrap();
    let text_error = serde_json::from_str::<Rule>(&text_wire).unwrap_err();
    assert!(text_error.to_string().contains("rule text limit"));

    let operand_wire = serde_json::to_string(&json!({
        "one_of": vec![false; MAX_RULE_OPERANDS + 1]
    }))
    .unwrap();
    let operand_error = serde_json::from_str::<Rule>(&operand_wire).unwrap_err();
    assert!(operand_error.to_string().contains("rule operand limit"));
}

#[test]
fn json_depth_limit_accepts_boundary_and_rejects_one_more_on_both_paths() {
    assert!(Rule::one_of([nested_json_value(MAX_RULE_JSON_DEPTH)]).is_ok());
    assert_eq!(
        Rule::one_of([nested_json_value(MAX_RULE_JSON_DEPTH + 1)]).unwrap_err(),
        RuleBuildError::JsonDepthLimit {
            limit: MAX_RULE_JSON_DEPTH,
        }
    );

    assert!(serde_json::from_str::<Rule>(&nested_json_wire(MAX_RULE_JSON_DEPTH)).is_ok());
    let error =
        serde_json::from_str::<Rule>(&nested_json_wire(MAX_RULE_JSON_DEPTH + 1)).unwrap_err();
    assert!(error.to_string().contains("rule JSON depth limit"));
}

#[test]
fn serde_rejects_large_numeric_array_and_object_operands() {
    for wire in [
        large_numeric_array_wire(100_000),
        large_numeric_object_wire(100_000),
        large_numeric_in_wire(100_000),
    ] {
        let error = serde_json::from_str::<Rule>(&wire).unwrap_err();
        assert!(
            error.to_string().contains("rule JSON node limit"),
            "{error}"
        );
    }
}

#[test]
fn constructors_enforce_the_same_aggregate_json_node_limit() {
    let array = Value::Array(vec![Value::Null; MAX_RULE_JSON_NODES]);
    assert_eq!(
        Rule::one_of([array]).unwrap_err(),
        RuleBuildError::JsonNodeLimit {
            limit: MAX_RULE_JSON_NODES,
        }
    );

    let object = (0..MAX_RULE_JSON_NODES)
        .map(|index| (index.to_string(), Value::Null))
        .collect();
    assert_eq!(
        Rule::predicate(Predicate::Eq(
            FieldPath::single("candidate"),
            Value::Object(object),
        ))
        .unwrap_err(),
        RuleBuildError::JsonNodeLimit {
            limit: MAX_RULE_JSON_NODES,
        }
    );

    let array = Value::Array(vec![Value::Null; MAX_RULE_JSON_NODES]);
    assert_eq!(
        Rule::predicate(Predicate::In(FieldPath::single("candidate"), vec![array],)).unwrap_err(),
        RuleBuildError::JsonNodeLimit {
            limit: MAX_RULE_JSON_NODES,
        }
    );
}

#[test]
fn rejected_deep_constructor_operand_drops_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1_024)
        .spawn(|| {
            let value = nested_json_value(100_000);
            assert_eq!(
                Rule::predicate(Predicate::Eq(FieldPath::single("candidate"), value)).unwrap_err(),
                RuleBuildError::JsonDepthLimit {
                    limit: MAX_RULE_JSON_DEPTH,
                }
            );
        })
        .expect("hostile-operand regression thread must start")
        .join()
        .expect("rejected operand must drop without recursion");
}

#[test]
fn operand_limit_rejection_drops_current_and_uninspected_values_iteratively() {
    std::thread::Builder::new()
        .stack_size(128 * 1_024)
        .spawn(|| {
            let mut values = vec![Value::Null; MAX_RULE_OPERANDS];
            values.push(nested_json_value(100_000));
            values.push(nested_json_value(100_000));
            assert_eq!(
                Rule::one_of(values).unwrap_err(),
                RuleBuildError::OperandLimit {
                    limit: MAX_RULE_OPERANDS,
                }
            );
        })
        .expect("operand-limit regression thread must start")
        .join()
        .expect("all rejected operands must drop without recursion");
}

#[test]
fn extra_tuple_element_is_rejected_without_traversing_it() {
    let mut wire = String::from("{\"eq\":[\"/candidate\",0,");
    wire.push_str(&"[".repeat(100_000));
    let error = serde_json::from_str::<Rule>(&wire).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("rule tuple must contain exactly two elements"),
        "{error}"
    );
}

#[test]
fn public_rule_debug_graph_is_payload_free() {
    const SECRET: &str = "RULE_DEBUG_SECRET_4cc174";

    let pattern = RulePattern::new(SECRET).unwrap();
    let predicate = Predicate::Eq(FieldPath::single(SECRET), json!({(SECRET): SECRET}));
    let value_rule = ValueRule::OneOf(vec![json!({(SECRET): SECRET})]);
    let deferred = DeferredRule::Custom(SECRET.to_owned());
    let operands = RuleOperands::from([json!({(SECRET): SECRET})]);
    let composite = Rule::all([
        Rule::predicate(predicate.clone()).unwrap(),
        Rule::value(value_rule.clone()).unwrap(),
        Rule::custom(SECRET).unwrap(),
    ])
    .unwrap();
    let children = match composite.view() {
        RuleView::All(children) => children,
        other => panic!("expected all rule, got {other:?}"),
    };
    let child = children.clone().next().unwrap();
    let described = composite.clone().with_message(SECRET).unwrap();

    for formatted in [
        format!("{composite:?}"),
        format!("{:?}", composite.root()),
        format!("{:?}", composite.view()),
        format!("{children:?}"),
        format!("{child:?}"),
        format!("{described:?}"),
        format!("{predicate:?}"),
        format!("{value_rule:?}"),
        format!("{deferred:?}"),
        format!("{operands:?}"),
        format!("{pattern:?}"),
    ] {
        assert!(!formatted.contains(SECRET), "{formatted}");
    }
}

#[test]
fn omitted_predicate_diagnostics_hide_operands_and_descriptions() {
    const OPERAND_SECRET: &str = "RULE_OPERAND_SECRET_8dd641";
    const DESCRIPTION_SECRET: &str = "RULE_DESCRIPTION_SECRET_97d8a0";

    let rule = Rule::predicate(Predicate::Eq(
        FieldPath::single("candidate"),
        json!({"token": OPERAND_SECRET}),
    ))
    .unwrap()
    .with_message(format!(
        "{DESCRIPTION_SECRET}: expected {{expected}} from {{allowed}}"
    ))
    .unwrap();
    let context = PredicateContext::from_json(json!({"candidate": "different"}));
    let error = rule
        .validate(
            &Value::Null,
            Some(&context),
            ExecutionMode::Full,
            DiagnosticDisclosure::OmitValue,
        )
        .unwrap_err();

    assert!(error.params().is_empty());
    assert_eq!(error.message, "predicate failed");
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(OPERAND_SECRET), "{rendered}");
    assert!(!rendered.contains(DESCRIPTION_SECRET), "{rendered}");
}

#[test]
fn implicit_validate_trait_path_omits_the_rejected_value() {
    const INPUT_SECRET: &str = "RULE_INPUT_SECRET_17fbc9";
    const DESCRIPTION_SECRET: &str = "RULE_DESCRIPTION_SECRET_571e09";

    let rule = Rule::min_length(100)
        .with_message(format!("{DESCRIPTION_SECRET}: {{value}}"))
        .unwrap();
    let error = Validate::validate(&rule, &json!(INPUT_SECRET)).unwrap_err();
    assert_eq!(error.param("value"), None);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(INPUT_SECRET), "{rendered}");
    assert!(!rendered.contains(DESCRIPTION_SECRET), "{rendered}");
}
