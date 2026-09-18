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
    pub(crate) shared_outputs: &'a DashMap<NodeKey, Arc<nebula_expression::RuntimeValue>>,
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
) -> Result<Arc<nebula_expression::RuntimeValue>, nebula_expression::ExpressionError> {
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
#[path = "resolver_tests.rs"]
mod tests;
