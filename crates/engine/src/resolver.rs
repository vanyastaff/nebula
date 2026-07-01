//! Parameter resolution — resolves `ParamValue`s into concrete JSON values.
//!
//! Each node in a workflow can have parameters of type [`ParamValue`]:
//! - `Literal` — static JSON, used as-is
//! - `Expression` — evaluated via [`ExpressionEngine`]
//! - `Template` — parsed and rendered via [`ExpressionEngine`]
//! - `Reference` — looked up from a predecessor node's output

use std::{collections::HashMap, sync::Arc};

use dashmap::DashMap;
use nebula_core::NodeKey;
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
        node_key: &NodeKey,
        params: &HashMap<String, ParamValue>,
        predecessor_input: &serde_json::Value,
        outputs: &DashMap<NodeKey, serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, EngineError> {
        if params.is_empty() {
            return Ok(None);
        }

        // Build expression context
        let mut ctx = EvaluationContext::new();
        ctx.set_input(predecessor_input.clone());

        // Populate $node with all available outputs
        for entry in outputs {
            ctx.set_node_data(entry.key(), entry.value().clone());
        }

        // Resolve each parameter
        let mut resolved = serde_json::Map::new();
        for (key, param_value) in params {
            let value = self.resolve_param(node_key, key, param_value, &ctx, outputs)?;
            resolved.insert(key.clone(), value);
        }

        Ok(Some(serde_json::Value::Object(resolved)))
    }

    /// Resolve a single parameter value.
    fn resolve_param(
        &self,
        node_key: &NodeKey,
        key: &str,
        param: &ParamValue,
        ctx: &EvaluationContext,
        outputs: &DashMap<NodeKey, serde_json::Value>,
    ) -> Result<serde_json::Value, EngineError> {
        match param {
            ParamValue::Literal { value } => Ok(value.clone()),

            ParamValue::Expression { expr } => {
                self.expression_engine
                    .evaluate(expr, ctx)
                    .map_err(|expression_error| EngineError::ParameterResolution {
                        node_key: node_key.clone(),
                        param_key: key.to_owned(),
                        error: expression_error.to_string(),
                        source: Some(Box::new(expression_error)),
                    })
            },

            ParamValue::Template { template } => {
                let tmpl = self.expression_engine.parse_template(template).map_err(
                    |expression_error| EngineError::ParameterResolution {
                        node_key: node_key.clone(),
                        param_key: key.to_owned(),
                        error: format!("template parse error: {expression_error}"),
                        source: Some(Box::new(expression_error)),
                    },
                )?;
                let rendered = self.expression_engine.render_template(&tmpl, ctx).map_err(
                    |expression_error| EngineError::ParameterResolution {
                        node_key: node_key.clone(),
                        param_key: key.to_owned(),
                        error: format!("template render error: {expression_error}"),
                        source: Some(Box::new(expression_error)),
                    },
                )?;
                Ok(serde_json::Value::String(rendered))
            },

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
                let value = navigate_path(output.value(), output_path);
                Ok(value)
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

/// Navigate a JSON value by a dot-separated path.
///
/// Supports object key access and array index access, with an optional JSONPath
/// root prefix:
/// - `"data.items"` → `value["data"]["items"]`
/// - `"$.data.items"` → same (the `$.` root is stripped)
/// - `"items.0.name"` → `value["items"][0]["name"]`
/// - `"$"` → the whole value (root)
///
/// The public [`ParamValue::reference`](nebula_workflow::ParamValue::reference)
/// constructor documents `output_path` as JSONPath (`$.data.items`), while this
/// navigator splits on `.`. Stripping an optional leading `$.` (or a bare `$`)
/// reconciles the two grammars so both forms resolve to the same location;
/// without it a `$.`-prefixed path looked up a literal `"$"` key and silently
/// resolved to `Null`. (A JSON object key literally named `$…` is not
/// addressable — `$` is reserved for the JSONPath root, as in the standard.)
///
/// Returns `Value::Null` for missing keys or out-of-bounds indices.
fn navigate_path(value: &serde_json::Value, path: &str) -> serde_json::Value {
    let path = match path.strip_prefix("$.") {
        Some(rest) => rest,
        None => path.strip_prefix('$').unwrap_or(path),
    };
    if path.is_empty() {
        return value.clone();
    }
    let mut current = value;
    for segment in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(segment).unwrap_or(&serde_json::Value::Null);
            },
            serde_json::Value::Array(arr) => {
                if let Ok(idx) = segment.parse::<usize>() {
                    current = arr.get(idx).unwrap_or(&serde_json::Value::Null);
                } else {
                    return serde_json::Value::Null;
                }
            },
            _ => return serde_json::Value::Null,
        }
    }
    current.clone()
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;
    use nebula_schema::{Field, FieldKey, PathWalk, Schema, ValidSchema};
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

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

    /// The documented JSONPath grammar (`$.data.items`) must resolve to the same
    /// place as the bare-dotted form. Before the root-prefix reconciliation a
    /// `$.`-prefixed path looked up a literal `"$"` key and returned `Null`.
    #[test]
    fn navigate_path_strips_jsonpath_root_prefix() {
        let val = json!({"data": {"items": [1, 2, 3]}});
        assert_eq!(
            navigate_path(&val, "$.data.items.0"),
            json!(1),
            "`$.`-prefixed JSONPath must resolve like the bare-dotted form"
        );
        // Both grammars agree.
        assert_eq!(
            navigate_path(&val, "$.data.items"),
            navigate_path(&val, "data.items"),
        );
    }

    /// A bare `$` is the JSONPath root — the whole value.
    #[test]
    fn navigate_path_bare_root_returns_whole_value() {
        let val = json!({"a": 1});
        assert_eq!(navigate_path(&val, "$"), val);
    }

    // -- resolve tests --

    #[test]
    fn empty_params_returns_none() {
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let result = resolver
            .resolve(&node_key!("test"), &HashMap::new(), &json!(null), &outputs)
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
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
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
            .resolve(&node_key!("test"), &params, &input, &outputs)
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
            .resolve(&node_key!("test"), &params, &input, &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["greeting"], json!("Hello World!"));
    }

    #[test]
    fn reference_resolution_looks_up_output() {
        let resolver = make_resolver();
        let source_id = node_key!("source");
        let outputs = DashMap::new();
        outputs.insert(source_id.clone(), json!({"data": "fetched"}));

        let mut params = HashMap::new();
        params.insert("input".to_owned(), ParamValue::reference(source_id, ""));

        let result = resolver
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["input"], json!({"data": "fetched"}));
    }

    #[test]
    fn reference_with_path_navigates_output() {
        let resolver = make_resolver();
        let source_id = node_key!("source");
        let outputs = DashMap::new();
        outputs.insert(source_id.clone(), json!({"nested": {"value": 42}}));

        let mut params = HashMap::new();
        params.insert(
            "val".to_owned(),
            ParamValue::reference(source_id, "nested.value"),
        );

        let result = resolver
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
            .unwrap()
            .unwrap();
        assert_eq!(result["val"], json!(42));
    }

    #[test]
    fn reference_to_missing_node_returns_error() {
        let resolver = make_resolver();
        let missing_id = node_key!("missing");
        let outputs = DashMap::new();

        let mut params = HashMap::new();
        params.insert("data".to_owned(), ParamValue::reference(missing_id, ""));

        let err = resolver
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
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
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
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
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
            .unwrap_err();
        assert!(matches!(err, EngineError::ParameterResolution { .. }));
    }

    // ── FIX 4: ParameterResolution carries a typed #[source] ─────────────────
    //
    // Before the fix, `ParameterResolution` was a `{ node_key, param_key,
    // error: String }` — the upstream `ExpressionError` was stringified and
    // thrown away, breaking the `std::error::Error` source chain. After the
    // fix, expression-originated failures carry `source: Some(ExpressionError)`
    // so callers can inspect or route on the typed upstream error and
    // `std::error::Error::source()` returns `Some(&ExpressionError)`.

    #[test]
    fn expression_resolution_error_preserves_typed_source() {
        use std::error::Error as StdError;

        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        params.insert("bad".to_owned(), ParamValue::expression("$nonexistent.foo"));

        let err = resolver
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
            .unwrap_err();

        // The error must be the ParameterResolution variant with a typed source.
        let EngineError::ParameterResolution { ref source, .. } = err else {
            panic!("expected ParameterResolution, got {err:?}");
        };
        assert!(
            source.is_some(),
            "expression eval failure must carry a typed ExpressionError source, got None"
        );

        // The std::error::Error source chain must be intact.
        assert!(
            (&err as &dyn StdError).source().is_some(),
            "std::error::Error::source() must return Some(_) for expression failures"
        );
    }

    #[test]
    fn reference_resolution_error_has_no_source() {
        use std::error::Error as StdError;

        // Reference-to-missing-node is a string-only failure: no typed upstream.
        // Verify `source: None` and that the chain terminates cleanly.
        let resolver = make_resolver();
        let outputs = DashMap::new();
        let mut params = HashMap::new();
        params.insert(
            "data".to_owned(),
            ParamValue::reference(node_key!("missing"), ""),
        );

        let err = resolver
            .resolve(&node_key!("test"), &params, &json!(null), &outputs)
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

    // -- walk_authored_path vs. navigate_path tripwire (ADR-0100 TypeDAG, W0 U5) --
    //
    // `nebula_schema::ValidSchema::walk_authored_path` is a validation-time,
    // schema-only walk; `navigate_path` above is the runtime, value-only
    // navigator. This crate is the one place both are reachable together
    // (`navigate_path` is private to this module; `nebula-schema` cannot depend
    // on `nebula-engine` to call it, and vice versa this crate already depends
    // on `nebula-schema`) — see the W0 U5 plan's "Option B" on why the two are
    // deliberately NOT unified into shared code.

    /// A fully-closed schema for the tripwire: `items: List<Object { name:
    /// String }>` — matches the plan's example paths (`items.0.name` /
    /// `$.items.0.name`) exactly.
    fn tripwire_schema() -> ValidSchema {
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

    /// A value conforming to [`tripwire_schema`]: two items, indices 0 and 1
    /// resolve to a real (non-`Null`) `name`.
    fn tripwire_conforming_value() -> serde_json::Value {
        json!({"items": [{"name": "a"}, {"name": "b"}]})
    }

    proptest! {
        /// Tripwire, not a completeness proof (framed per the W0 U5 plan): over a
        /// fully-closed `(schema, conforming value)` fixture, `walk_authored_path`
        /// must never claim a path is a PROVABLE MISTAKE
        /// (`PathWalk::Unresolved`) when the runtime `navigate_path` actually
        /// resolves that SAME authored path to a real (non-`Null`) value. A
        /// divergence here would mean the walk hard-rejects a `Reference` the
        /// engine would happily resolve at runtime — exactly the false-positive
        /// class the four review rounds were about.
        ///
        /// Runtime output is never validated against its declared schema
        /// (`engine.rs:2565-2567`), so the two navigators CAN legitimately
        /// diverge outside the cases this design already excludes (this fixture
        /// is fully closed by construction, so no such divergence is expected
        /// here — the property still only asserts the one-directional
        /// implication, not full agreement).
        #[test]
        fn walk_never_unresolved_where_navigate_path_resolves(
            index in 0usize..6,
            dollar_prefix in any::<bool>(),
        ) {
            let schema = tripwire_schema();
            let value = tripwire_conforming_value();
            let path = if dollar_prefix {
                format!("$.items.{index}.name")
            } else {
                format!("items.{index}.name")
            };

            let runtime = navigate_path(&value, &path);
            if runtime != serde_json::Value::Null {
                let walked = schema.walk_authored_path(&path);
                prop_assert!(
                    !matches!(walked, PathWalk::Unresolved(_)),
                    "navigate_path resolved `{path}` to {runtime:?}, but walk_authored_path \
                     claimed it was unresolvable: {walked:?}"
                );
            }
        }
    }
}
