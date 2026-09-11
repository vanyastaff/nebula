//! Schema-bound admission of authored node parameters and retained-program evaluation.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use dashmap::DashMap;
use nebula_core::NodeKey;
use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_schema::{
    AuthoredValue, CompiledProgram, EvalFuture, Expression, ExpressionContext, ResolvedValues,
    ValidSchema, ValidValues, ValidationError, ValidationReport,
};
use nebula_workflow::ParamValue;
use tokio_util::sync::CancellationToken;

use crate::error::EngineError;

/// Admits node parameters against their selected action contract.
pub(crate) struct ParamResolver {
    expression_engine: Arc<ExpressionEngine>,
}

/// Complete context for one node-input admission operation.
pub(crate) struct NodeInputRequest<'a> {
    pub(crate) node_key: &'a NodeKey,
    pub(crate) parameters: &'a HashMap<String, ParamValue>,
    pub(crate) predecessor_input: serde_json::Value,
    pub(crate) outputs: &'a DashMap<NodeKey, serde_json::Value>,
    pub(crate) shared_outputs: &'a DashMap<NodeKey, Arc<serde_json::Value>>,
    pub(crate) schema: &'a ValidSchema,
    pub(crate) cancellation: CancellationToken,
}

impl ParamResolver {
    /// Create a new resolver backed by the given expression engine.
    pub(crate) fn new(expression_engine: Arc<ExpressionEngine>) -> Self {
        Self { expression_engine }
    }

    /// Named parameters are internal schema fields; absent parameters use raw serde wire.
    #[tracing::instrument(name = "engine.input.prepare", skip_all, fields(node_key = %request.node_key), err)]
    pub(crate) fn prepare(
        &self,
        request: NodeInputRequest<'_>,
    ) -> Result<PreparedNodeInput, EngineError> {
        let NodeInputRequest {
            node_key,
            parameters: params,
            predecessor_input,
            outputs,
            shared_outputs,
            schema,
            cancellation,
        } = request;
        let (authored, context) = if params.is_empty() {
            let values = schema
                .values_from_wire(predecessor_input)
                .map_err(|error| {
                    input_error(node_key, error.into(), "input wire shape is invalid")
                })?;
            (values, None)
        } else {
            let has_expressions = params.values().any(|parameter| {
                matches!(
                    parameter,
                    ParamValue::Expression { .. } | ParamValue::Template { .. }
                )
            });
            let referenced_nodes: HashSet<NodeKey> = params
                .values()
                .filter_map(|parameter| match parameter {
                    ParamValue::Reference { node_key, .. } => Some(node_key.clone()),
                    _ => None,
                })
                .collect();
            let mut expression_values = EvaluationContext::new();
            if has_expressions || !referenced_nodes.is_empty() {
                let snapshot = outputs
                    .iter()
                    .filter(|entry| has_expressions || referenced_nodes.contains(entry.key()))
                    .collect::<Vec<_>>();
                EvaluationContext::validate_node_data_snapshot(
                    snapshot
                        .iter()
                        .map(|entry| (entry.key().as_str(), entry.value())),
                )
                .map_err(|source| expression_snapshot_error(node_key, source))?;

                if has_expressions {
                    for entry in snapshot {
                        if !shared_outputs.contains_key(entry.key()) {
                            let shared = share_output_for_expressions(entry.value())
                                .map_err(|source| expression_snapshot_error(node_key, source))?;
                            shared_outputs.insert(entry.key().clone(), shared);
                        }
                    }
                    expression_values
                        .try_set_shared_node_data_batch(shared_outputs.iter().filter_map(|entry| {
                            outputs
                                .contains_key(entry.key())
                                .then(|| (entry.key().clone(), Arc::clone(entry.value())))
                        }))
                        .map_err(|source| expression_snapshot_error(node_key, source))?;
                }
            }
            let values = params
                .iter()
                .map(|(key, parameter)| {
                    Self::author_parameter(node_key, key, parameter, outputs)
                        .map(|value| (key.clone(), value))
                })
                .collect::<Result<_, EngineError>>()?;
            let context = has_expressions.then(|| {
                expression_values.set_input(predecessor_input);
                NodeExpressionContext {
                    engine: Arc::clone(&self.expression_engine),
                    values: expression_values,
                    cancellation,
                }
            });
            (AuthoredValue::Object(values), context)
        };
        let values = schema
            .validate(authored)
            .map_err(|report| input_error(node_key, report, "input schema validation failed"))?;
        Ok(PreparedNodeInput {
            node_key: node_key.clone(),
            values,
            context,
        })
    }

    fn author_parameter(
        node_key: &NodeKey,
        key: &str,
        param: &ParamValue,
        outputs: &DashMap<NodeKey, serde_json::Value>,
    ) -> Result<AuthoredValue, EngineError> {
        let data = |value| {
            AuthoredValue::from_data(value)
                .map_err(|error| input_error(node_key, error.into(), "input data is invalid"))
        };
        match param {
            ParamValue::Literal { value } => data(value.clone()),
            ParamValue::Expression { expr } => {
                Ok(AuthoredValue::Expression(Expression::new(expr.as_str())))
            },
            ParamValue::Template { template } => Ok(AuthoredValue::Expression(
                Expression::template(template.as_str()),
            )),

            ParamValue::Reference {
                node_key: ref_node,
                output_path,
            } => {
                let output =
                    outputs
                        .get(ref_node)
                        .ok_or_else(|| EngineError::ParameterResolution {
                            node_key: node_key.clone(),
                            param_key: key.to_owned(),
                            error: format!("referenced node {ref_node} has no output"),
                            source: None,
                        })?;
                let pointer = output_path.as_str();
                let value = output.value().pointer(pointer).cloned().ok_or_else(|| {
                    EngineError::ParameterResolution {
                        node_key: node_key.clone(),
                        param_key: key.to_owned(),
                        error: format!(
                            "referenced node {ref_node} output path `{pointer}` does not resolve"
                        ),
                        source: None,
                    }
                })?;
                data(value)
            },

            _ => Err(EngineError::ParameterResolution {
                node_key: node_key.clone(),
                param_key: key.to_owned(),
                error: format!("unsupported parameter type for key `{key}`"),
                source: None,
            }),
        }
    }
}

pub(crate) fn share_output_for_expressions(
    value: &serde_json::Value,
) -> Result<Arc<serde_json::Value>, nebula_expression::ExpressionError> {
    EvaluationContext::try_share_value(value)
}

/// An admitted input whose retained programs have not yet been evaluated.
pub(crate) struct PreparedNodeInput {
    node_key: NodeKey,
    values: ValidValues,
    context: Option<NodeExpressionContext>,
}

impl fmt::Debug for PreparedNodeInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedNodeInput")
            .field("node_key", &self.node_key)
            .finish_non_exhaustive()
    }
}

impl PreparedNodeInput {
    #[tracing::instrument(name = "engine.input.resolve", skip_all, fields(node_key = %self.node_key), err)]
    pub(crate) async fn resolve(self) -> Result<ResolvedValues, EngineError> {
        let resolved = match self.context {
            Some(context) => {
                tokio::select! {
                    biased;
                    () = context.cancellation.cancelled() => return Err(EngineError::Cancelled),
                    resolved = self.values.resolve(&context) => resolved,
                }
            },
            None => self.values.resolve_data(),
        };
        resolved.map_err(|report| {
            input_error(&self.node_key, report, "input expression resolution failed")
        })
    }
}

struct NodeExpressionContext {
    engine: Arc<ExpressionEngine>,
    values: EvaluationContext,
    cancellation: CancellationToken,
}

impl ExpressionContext for NodeExpressionContext {
    fn evaluate<'a>(&'a self, program: &'a CompiledProgram) -> EvalFuture<'a> {
        let engine = Arc::clone(&self.engine);
        let values = self.values.clone();
        let program = program.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || engine.evaluate_compiled(&program, &values))
                .await
                .map_err(|source| {
                    ValidationError::builder("expression.runtime")
                        .message("expression evaluation failed")
                        .source(PrivateInputCause { _source: source })
                        .build()
                })?
                .map_err(|source| {
                    ValidationError::builder("expression.runtime")
                        .message("expression evaluation failed")
                        .source(PrivateInputCause { _source: source })
                        .build()
                })
        })
    }
}

// Preserve typed causes without exposing evaluator sources or input payloads.
struct PrivateInputCause<E> {
    _source: E,
}

impl<E> fmt::Debug for PrivateInputCause<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateInputCause(..)")
    }
}

impl<E> fmt::Display for PrivateInputCause<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("input processing failed")
    }
}

impl<E> std::error::Error for PrivateInputCause<E> {}

fn expression_snapshot_error(
    node_key: &NodeKey,
    error: nebula_expression::ExpressionError,
) -> EngineError {
    let message = "expression context exceeds fixed resource limits";
    let source = ValidationError::builder("expression.context_limit")
        .message(message)
        .source(PrivateInputCause { _source: error })
        .build();
    EngineError::ParameterResolution {
        node_key: node_key.clone(),
        param_key: String::new(),
        error: message.to_owned(),
        source: Some(Box::new(source)),
    }
}

fn input_error(node_key: &NodeKey, report: ValidationReport, message: &'static str) -> EngineError {
    let param_key = report
        .errors()
        .next()
        .and_then(|error| {
            error
                .path()
                .segments()
                .next()
                .map(std::borrow::Cow::into_owned)
        })
        .unwrap_or_default();
    let source = ValidationError::builder("input.validation")
        .message(message)
        .source(PrivateInputCause { _source: report })
        .build();
    EngineError::ParameterResolution {
        node_key: node_key.clone(),
        param_key,
        error: message.to_owned(),
        source: Some(Box::new(source)),
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;
    use nebula_schema::{Field, FieldKey, PathWalk, Schema, ValidSchema, ValuePath};
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
            .add(Field::string(FieldKey::new("url").unwrap()))
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
            .add(Field::number(FieldKey::new("count").unwrap()).integer())
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
            .add(Field::number(FieldKey::new("count").unwrap()).integer())
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
            .add(Field::string(FieldKey::new("greeting").unwrap()))
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
            .add(
                Field::object(FieldKey::new("input").unwrap())
                    .add(Field::string(FieldKey::new("data").unwrap())),
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
            .add(Field::number(FieldKey::new("val").unwrap()).integer())
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
        let parameter =
            ParamValue::reference(source_id, ValuePath::from_pointer("/nested").unwrap());

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
        let parameter =
            ParamValue::reference(source_id, ValuePath::from_pointer(output_path).unwrap());

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
            .add(Field::object(FieldKey::new("data").unwrap()))
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
            .add(Field::number(FieldKey::new("bad").unwrap()))
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
            .add(Field::string(FieldKey::new("bad").unwrap()))
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
            .add(Field::number(FieldKey::new("bad").unwrap()))
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
            .add(Field::object(FieldKey::new("data").unwrap()))
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
            .add(
                Field::list(FieldKey::new("items").unwrap()).item(
                    Field::object(FieldKey::new("item").unwrap())
                        .add(Field::string(FieldKey::new("name").unwrap())),
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
}
