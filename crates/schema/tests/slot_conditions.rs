//! Conditions must refer to the exact admitted public input graph.

use nebula_schema::{InputContract, SchemaGraphDocument};
use nebula_validator::{Condition, Predicate, Rule, foundation::FieldPath};
use serde_json::{Value, json};

fn input() -> InputContract {
    let flag = json!({"key":"flag", "target":"bool", "null":"reject", "presence":"required", "aliases":{"read":["old_flag"]}});
    let root = json!({"target":"root", "null":"reject"});
    let wire = json!({
        "version":3, "root":root,
        "definitions":[
            {"key":"root", "body":{"kind":"record", "additional_properties":"closed", "properties":[flag,
                {"key":"label", "target":"text", "null":"allow", "presence":"optional"},
                {"key":"counter", "target":"int", "null":"reject", "presence":"required"}
            ]}},
            {"key":"bool", "body":{"kind":"boolean"}},
            {"key":"text", "body":{"kind":"string"}},
            {"key":"int", "body":{"kind":"integer"}}
        ],
        "x-nebula-conditions":{
            "enabled":{"is_true":"/old_flag"}
        }
    });
    let graph = serde_json::from_value::<SchemaGraphDocument>(wire)
        .unwrap()
        .admit()
        .unwrap();
    InputContract::from_graph(&graph).unwrap()
}

fn condition(predicate: Predicate) -> Condition {
    Condition::try_from(Rule::predicate(predicate).unwrap()).unwrap()
}

#[test]
fn named_alias_is_canonical_before_prepared_snapshot_evaluation() {
    let input = input();
    let condition = input.named_condition("enabled").unwrap();
    assert_eq!(
        condition.predicates().next().unwrap().field().as_str(),
        "/flag"
    );
    input.admit_condition(&condition).unwrap();
    let prepared = input
        .validate_data(json!({"old_flag":true,"counter":1}))
        .unwrap();
    assert!(condition.matches(prepared.predicate_context()).unwrap());
    assert!(input.named_condition("missing").is_err());
}

#[test]
fn inline_alias_must_be_canonicalized_before_admission() {
    let input = input();
    let authored = condition(Predicate::IsTrue(FieldPath::single("old_flag")));
    assert!(input.admit_condition(&authored).is_err());
    let canonical = input.canonical_condition(&authored).unwrap();
    input.admit_condition(&canonical).unwrap();
}

#[test]
fn incompatible_and_undeclared_predicates_never_become_runtime_false() {
    let input = input();
    for predicate in [
        Predicate::IsTrue(FieldPath::single("label")),
        Predicate::Eq(FieldPath::single("flag"), json!("true")),
        Predicate::IsTrue(FieldPath::single("missing")),
        Predicate::Eq(FieldPath::root(), json!(null)),
        Predicate::Eq(FieldPath::single("counter"), json!(0.5)),
        Predicate::In(FieldPath::single("flag"), vec![json!(true), json!("false")]),
    ] {
        assert!(input.admit_condition(&condition(predicate)).is_err());
    }
    input
        .admit_condition(&condition(Predicate::Eq(
            FieldPath::single("label"),
            Value::Null,
        )))
        .unwrap();
    assert!(
        input
            .admit_condition(&condition(Predicate::Eq(
                FieldPath::single("flag"),
                Value::Null
            )))
            .is_err()
    );
}

#[test]
fn every_logic_branch_is_admitted_even_with_a_known_boolean_alternative() {
    let input = input();
    let condition = Condition::try_from(
        Rule::any([
            Rule::predicate(Predicate::IsTrue(FieldPath::single("flag"))).unwrap(),
            Rule::not(Rule::predicate(Predicate::IsTrue(FieldPath::single("missing"))).unwrap())
                .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    assert!(input.admit_condition(&condition).is_err());
}

#[test]
fn alias_protection_cannot_be_bypassed_by_a_public_terminal_definition() {
    let wire = json!({
        "version":3,"root":{"target":"root","null":"reject"},
        "definitions":[
            {"key":"root","body":{"kind":"record","additional_properties":"closed","properties":[
                {"key":"token","target":"protected","null":"reject","presence":"required"}
            ]}},
            {"key":"protected","body":{"kind":"alias","alias":{
                "target":"text","null":"reject","protection":"secret_utf8"
            }}},
            {"key":"text","body":{"kind":"string"}}
        ]
    });
    let graph = serde_json::from_value::<SchemaGraphDocument>(wire)
        .unwrap()
        .admit()
        .unwrap();
    let input = InputContract::from_graph(&graph).unwrap();
    let condition = condition(Predicate::Eq(
        FieldPath::single("token"),
        json!("never-render-this"),
    ));
    let report = input.admit_condition(&condition).unwrap_err();
    assert!(!format!("{report:?}").contains("never-render-this"));
}

#[test]
fn named_table_cannot_bypass_the_condition_inventory_budget() {
    let mut document = serde_json::to_value(input().graph().to_document()).unwrap();
    let table = document["x-nebula-conditions"].as_object_mut().unwrap();
    for index in 0..=nebula_validator::MAX_RULE_NODES {
        table.insert(format!("condition_{index}"), json!({"is_true":"/flag"}));
    }
    let graph = serde_json::from_value::<SchemaGraphDocument>(document)
        .unwrap()
        .admit()
        .unwrap();
    assert!(InputContract::from_graph(&graph).is_err());
}

#[test]
fn invalid_named_declarations_are_rejected_before_contract_compilation() {
    for declaration in [json!({"is_true":"/missing"}), json!({"min_length":1})] {
        let mut document = serde_json::to_value(input().graph().to_document()).unwrap();
        document["x-nebula-conditions"]["invalid"] = declaration;
        let graph = serde_json::from_value::<SchemaGraphDocument>(document)
            .unwrap()
            .admit()
            .unwrap();
        assert!(InputContract::from_graph(&graph).is_err());
    }
}

fn nested_document(local_rule: Value) -> Value {
    json!({
        "version":3, "root":{"target":"root", "null":"reject"},
        "definitions":[
            {"key":"root","body":{"kind":"record","additional_properties":"closed","properties":[
                {"key":"flag","target":"bool","null":"reject","presence":"required"},
                {"key":"child","target":"nested","null":"reject","presence":"required"}
            ]}},
            {"key":"nested","body":{"kind":"record","additional_properties":"closed","properties":[
                {"key":"flag","target":"text","null":"reject","presence":"required","aliases":{"read":["legacy"]}}
            ]}},
            {"key":"text","body":{"kind":"string"}},
            {"key":"bool","body":{"kind":"boolean"}}
        ],
        "x-nebula-local-conditions":{"nested":{"ready":local_rule}}
    })
}

fn compile_document(document: Value) -> Result<InputContract, nebula_schema::ValidationReport> {
    let graph = serde_json::from_value::<SchemaGraphDocument>(document)
        .unwrap()
        .admit()
        .unwrap();
    InputContract::from_graph(&graph)
}

#[test]
fn unused_local_conditions_are_checked_in_the_declaring_type_scope() {
    assert!(compile_document(nested_document(json!({"eq":["/legacy","ready"]}))).is_ok());
    // The outer field has boolean type, but the local field is string. Checking
    // only the outer root would incorrectly accept this unused declaration.
    assert!(compile_document(nested_document(json!({"is_true":"/flag"}))).is_err());
    assert!(compile_document(nested_document(json!({"is_true":"/absent"}))).is_err());
    let mut unknown = nested_document(json!({"eq":["/legacy","ready"]}));
    unknown["x-nebula-local-conditions"]["missing_anchor"] = json!({});
    assert!(compile_document(unknown).is_err());
}

#[test]
fn all_scoped_behavior_is_committed_after_alias_canonicalization() {
    let alias = compile_document(nested_document(json!({"eq":["/legacy","ready"]}))).unwrap();
    let canonical = compile_document(nested_document(json!({"eq":["/flag","ready"]}))).unwrap();
    let changed = compile_document(nested_document(json!({"eq":["/flag","different"]}))).unwrap();
    assert_eq!(alias.semantic_commitment(), canonical.semantic_commitment());
    assert_ne!(
        canonical.semantic_commitment(),
        changed.semantic_commitment()
    );
    assert_eq!(
        alias.record().readmit_input().unwrap().record(),
        alias.record()
    );
}

#[test]
fn local_inventory_budget_is_global_across_scopes() {
    let mut document = nested_document(json!({"eq":["/flag","ready"]}));
    let scopes = document["x-nebula-local-conditions"]
        .as_object_mut()
        .unwrap();
    let nested = scopes.get_mut("nested").unwrap().as_object_mut().unwrap();
    for index in 1..nebula_validator::MAX_RULE_NODES {
        nested.insert(
            format!("condition_{index}"),
            json!({"eq":["/flag","ready"]}),
        );
    }
    scopes.insert("root".to_owned(), json!({"extra":{"is_true":"/flag"}}));
    assert!(compile_document(document).is_err());
}
