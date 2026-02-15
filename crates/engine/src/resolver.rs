//! Parameter resolution — resolves `ParamValue`s into concrete JSON values.
//!
//! Each node in a workflow can have parameters of type [`ParamValue`]:
//! - `Literal` — static JSON, used as-is
//! - `Expression` — evaluated via [`ExpressionEngine`]
//! - `Template` — parsed and rendered via [`ExpressionEngine`]
//! - `Reference` — looked up from a predecessor node's output

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use nebula_core::id::NodeId;
use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_workflow::ParamValue;

use crate::error::EngineError;

/// Resolves node parameters into concrete JSON values.
pub(crate) struct ParamResolver {
    expression_engine: Arc<ExpressionEngine>,
}

impl ParamResolver {
    /// Create a new resolver backed by the given expression engine.
    pub(crate) fn new(expression_engine: Arc<ExpressionEngine>) -> Self {
        Self { expression_engine }
    }

    /// Resolve all parameters for a node, producing a JSON object.
    ///
    /// If the node has no parameters, returns `None` (caller uses
    /// predecessor output as-is for backward compatibility).
    pub(crate) fn resolve(
        &self,
        node_id: NodeId,
        params: &HashMap<String, ParamValue>,
        predecessor_input: &serde_json::Value,
        outputs: &DashMap<NodeId, serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, EngineError> {
        if params.is_empty() {
            return Ok(None);
        }

        // Build expression context
        let mut ctx = EvaluationContext::new();
        ctx.set_input(predecessor_input.clone());

        // Populate $node with all available outputs
        for entry in outputs.iter() {
            ctx.set_node_data(entry.key().to_string(), entry.value().clone());
        }

        // Resolve each parameter
        let mut resolved = serde_json::Map::new();
        for (key, param_value) in params {
            let value = self.resolve_param(node_id, key, param_value, &ctx, outputs)?;
            resolved.insert(key.clone(), value);
        }

        Ok(Some(serde_json::Value::Object(resolved)))
    }

    /// Resolve a single parameter value.
    fn resolve_param(
        &self,
        node_id: NodeId,
        key: &str,
        param: &ParamValue,
        ctx: &EvaluationContext,
        outputs: &DashMap<NodeId, serde_json::Value>,
    ) -> Result<serde_json::Value, EngineError> {
        match param {
            ParamValue::Literal { value } => Ok(value.clone()),

            ParamValue::Expression { expr } => {
                self.expression_engine.evaluate(expr, ctx).map_err(|e| {
                    EngineError::ParameterResolution {
                        node_id,
                        param_key: key.to_owned(),
                        error: e.to_string(),
                    }
                })
            }

            ParamValue::Template { template } => {
                let tmpl = self
                    .expression_engine
                    .parse_template(template)
                    .map_err(|e| EngineError::ParameterResolution {
                        node_id,
                        param_key: key.to_owned(),
                        error: format!("template parse error: {e}"),
                    })?;
                let rendered = self
                    .expression_engine
                    .render_template(&tmpl, ctx)
                    .map_err(|e| EngineError::ParameterResolution {
                        node_id,
                        param_key: key.to_owned(),
                        error: format!("template render error: {e}"),
                    })?;
                Ok(serde_json::Value::String(rendered))
            }

            ParamValue::Reference {
                node_id: ref_node,
                output_path,
            } => {
                let output =
                    outputs
                        .get(ref_node)
                        .ok_or_else(|| EngineError::ParameterResolution {
                            node_id,
                            param_key: key.to_owned(),
                            error: format!("referenced node {} has no output", ref_node),
                        })?;
                let value = navigate_path(output.value(), output_path);
                Ok(value)
            }
        }
    }
}

/// Navigate a JSON value by a dot-separated path.
///
/// Supports object key access and array index access:
/// - `"data.items"` → `value["data"]["items"]`
/// - `"items.0.name"` → `value["items"][0]["name"]`
///
/// Returns `Value::Null` for missing keys or out-of-bounds indices.
fn navigate_path(value: &serde_json::Value, path: &str) -> serde_json::Value {
    if path.is_empty() {
        return value.clone();
    }
    let mut current = value;
    for segment in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(segment).unwrap_or(&serde_json::Value::Null);
            }
            serde_json::Value::Array(arr) => {
                if let Ok(idx) = segment.parse::<usize>() {
                    current = arr.get(idx).unwrap_or(&serde_json::Value::Null);
                } else {
                    return serde_json::Value::Null;
                }
            }
            _ => return serde_json::Value::Null,
        }
    }
    current.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_resolver() -> ParamResolver {
        let engine = Arc::new(ExpressionEngine::new());
        ParamResolver::new(engine)
    }

    // -- navigate_path tests --

    #[test]
    fn navigate_path_empty_returns_value() {
        let val = json!({"a": 1});
        assert_eq!(navigate_path(&val, ""), val);
    }

    #[test]
    fn navigate_path_object_key() {
        let val = json!({"data": {"name": "Alice"}});
        assert_eq!(navigate_path(&val, "data.name"), json!("Alice"));
    }

    #[test]
    fn navigate_path_array_index() {
        let val = json!({"items": [10, 20, 30]});
        assert_eq!(navigate_path(&val, "items.1"), json!(20));
    }

    #[test]
    fn navigate_path_missing_key_returns_null() {
        let val = json!({"a": 1});
        assert_eq!(navigate_path(&val, "b"), json!(null));
    }

    #[test]
    fn navigate_path_out_of_bounds_returns_null() {
        let val = json!({"items": [1]});
        assert_eq!(navigate_path(&val, "items.5"), json!(null));
    }

    #[test]
    fn navigate_path_non_container_returns_null() {
        let val = json!(42);
        assert_eq!(navigate_path(&val, "key"), json!(null));
    }

    #[test]
    fn navigate_path_array_with_non_numeric_returns_null() {
        let val = json!([1, 2, 3]);
        assert_eq!(navigate_path(&val, "name"), json!(null));
    }

    // -- resolve tests --

    #[test]
    fn empty_params_returns_none() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let result = resolver
            .resolve(NodeId::v4(), &HashMap::new(), &json!(null), &outputs)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn literal_resolution_passthrough() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        params.insert(
            "url".to_owned(),
            ParamValue::literal(json!("https://example.com")),
        );

        let result = resolver
            .resolve(NodeId::v4(), &params, &json!(null), &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["url"], json!("https://example.com"));
    }

    #[test]
    fn expression_resolution_evaluates() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        params.insert(
            "count".to_owned(),
            ParamValue::expression("$input.count + 1"),
        );

        let input = json!({"count": 5});
        let result = resolver
            .resolve(NodeId::v4(), &params, &input, &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["count"], json!(6));
    }

    #[test]
    fn template_resolution_renders() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        params.insert(
            "greeting".to_owned(),
            ParamValue::template("Hello {{ $input.name }}!"),
        );

        let input = json!({"name": "World"});
        let result = resolver
            .resolve(NodeId::v4(), &params, &input, &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["greeting"], json!("Hello World!"));
    }

    #[test]
    fn reference_resolution_looks_up_output() {
        let resolver = make_resolver();
        let source_id = NodeId::v4();
        let outputs = DashMap::new();
        outputs.insert(source_id, json!({"data": "fetched"}));

        let mut params = HashMap::new();
        params.insert("input".to_owned(), ParamValue::reference(source_id, ""));

        let result = resolver
            .resolve(NodeId::v4(), &params, &json!(null), &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["input"], json!({"data": "fetched"}));
    }

    #[test]
    fn reference_with_path_navigates_output() {
        let resolver = make_resolver();
        let source_id = NodeId::v4();
        let outputs = DashMap::new();
        outputs.insert(source_id, json!({"nested": {"value": 42}}));

        let mut params = HashMap::new();
        params.insert(
            "val".to_owned(),
            ParamValue::reference(source_id, "nested.value"),
        );

        let result = resolver
            .resolve(NodeId::v4(), &params, &json!(null), &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["val"], json!(42));
    }

    #[test]
    fn reference_to_missing_node_returns_error() {
        let resolver = make_resolver();
        let missing_id = NodeId::v4();
        let outputs = DashMap::new();

        let mut params = HashMap::new();
        params.insert("data".to_owned(), ParamValue::reference(missing_id, ""));

        let err = resolver
            .resolve(NodeId::v4(), &params, &json!(null), &outputs)
            .unwrap_err();
        assert!(matches!(err, EngineError::ParameterResolution { .. }));
        assert!(err.to_string().contains("has no output"));
    }

    #[test]
    fn expression_eval_failure_returns_error() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        // invalid expression: accessing property on undefined variable
        params.insert(
            "bad".to_owned(),
            ParamValue::expression("$nonexistent.foo.bar"),
        );

        let err = resolver
            .resolve(NodeId::v4(), &params, &json!(null), &outputs)
            .unwrap_err();
        assert!(matches!(err, EngineError::ParameterResolution { .. }));
    }

    #[test]
    fn template_parse_failure_returns_error() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        // Unclosed template delimiter
        params.insert("bad".to_owned(), ParamValue::template("Hello {{ unclosed"));

        let err = resolver
            .resolve(NodeId::v4(), &params, &json!(null), &outputs)
            .unwrap_err();
        assert!(matches!(err, EngineError::ParameterResolution { .. }));
    }
}
