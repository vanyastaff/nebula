use nebula_core::node_key;
use nebula_schema::{FieldKey, PathWalk, Property, Schema, ValidSchema, ValuePath};
use proptest::prelude::*;
use serde_json::json;

use super::*;

fn make_resolver() -> ParamResolver {
    let engine = Arc::new(ExpressionEngine::new());
    ParamResolver::new(engine)
}

async fn resolved_data(
    schema: &ValidSchema,
    params: &HashMap<String, ParamValue>,
    input: serde_json::Value,
    outputs: &DashMap<NodeKey, serde_json::Value>,
) -> Result<serde_json::Value, EngineError> {
    let shared_outputs = DashMap::new();
    let resolved = make_resolver()
        .prepare(NodeInputRequest {
            node_key: &node_key!("test"),
            parameters: params,
            predecessor_input: input,
            outputs,
            shared_outputs: &shared_outputs,
            schema,
            cancellation: CancellationToken::new(),
        })?
        .resolve()
        .await?;
    Ok(resolved.into_typed_exposing_secrets().unwrap())
}

// -- resolve tests --

#[tokio::test]
async fn empty_params_preserve_predecessor_data() {
    let outputs = DashMap::new();
    let schema = nebula_schema::schema_of::<serde_json::Value>().unwrap();
    for input in [json!(null), json!(7), json!({"literal": "{{ 7 }}"})] {
        let result = resolved_data(&schema, &HashMap::new(), input.clone(), &outputs)
            .await
            .unwrap();
        assert_eq!(result, input);
    }
}

#[tokio::test]
async fn literal_resolution_passthrough() {
    let schema = Schema::builder()
        .property(Property::string(FieldKey::new("url").unwrap()))
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert(
        "url".to_owned(),
        ParamValue::literal(json!("https://example.com")),
    );

    let result = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap();
    assert_eq!(result["url"], json!("https://example.com"));
}

#[tokio::test]
async fn expression_resolution_evaluates() {
    let schema = Schema::builder()
        .property(Property::number(FieldKey::new("count").unwrap()).integer())
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert(
        "count".to_owned(),
        ParamValue::expression("$input.count + 1"),
    );

    let input = json!({"count": 5});
    let result = resolved_data(&schema, &params, input, &outputs)
        .await
        .unwrap();
    assert_eq!(result["count"], json!(6));
}

#[tokio::test]
async fn expression_resolution_observes_cancellation_before_evaluation() {
    let schema = Schema::builder()
        .property(Property::number(FieldKey::new("count").unwrap()).integer())
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let shared_outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert("count".to_owned(), ParamValue::expression("1 + 1"));
    let cancellation = CancellationToken::new();
    let prepared = make_resolver()
        .prepare(NodeInputRequest {
            node_key: &node_key!("test"),
            parameters: &params,
            predecessor_input: json!(null),
            outputs: &outputs,
            shared_outputs: &shared_outputs,
            schema: &schema,
            cancellation: cancellation.clone(),
        })
        .unwrap();
    cancellation.cancel();

    let error = prepared.resolve().await.unwrap_err();

    std::assert_matches!(error, EngineError::Cancelled);
}

#[test]
fn expression_snapshot_rejects_deep_output_before_admission() {
    let schema = nebula_schema::schema_of::<serde_json::Value>().unwrap();
    let source_id = node_key!("source");
    let outputs = DashMap::new();
    let mut nested = json!(null);
    for _ in 0..300 {
        nested = json!([nested]);
    }
    outputs.insert(source_id, nested);
    let shared_outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert("value".to_owned(), ParamValue::expression("$node.source"));

    let error = make_resolver()
        .prepare(NodeInputRequest {
            node_key: &node_key!("test"),
            parameters: &params,
            predecessor_input: json!(null),
            outputs: &outputs,
            shared_outputs: &shared_outputs,
            schema: &schema,
            cancellation: CancellationToken::new(),
        })
        .unwrap_err();

    std::assert_matches!(error, EngineError::ParameterResolution { .. });
}

#[test]
fn expression_snapshot_rejects_aggregate_before_populating_shared_cache() {
    let schema = nebula_schema::schema_of::<serde_json::Value>().unwrap();
    let outputs = DashMap::new();
    outputs.insert(node_key!("first"), json!("a".repeat(600_000)));
    outputs.insert(node_key!("second"), json!("b".repeat(600_000)));
    let shared_outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert("value".to_owned(), ParamValue::expression("$node.first"));

    let error = make_resolver()
        .prepare(NodeInputRequest {
            node_key: &node_key!("test"),
            parameters: &params,
            predecessor_input: json!(null),
            outputs: &outputs,
            shared_outputs: &shared_outputs,
            schema: &schema,
            cancellation: CancellationToken::new(),
        })
        .unwrap_err();

    std::assert_matches!(error, EngineError::ParameterResolution { .. });
    assert!(shared_outputs.is_empty());
}

#[tokio::test]
async fn template_resolution_renders() {
    let schema = Schema::builder()
        .property(Property::string(FieldKey::new("greeting").unwrap()))
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert(
        "greeting".to_owned(),
        ParamValue::template("Hello {{ $input.name }}!"),
    );

    let input = json!({"name": "World"});
    let result = resolved_data(&schema, &params, input, &outputs)
        .await
        .unwrap();
    assert_eq!(result["greeting"], json!("Hello World!"));
}

#[tokio::test]
async fn reference_resolution_looks_up_output() {
    let schema = Schema::builder()
        .property(
            Property::object(FieldKey::new("input").unwrap())
                .property(Property::string(FieldKey::new("data").unwrap())),
        )
        .build()
        .unwrap();
    let source_id = node_key!("source");
    let outputs = DashMap::new();
    outputs.insert(source_id.clone(), json!({"data": "fetched"}));

    let mut params = HashMap::new();
    params.insert("input".to_owned(), ParamValue::root_reference(source_id));

    let result = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap();
    assert_eq!(result["input"], json!({"data": "fetched"}));
}

#[tokio::test]
async fn reference_with_path_navigates_output() {
    let schema = Schema::builder()
        .property(Property::number(FieldKey::new("val").unwrap()).integer())
        .build()
        .unwrap();
    let source_id = node_key!("source");
    let outputs = DashMap::new();
    outputs.insert(source_id.clone(), json!({"nested": {"value": 42}}));

    let mut params = HashMap::new();
    params.insert(
        "val".to_owned(),
        ParamValue::reference(source_id, ValuePath::from_pointer("/nested/value").unwrap()),
    );

    let result = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap();
    assert_eq!(result["val"], json!(42));
}

#[test]
fn reference_path_uses_rfc6901_escaping() {
    let source_id = node_key!("source");
    let outputs = DashMap::new();
    outputs.insert(source_id.clone(), json!({"a/b": {"~key": 42}}));
    let parameter =
        ParamValue::reference(source_id, ValuePath::from_pointer("/a~1b/~0key").unwrap());

    let authored =
        ParamResolver::author_parameter(&node_key!("consumer"), "val", &parameter, &outputs)
            .unwrap();
    let AuthoredValue::Literal(value) = authored else {
        panic!("expected literal reference output");
    };
    assert_eq!(value.as_json(), &json!(42));
}

#[test]
fn reference_to_explicit_null_preserves_null() {
    let source_id = node_key!("source");
    let outputs = DashMap::new();
    outputs.insert(source_id.clone(), json!({"nested": null}));
    let parameter = ParamValue::reference(source_id, ValuePath::from_pointer("/nested").unwrap());

    let authored =
        ParamResolver::author_parameter(&node_key!("consumer"), "val", &parameter, &outputs)
            .unwrap();
    let AuthoredValue::Literal(value) = authored else {
        panic!("expected literal reference output");
    };
    assert_eq!(value.as_json(), &serde_json::Value::Null);
}

fn assert_reference_path_error(output: serde_json::Value, output_path: &str) {
    let source_id = node_key!("source");
    let outputs = DashMap::new();
    outputs.insert(source_id.clone(), output);
    let parameter = ParamValue::reference(source_id, ValuePath::from_pointer(output_path).unwrap());

    let error =
        ParamResolver::author_parameter(&node_key!("consumer"), "val", &parameter, &outputs)
            .unwrap_err();
    let EngineError::ParameterResolution {
        node_key,
        param_key,
        error: detail,
        source,
    } = error
    else {
        panic!("expected ParameterResolution, got {error:?}");
    };
    assert_eq!(node_key, node_key!("consumer"));
    assert_eq!(param_key, "val");
    assert!(detail.contains(output_path), "unexpected detail: {detail}");
    assert!(source.is_none());
}

#[test]
fn reference_to_missing_key_returns_parameter_resolution_error() {
    assert_reference_path_error(json!({"nested": 42}), "/missing");
}

#[test]
fn reference_to_bad_array_index_returns_parameter_resolution_error() {
    assert_reference_path_error(json!({"items": [1]}), "/items/5");
}

#[test]
fn reference_descending_through_scalar_returns_parameter_resolution_error() {
    assert_reference_path_error(json!({"scalar": 42}), "/scalar/value");
}

#[tokio::test]
async fn reference_to_missing_node_returns_error() {
    let schema = Schema::builder()
        .property(Property::object(FieldKey::new("data").unwrap()))
        .build()
        .unwrap();
    let missing_id = node_key!("missing");
    let outputs = DashMap::new();

    let mut params = HashMap::new();
    params.insert("data".to_owned(), ParamValue::root_reference(missing_id));

    let err = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap_err();
    std::assert_matches!(err, EngineError::ParameterResolution { .. });
    assert!(err.to_string().contains("has no output"));
}

#[tokio::test]
async fn expression_eval_failure_returns_error() {
    let schema = Schema::builder()
        .property(Property::number(FieldKey::new("bad").unwrap()))
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    // invalid expression: accessing property on undefined variable
    params.insert(
        "bad".to_owned(),
        ParamValue::expression("$nonexistent.foo.bar"),
    );

    let err = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap_err();
    std::assert_matches!(err, EngineError::ParameterResolution { .. });
}

#[tokio::test]
async fn template_parse_failure_returns_error() {
    let schema = Schema::builder()
        .property(Property::string(FieldKey::new("bad").unwrap()))
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    // Unclosed template delimiter
    params.insert("bad".to_owned(), ParamValue::template("Hello {{ unclosed"));

    let err = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap_err();
    std::assert_matches!(err, EngineError::ParameterResolution { .. });
}

// Typed validation sources remain available without publishing expression text.
#[tokio::test]
async fn expression_resolution_error_preserves_typed_source() {
    use std::error::Error as StdError;

    let schema = Schema::builder()
        .property(Property::number(FieldKey::new("bad").unwrap()))
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert(
        "bad".to_owned(),
        ParamValue::expression("$NONEXISTENT_SECRET_CANARY.foo"),
    );

    let err = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap_err();

    // The error must be the ParameterResolution variant with a typed source.
    let EngineError::ParameterResolution { ref source, .. } = err else {
        panic!("expected ParameterResolution, got {err:?}");
    };
    assert!(
        source.is_some(),
        "expression eval failure must carry a typed validation source, got None"
    );

    // The std::error::Error source chain must be intact.
    assert!(
        (&err as &dyn StdError).source().is_some(),
        "std::error::Error::source() must return Some(_) for expression failures"
    );
    let source = source.as_ref().unwrap();
    assert_eq!(source.code(), "input.validation");
    let mut cause: Option<&dyn StdError> = Some(&err);
    while let Some(error) = cause {
        assert!(!format!("{error} {error:?}").contains("NONEXISTENT_SECRET_CANARY"));
        cause = error.source();
    }
}

#[tokio::test]
async fn reference_resolution_error_has_no_source() {
    use std::error::Error as StdError;

    // Reference-to-missing-node is a string-only failure: no typed upstream.
    // Verify `source: None` and that the chain terminates cleanly.
    let schema = Schema::builder()
        .property(Property::object(FieldKey::new("data").unwrap()))
        .build()
        .unwrap();
    let outputs = DashMap::new();
    let mut params = HashMap::new();
    params.insert(
        "data".to_owned(),
        ParamValue::root_reference(node_key!("missing")),
    );

    let err = resolved_data(&schema, &params, json!(null), &outputs)
        .await
        .unwrap_err();

    let EngineError::ParameterResolution { ref source, .. } = err else {
        panic!("expected ParameterResolution, got {err:?}");
    };
    assert!(
        source.is_none(),
        "reference failures must have source: None (no typed upstream)"
    );
    assert!(
        (&err as &dyn StdError).source().is_none(),
        "std::error::Error::source() must return None for reference failures"
    );
}

fn reference_path_schema() -> ValidSchema {
    Schema::builder()
        .property(
            Property::list(FieldKey::new("items").unwrap()).item(
                Property::object(FieldKey::new("item").unwrap())
                    .property(Property::string(FieldKey::new("name").unwrap())),
            ),
        )
        .build()
        .unwrap()
}

proptest! {
    #[test]
    fn schema_walk_never_rejects_a_runtime_resolvable_reference(index in 0usize..6) {
        let schema = reference_path_schema();
        let output = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let path = ValuePath::from_pointer(&format!("/items/{index}/name")).unwrap();

        if let Some(runtime) = output.pointer(path.as_str()) {
            let walked = schema.walk_reference_path(&path);
            prop_assert!(
                !matches!(walked, PathWalk::Unresolved(_)),
                "runtime resolved `{}` to {runtime:?}, but schema rejected it: {walked:?}",
                path.as_str(),
            );
        }
    }
}
