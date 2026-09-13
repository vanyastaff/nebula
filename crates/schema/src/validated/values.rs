//! Schema-bound proof custody and the consuming runtime transition.

use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use nebula_validator::{ExecutionMode, PredicateContext};
use serde_json::Value;

use super::{
    ValidSchema,
    preparation::{self, Scope},
    typed::{self, SecretDisclosure},
    validation::{self, PendingValidation},
};
use crate::{
    AuthoredValue, CompiledValue, ExpressionContext, FieldKey, MAX_VALUE_DEPTH, MAX_VALUE_NODES,
    MAX_VALUE_TEXT_BYTES, ResolvedValue, SecretValue, ValidationError, ValidationReport, ValuePath,
    ValueTree,
};

const COOPERATIVE_NODE_INTERVAL: usize = 32;

#[derive(Debug, Default)]
struct ResolutionBudget {
    nodes: usize,
    text_bytes: usize,
    visited_since_yield: usize,
}

impl ResolutionBudget {
    async fn cooperate(&mut self) {
        self.visited_since_yield = self.visited_since_yield.saturating_add(1);
        if self.visited_since_yield < COOPERATIVE_NODE_INTERVAL {
            return;
        }
        self.visited_since_yield = 0;
        let mut yielded = false;
        poll_fn(|context| {
            if yielded {
                Poll::Ready(())
            } else {
                yielded = true;
                context.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await;
    }

    fn charge_node(&mut self, depth: usize, path: &ValuePath) -> Result<(), ValidationError> {
        let max_depth = usize::from(MAX_VALUE_DEPTH);
        if depth > max_depth {
            return Err(resolution_limit_exceeded("data depth", max_depth, path));
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_VALUE_NODES {
            return Err(resolution_limit_exceeded(
                "data nodes",
                MAX_VALUE_NODES,
                path,
            ));
        }
        Ok(())
    }

    fn charge_text(&mut self, bytes: usize, path: &ValuePath) -> Result<(), ValidationError> {
        self.text_bytes = self.text_bytes.saturating_add(bytes);
        if self.text_bytes > MAX_VALUE_TEXT_BYTES {
            return Err(resolution_limit_exceeded(
                "data text bytes",
                MAX_VALUE_TEXT_BYTES,
                path,
            ));
        }
        Ok(())
    }

    async fn charge_tree(
        &mut self,
        value: &ResolvedValue,
        path: &ValuePath,
        depth: usize,
    ) -> Result<(), ValidationError> {
        let mut pending = vec![(value, path.clone(), depth)];
        while let Some((value, path, depth)) = pending.pop() {
            self.cooperate().await;
            self.charge_node(depth, &path)?;
            match value {
                ValueTree::Literal(value) => {
                    if let Some(text) = value.as_json().as_str() {
                        self.charge_text(text.len(), &path)?;
                    }
                },
                ValueTree::Secret(secret) => self.charge_text(secret.len_bytes(), &path)?,
                ValueTree::Object(values) => {
                    for (key, value) in values {
                        let child_path = path.push(key);
                        self.charge_text(key.len(), &child_path)?;
                        pending.push((value, child_path, depth.saturating_add(1)));
                    }
                },
                ValueTree::List(values) => {
                    for (index, value) in values.iter().enumerate() {
                        pending.push((
                            value,
                            path.push(index.to_string()),
                            depth.saturating_add(1),
                        ));
                    }
                },
                ValueTree::Expression(_) => {},
            }
        }
        Ok(())
    }
}

fn resolution_limit_exceeded(
    resource: &'static str,
    limit: usize,
    path: &ValuePath,
) -> ValidationError {
    tracing::warn!(
        target: "nebula_schema::dos",
        resource,
        limit,
        path = %path,
        "aggregate expression result budget exceeded"
    );
    ValidationError::builder("value.limit_exceeded")
        .at(path.clone())
        .param("resource", resource)
        .param("limit", limit)
        .message(format!("value {resource} exceeds the {limit}-unit limit"))
        .build()
}

/// Prepared, admitted values tied to an immutable schema snapshot.
///
/// This is not a runtime proof: pending obligations are explicit and only
/// consuming resolution can discharge them.
#[derive(Debug, Clone)]
#[must_use]
pub struct ValidValues {
    schema: ValidSchema,
    values: CompiledValue,
    pending: Arc<[PendingValidation]>,
    expression_paths: Arc<[ValuePath]>,
    warnings: Arc<[ValidationError]>,
    predicate_context: Option<PredicateContext>,
}

pub(super) fn validate_input(
    schema: &ValidSchema,
    values: AuthoredValue,
) -> Result<ValidValues, ValidationReport> {
    let prepared = preparation::prepare_input(values, schema)?;
    let checked = validation::validate_tree(
        schema,
        &prepared.values,
        &prepared.expression_paths,
        ExecutionMode::StaticOnly,
        None,
    );
    if checked.report.has_errors() {
        return Err(checked.report);
    }
    Ok(ValidValues {
        schema: schema.clone(),
        values: prepared.values,
        pending: checked.pending.into(),
        expression_paths: prepared.expression_paths.into(),
        warnings: checked.report.warnings().cloned().collect(),
        predicate_context: checked.predicate_context,
    })
}

impl ValidValues {
    /// Schema snapshot this preparation is bound to.
    #[must_use]
    pub const fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Prepared values. Declared secrets are protected and programs are retained.
    #[must_use]
    pub const fn values(&self) -> &CompiledValue {
        &self.values
    }

    /// Checks not yet proven against runtime values.
    #[must_use]
    pub fn pending(&self) -> &[PendingValidation] {
        &self.pending
    }

    /// Non-fatal validation diagnostics.
    #[must_use]
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Borrow a prepared field.
    #[must_use]
    pub fn get(&self, key: &FieldKey) -> Option<&CompiledValue> {
        self.values.get(key.as_str())
    }

    /// Borrow prepared data at an RFC6901 pointer.
    #[must_use]
    pub fn get_path(&self, path: &ValuePath) -> Option<&CompiledValue> {
        self.values.get_path(path)
    }

    /// Project authored fields with output aliases, union tagging, and secrets omitted.
    #[must_use]
    pub fn to_wire_json(&self) -> Value {
        let data = super::project_tree(
            self.schema.fields(),
            &self.values,
            &|program| serde_json::json!({crate::EXPRESSION_KEY: program.source()}),
        );
        self.schema.raw_values_to_wire(data)
    }

    /// Evaluate admitted programs and fully validate the resulting data.
    ///
    /// Every returned subtree is decoded as data and prepared exactly once.
    /// Existing literal siblings are never transformed a second time. There
    /// is no flag-based bypass of the final rule and conditional-policy pass.
    ///
    /// # Errors
    /// Returns runtime, type, rule, or unresolved-obligation diagnostics.
    ///
    /// # Cancellation
    /// Dropping this future discards its owned, partially resolved tree. The
    /// caller's expression adapter is responsible for its own side effects.
    #[tracing::instrument(level = "debug", target = "nebula_schema::resolve", skip_all,
        fields(expressions = self.expression_paths.len(), pending = self.pending.len()))]
    pub async fn resolve(
        self,
        context: &dyn ExpressionContext,
    ) -> Result<ResolvedValues, ValidationReport> {
        let predicate_context = self
            .predicate_context
            .filter(|_| self.expression_paths.is_empty());
        let mut budget = ResolutionBudget::default();
        let values = resolve_node(
            self.values,
            Scope::Root(self.schema.fields()),
            ValuePath::root(),
            0,
            context,
            &mut budget,
        )
        .await?;
        complete(
            self.schema,
            values,
            &self.expression_paths,
            self.warnings,
            predicate_context,
        )
    }

    /// Complete literal-only input without invoking an expression engine.
    ///
    /// Useful for credentials and other configuration that must not execute code.
    /// Full rules and conditional policies are checked even when preparation
    /// reported no pending expressions.
    ///
    /// # Errors
    /// Returns `expression.forbidden` for any program, or a final validation report.
    #[tracing::instrument(level = "debug", skip_all, fields(pending = self.pending.len()))]
    pub fn resolve_data(self) -> Result<ResolvedValues, ValidationReport> {
        let predicate_context = self
            .predicate_context
            .filter(|_| self.expression_paths.is_empty());
        let values = resolve_data_node(self.values, &ValuePath::root())?;
        complete(
            self.schema,
            values,
            &self.expression_paths,
            self.warnings,
            predicate_context,
        )
    }
}

fn complete(
    schema: ValidSchema,
    values: ResolvedValue,
    expression_paths: &[ValuePath],
    warnings: Arc<[ValidationError]>,
    predicate_context: Option<PredicateContext>,
) -> Result<ResolvedValues, ValidationReport> {
    values.check_budget(|&never| match never {})?;
    let mut checked = validation::validate_tree(
        &schema,
        &values,
        &[],
        ExecutionMode::Full,
        predicate_context,
    );
    for pending in checked.pending {
        checked.report.push(
            ValidationError::builder("validation.incomplete")
                .at(pending.path().clone())
                .message("runtime validation left an unresolved obligation")
                .build(),
        );
    }
    if checked.report.has_errors() {
        let mut report = ValidationReport::new();
        for error in checked.report.iter().cloned() {
            let error = if error.code() == "type_mismatch"
                && expression_paths
                    .iter()
                    .any(|path| error.path().starts_with(path))
            {
                error.with_code("expression.type_mismatch")
            } else {
                error
            };
            report.push(error);
        }
        report.extend(warnings.iter().cloned());
        return Err(report);
    }
    let warnings = warnings
        .iter()
        .cloned()
        .chain(checked.report.warnings().cloned())
        .collect();
    Ok(ResolvedValues {
        schema,
        values,
        warnings,
    })
}

fn resolve_data_node(
    value: CompiledValue,
    path: &ValuePath,
) -> Result<ResolvedValue, ValidationError> {
    match value {
        ValueTree::Literal(value) => Ok(ValueTree::Literal(value)),
        ValueTree::Secret(secret) => Ok(ValueTree::Secret(secret)),
        ValueTree::Object(values) => values
            .into_iter()
            .map(|(key, value)| {
                let value = resolve_data_node(value, &path.push(&key))?;
                Ok((key, value))
            })
            .collect::<Result<_, _>>()
            .map(ValueTree::Object),
        ValueTree::List(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| resolve_data_node(value, &path.push(index.to_string())))
            .collect::<Result<_, _>>()
            .map(ValueTree::List),
        ValueTree::Expression(_) => Err(ValidationError::builder("expression.forbidden")
            .at(path.clone())
            .message("data-only resolution does not execute expressions")
            .build()),
    }
}

type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ResolvedValue, ValidationError>> + Send + 'a>>;

fn resolve_node<'a>(
    value: CompiledValue,
    scope: Scope<'a>,
    path: ValuePath,
    depth: usize,
    context: &'a dyn ExpressionContext,
    budget: &'a mut ResolutionBudget,
) -> ResolveFuture<'a> {
    Box::pin(async move {
        budget.cooperate().await;
        match value {
            ValueTree::Literal(value) => {
                let value = ValueTree::Literal(value);
                budget.charge_tree(&value, &path, depth).await?;
                Ok(value)
            },
            ValueTree::Secret(secret) => {
                let value = ValueTree::Secret(secret);
                budget.charge_tree(&value, &path, depth).await?;
                Ok(value)
            },
            ValueTree::Expression(program) => {
                let result = context
                    .evaluate(&program)
                    .await
                    .map_err(|error| error.at(path.clone()))?;
                let value = preparation::prepare_result(scope, result, &path)?;
                budget.charge_tree(&value, &path, depth).await?;
                Ok(value)
            },
            ValueTree::Object(mut values) => {
                budget.charge_node(depth, &path)?;
                let properties = scope.properties(&mut values);
                let mut resolved = indexmap::IndexMap::with_capacity(values.len());
                for (key, value) in values {
                    let child_path = path.push(&key);
                    budget.charge_text(key.len(), &child_path)?;
                    let value = resolve_node(
                        value,
                        properties.child(&key),
                        child_path,
                        depth.saturating_add(1),
                        context,
                        budget,
                    )
                    .await?;
                    resolved.insert(key, value);
                }
                Ok(ValueTree::Object(resolved))
            },
            ValueTree::List(values) => {
                budget.charge_node(depth, &path)?;
                let mut resolved = Vec::with_capacity(values.len());
                for (index, value) in values.into_iter().enumerate() {
                    resolved.push(
                        resolve_node(
                            value,
                            scope.item(),
                            path.push(index.to_string()),
                            depth.saturating_add(1),
                            context,
                            budget,
                        )
                        .await?,
                    );
                }
                Ok(ValueTree::List(resolved))
            },
        }
    })
}

/// Fully checked runtime data bound to its immutable schema.
///
/// Expression nodes are uninhabited. Construction and deserialization cannot
/// bypass the schema's consuming validation and resolution transitions.
#[derive(Debug, Clone)]
#[must_use]
pub struct ResolvedValues {
    schema: ValidSchema,
    values: ResolvedValue,
    warnings: Arc<[ValidationError]>,
}

/// Distinguishes a missing property from a scalar, secret, or structured value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedLookup<'a> {
    /// No property exists.
    Missing,
    /// A JSON scalar (including explicit null).
    Literal(&'a Value),
    /// Protected material requiring explicit exposure.
    Secret(&'a SecretValue),
    /// Structured object or array.
    Complex(&'a ResolvedValue),
}

impl ResolvedValues {
    /// Schema snapshot this proof is bound to.
    #[must_use]
    pub const fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Fully checked data; expression nodes cannot be constructed.
    #[must_use]
    pub const fn values(&self) -> &ResolvedValue {
        &self.values
    }

    /// Non-fatal diagnostics across preparation and runtime validation.
    #[must_use]
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Borrow a non-secret scalar field.
    #[must_use]
    pub fn get(&self, key: &FieldKey) -> Option<&Value> {
        match self.lookup(key) {
            ResolvedLookup::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow secret material for an explicit trusted consumer.
    #[must_use]
    pub fn get_secret(&self, key: &FieldKey) -> Option<&SecretValue> {
        match self.lookup(key) {
            ResolvedLookup::Secret(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow any data node using its RFC6901 location.
    #[must_use]
    pub fn get_path(&self, path: &ValuePath) -> Option<&ResolvedValue> {
        self.values.get_path(path)
    }

    /// Inspect presence and data kind without exposing secret plaintext.
    #[must_use]
    pub fn lookup(&self, key: &FieldKey) -> ResolvedLookup<'_> {
        match self.values.get(key.as_str()) {
            None => ResolvedLookup::Missing,
            Some(ValueTree::Literal(value)) => ResolvedLookup::Literal(value.as_json()),
            Some(ValueTree::Secret(secret)) => ResolvedLookup::Secret(secret),
            Some(value) => ResolvedLookup::Complex(value),
        }
    }

    /// Return a redacted JSON data view, not a persistence proof.
    #[must_use]
    pub fn into_json(self) -> Value {
        self.values.to_json()
    }

    /// Project output aliases and union tagging while omitting secret fields.
    #[must_use]
    pub fn to_wire_json(&self) -> Value {
        let data =
            super::project_tree(
                self.schema.fields(),
                &self.values,
                &|&impossible| match impossible {},
            );
        self.schema.raw_values_to_wire(data)
    }

    /// Decode validated runtime data to a Rust type, honoring union wire tagging.
    ///
    /// # Errors
    /// Secret-bearing trees require explicit access via `get_secret` or
    /// [`Self::into_typed_exposing_secrets`]; ordinary decoding refuses to turn
    /// protected values into credential strings. Type errors retain their cause
    /// without publishing data.
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, ValidationError> {
        typed::decode(&self.schema, &self.values, SecretDisclosure::Refuse)
    }

    /// Decode at a trusted boundary, explicitly exposing protected secret leaves.
    ///
    /// Secret text is borrowed directly from its protected allocation while serde
    /// constructs the target; no ordinary plaintext JSON string is created. Types
    /// deriving [`derive@crate::Schema`] must use a [`crate::SecretInput`] leaf for every
    /// `#[field(secret)]` property. Schema output aliases and recursive field
    /// projection are applied before root union tagging; no evaluator is involved.
    ///
    /// # Errors
    /// Returns a redacted decoding diagnostic on a target-type mismatch.
    #[track_caller]
    pub fn into_typed_exposing_secrets<T: serde::de::DeserializeOwned>(
        self,
    ) -> Result<T, ValidationError> {
        tracing::debug!(target: "nebula_schema::secret", location = %std::panic::Location::caller(),
            "explicit typed secret exposure");
        typed::decode(&self.schema, &self.values, SecretDisclosure::Expose)
    }
}
