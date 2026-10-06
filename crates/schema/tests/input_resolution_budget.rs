//! Graph-bound input preparation and resolution enforce the overall value
//! budget incrementally, before hostile growth is materialized.

use nebula_schema::{
    AdmittedSchemaGraph, AuthoredValue, CompiledProgram, EvalFuture, Expression, ExpressionContext,
    InputContract, MAX_VALUE_TEXT_BYTES, SchemaGraphDocument, ValidationReport, ValueTree,
};
use serde_json::{Value, json};

fn graph(root: Value, definitions: Value) -> AdmittedSchemaGraph {
    serde_json::from_value::<SchemaGraphDocument>(
        json!({"version":3,"root":root,"definitions":definitions}),
    )
    .unwrap()
    .admit()
    .unwrap()
}

fn codes(report: &ValidationReport) -> Vec<String> {
    report
        .errors()
        .map(|error| format!("{}@{}", error.code(), error.path()))
        .collect()
}

/// More than half of the text budget: two copies cannot coexist.
fn large_text() -> String {
    "x".repeat(MAX_VALUE_TEXT_BYTES / 2 + 1)
}

#[test]
fn repeated_record_defaults_fail_before_later_items_are_prepared() {
    // Each list item is a record whose `blob` default is admissible alone but
    // whose repetition exceeds the overall budget. The last item also carries
    // a literal where an expression is required: preparation that materializes
    // every default before checking the budget reaches that error instead.
    let items = graph(
        json!({"target":"items","null":"reject"}),
        json!([
            {"key":"items","body":{"kind":"array","element":{"target":"item","null":"reject"}}},
            {"key":"item","body":{"kind":"record","properties":[
                {"key":"blob","target":"text","presence":"optional","null":"reject","input_default":large_text()},
                {"key":"must","target":"number","presence":"optional","null":"reject","expression":"required"}
            ],"additional_properties":"closed"}},
            {"key":"text","body":{"kind":"string"}},
            {"key":"number","body":{"kind":"number"}}
        ]),
    );
    let input = InputContract::from_graph(&items).unwrap();
    let error = input
        .validate_data(json!([{}, {}, {"must": 1}]))
        .unwrap_err();
    assert_eq!(
        codes(&error),
        ["value.limit_exceeded@/1/blob"],
        "the second default must exhaust the budget before item 2 is prepared"
    );
    // A single default stays admissible.
    input.validate_data(json!([{}])).unwrap();
}

struct LargeResults;

impl ExpressionContext for LargeResults {
    fn evaluate<'a>(&'a self, program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move {
            if program.source().contains("fail") {
                Err(nebula_schema::ValidationError::builder("expression.runtime").build())
            } else {
                Ok(Value::String(large_text()))
            }
        })
    }
}

#[tokio::test]
async fn aggregate_expression_results_fail_before_later_programs_run() {
    // Each result is admissible alone; together they exceed the overall
    // budget. A third program fails at evaluation: resolution without an
    // aggregate budget evaluates it and reports that failure instead.
    let texts = graph(
        json!({"target":"texts","null":"reject"}),
        json!([
            {"key":"texts","body":{"kind":"array","element":{"target":"text","null":"reject","expression":"allowed"}}},
            {"key":"text","body":{"kind":"string"}}
        ]),
    );
    let input = InputContract::from_graph(&texts).unwrap();
    let expression =
        |source: &str| -> AuthoredValue { ValueTree::Expression(Expression::new(source)) };
    let prepared = input
        .validate(ValueTree::List(vec![
            expression("{{ $first }}"),
            expression("{{ $second }}"),
            expression("{{ $fail }}"),
        ]))
        .unwrap();
    let error = prepared.resolve(&LargeResults).await.unwrap_err();
    assert_eq!(codes(&error), ["value.limit_exceeded@/1"]);
}
