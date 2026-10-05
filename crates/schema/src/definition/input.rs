//! Graph-bound consuming input preparation, resolution and typed decode custody.

use std::{future::Future, pin::Pin};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use nebula_validator::PredicateContext;
use serde_json::Value;
use zeroize::Zeroize;

use super::{
    AdmittedSchemaGraph, InputContract,
    model::{AdditionalProperties, Body, UseSiteCore, ValueProtection},
    runtime, sensitive,
};
use crate::{
    AuthoredValue, CompiledValue, ExpressionContext, ExpressionMode, InputCodec, PendingValidation,
    ResolvedValue, ScalarValue, SecretValue, SerdeTagging, ValidationError, ValidationReport,
    ValuePath, ValueTree,
};

/// Prepared input with unresolved programs and explicit validation obligations.
pub struct ValidInputValues {
    contract: InputContract,
    values: CompiledValue,
    expression_paths: Vec<ValuePath>,
    pending: Vec<PendingValidation>,
    symbolic_paths: Vec<ValuePath>,
}

/// Fully resolved values bound to one exact admitted inbound contract.
pub struct ResolvedInputValues {
    contract: InputContract,
    values: ResolvedValue,
    predicate_context: PredicateContext,
}

impl std::fmt::Debug for ValidInputValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidInputValues")
            .field("contract", &self.contract)
            .field("expressions", &self.expression_paths.len())
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ResolvedInputValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedInputValues")
            .field("contract", &self.contract)
            .finish_non_exhaustive()
    }
}

impl InputContract {
    /// Decode ordinary wire data without inferring authoring expressions.
    ///
    /// # Errors
    /// Rejects value depth, node and text-budget violations.
    pub fn values_from_wire(&self, data: Value) -> Result<AuthoredValue, ValidationError> {
        AuthoredValue::from_data(data)
    }

    /// Consume authored input and validate all currently available obligations.
    ///
    /// # Errors
    /// Rejects expression permission, preparation, shape, rules and policies.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn validate(&self, values: AuthoredValue) -> Result<ValidInputValues, ValidationReport> {
        self.validate_symbolic(values, &[])
    }

    /// Validate authored constants alongside independently recorded references.
    ///
    /// Symbolic destinations retain static obligations and never authorize
    /// expression execution or resolved proof. The compiler owns source binding
    /// and producer-to-consumer graph compatibility.
    ///
    /// # Errors
    /// Rejects undeclared destinations and all available constant obligations.
    pub fn validate_symbolic(
        &self,
        values: AuthoredValue,
        symbolic_paths: &[ValuePath],
    ) -> Result<ValidInputValues, ValidationReport> {
        if symbolic_paths.len() > crate::value::MAX_EXPRESSION_ENTRIES
            || symbolic_paths
                .iter()
                .map(|path| path.as_str().len())
                .try_fold(0_usize, usize::checked_add)
                .is_none_or(|bytes| bytes > crate::value::MAX_EXPRESSION_TEXT_BYTES)
        {
            return Err(runtime::error("value.limit_exceeded", &ValuePath::root()));
        }
        let symbolic_paths = symbolic_paths
            .iter()
            .map(|path| {
                self.graph
                    .input_reference_at(path)
                    .map(|reference| reference.canonical_path().clone())
                    .map_err(|_| runtime::error("schema.reference.undeclared", path))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if symbolic_paths
            .iter()
            .map(|path| path.as_str().len())
            .try_fold(0_usize, usize::checked_add)
            .is_none_or(|bytes| bytes > crate::value::MAX_EXPRESSION_TEXT_BYTES)
        {
            return Err(runtime::error("value.limit_exceeded", &ValuePath::root()));
        }
        for (index, path) in symbolic_paths.iter().enumerate() {
            if symbolic_paths[..index]
                .iter()
                .any(|other| path.starts_with(other) || other.starts_with(path))
            {
                return Err(runtime::error("schema.reference.ambiguous", path));
            }
        }
        values.check_budget(|expression| expression.source())?;
        let mut expression_paths = Vec::new();
        let values = prepare(
            &self.graph,
            Some(&self.graph.0.graph.root.0),
            values,
            &ValuePath::root(),
            PreparationScope::default(),
            &mut expression_paths,
            &symbolic_paths,
        )?;
        for path in &symbolic_paths {
            if contains_path(&values, path) {
                return Err(runtime::error("schema.reference.literal_conflict", path));
            }
        }
        values.check_budget(|program| program.source())?;
        let unavailable: Vec<_> = expression_paths
            .iter()
            .chain(&symbolic_paths)
            .cloned()
            .collect();
        let context = sensitive::predicate_context(&self.graph, &values, &unavailable)?;
        let skeleton = literal_skeleton(&values);
        let json = sensitive::SensitiveJson::new(&skeleton)?;
        let pending = runtime::validate_literal(
            &self.graph,
            json.value(),
            &context,
            false,
            false,
            &unavailable,
        )?;
        Ok(ValidInputValues {
            contract: self.clone(),
            values,
            expression_paths,
            pending,
            symbolic_paths: symbolic_paths.clone(),
        })
    }

    /// Admit literal wire data without evaluating expressions.
    ///
    /// # Errors
    /// Rejects any outstanding expression or full-validation obligation.
    pub fn validate_data(&self, data: Value) -> Result<ResolvedInputValues, ValidationReport> {
        self.validate(self.values_from_wire(data)?)?.resolve_data()
    }
}

fn contains_path<E>(value: &ValueTree<E>, path: &ValuePath) -> bool {
    let mut current = value;
    for segment in path.segments() {
        current = match current {
            ValueTree::Object(values) => match values.get(segment.as_ref()) {
                Some(value) => value,
                None => return false,
            },
            ValueTree::List(values) => match segment
                .parse::<usize>()
                .ok()
                .and_then(|index| values.get(index))
            {
                Some(value) => value,
                None => return false,
            },
            _ => return false,
        };
    }
    true
}

impl ValidInputValues {
    /// Exact compiled contract retained by this preparation.
    #[must_use]
    pub const fn contract(&self) -> &InputContract {
        &self.contract
    }
    /// Prepared protected values and retained programs.
    #[must_use]
    pub const fn values(&self) -> &CompiledValue {
        &self.values
    }
    /// Canonical inbound paths of admitted programs.
    #[must_use]
    pub fn expression_paths(&self) -> &[ValuePath] {
        &self.expression_paths
    }
    /// Explicit obligations that static validation could not discharge.
    #[must_use]
    pub fn pending(&self) -> &[PendingValidation] {
        &self.pending
    }
    /// Whether root rules still depend on unavailable values or evaluators.
    #[must_use]
    pub fn has_pending_root_checks(&self) -> bool {
        self.pending
            .iter()
            .any(|pending| pending.path().depth() == 0)
    }

    /// Resolve retained programs and recheck the complete graph before minting proof.
    ///
    /// # Errors
    /// Returns payload-free evaluation or final validation diagnostics.
    ///
    /// # Cancellation
    /// Dropping the future drops its owned partially resolved values; expression
    /// adapters retain responsibility for their own side effects.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn resolve(
        self,
        context: &dyn ExpressionContext,
    ) -> Result<ResolvedInputValues, ValidationReport> {
        if !self.symbolic_paths.is_empty() {
            return Err(runtime::error(
                "validation.symbolic_input",
                &ValuePath::root(),
            ));
        }
        let values = resolve_node(
            &self.contract.graph,
            Some(&self.contract.graph.0.graph.root.0),
            self.values,
            ValuePath::root(),
            Some(context),
        )
        .await?;
        complete(self.contract, values)
    }

    /// Complete literal input, rejecting programs instead of executing them.
    ///
    /// # Errors
    /// Returns expression refusal or full-validation diagnostics.
    pub fn resolve_data(self) -> Result<ResolvedInputValues, ValidationReport> {
        if !self.symbolic_paths.is_empty() {
            return Err(runtime::error(
                "validation.symbolic_input",
                &ValuePath::root(),
            ));
        }
        if !self.expression_paths.is_empty() {
            return Err(runtime::error(
                "expression.forbidden",
                &self.expression_paths[0],
            ));
        }
        complete(self.contract, literal_skeleton(&self.values))
    }
}

impl ResolvedInputValues {
    /// Exact retained inbound contract.
    #[must_use]
    pub const fn contract(&self) -> &InputContract {
        &self.contract
    }
    /// Fully resolved protected native wire tree.
    #[must_use]
    pub const fn values(&self) -> &ResolvedValue {
        &self.values
    }
    /// Secret-free prepared context for checked slot conditions.
    #[must_use]
    pub const fn predicate_context(&self) -> &PredicateContext {
        &self.predicate_context
    }
    /// Whether this proof belongs to the exact requested codec and graph.
    #[must_use]
    pub fn belongs_to(&self, contract: &InputContract) -> bool {
        self.contract.semantic_commitment() == contract.semantic_commitment()
            && std::sync::Arc::ptr_eq(&self.contract.graph().0, &contract.graph().0)
            && self.contract.codec_type == contract.codec_type
    }

    /// Consume this proof into ordinary data, refusing any protected material.
    ///
    /// # Errors
    /// Returns a payload-free disclosure refusal for secret-bearing trees.
    pub fn into_wire_data(self) -> Result<Value, ValidationError> {
        sensitive::decode_wire(&self.values, false)
    }

    /// Decode once through the schema-owned input codec, refusing protected values.
    ///
    /// # Errors
    /// Rejects mismatched proof custody, codec definitions or decoding failures.
    pub fn into_typed<T: InputCodec>(self, contract: &InputContract) -> Result<T, ValidationError> {
        self.decode(contract, false)
    }

    /// Explicit trusted disclosure boundary for typed protected input.
    ///
    /// # Errors
    /// Rejects mismatched proof custody, codec definitions or decoding failures.
    pub fn into_typed_exposing_secrets<T: InputCodec>(
        self,
        contract: &InputContract,
    ) -> Result<T, ValidationError> {
        self.decode(contract, true)
    }

    fn decode<T: InputCodec>(
        self,
        contract: &InputContract,
        expose: bool,
    ) -> Result<T, ValidationError> {
        if !self.belongs_to(contract) {
            return Err(custody_error());
        }
        if contract.codec_type != Some(std::any::TypeId::of::<T>()) {
            return Err(custody_error());
        }
        sensitive::decode_wire(&self.values, expose)
    }
}

fn custody_error() -> ValidationError {
    ValidationError::builder("schema.input.contract_mismatch")
        .message("input proof does not belong to the requested codec contract")
        .build()
}

fn complete(
    contract: InputContract,
    values: ResolvedValue,
) -> Result<ResolvedInputValues, ValidationReport> {
    values.check_budget(|&never| match never {})?;
    let predicate_context = sensitive::predicate_context(&contract.graph, &values, &[])?;
    {
        let json = sensitive::SensitiveJson::new(&values)?;
        let pending = runtime::validate_literal(
            &contract.graph,
            json.value(),
            &predicate_context,
            false,
            true,
            &[],
        )?;
        if !pending.is_empty() {
            return Err(runtime::error("validation.incomplete", &ValuePath::root()));
        }
    }
    Ok(ResolvedInputValues {
        contract,
        values,
        predicate_context,
    })
}

fn literal_skeleton(values: &CompiledValue) -> ResolvedValue {
    match values {
        ValueTree::Literal(value) => ValueTree::Literal(value.clone()),
        ValueTree::Secret(value) => ValueTree::Secret(value.clone()),
        ValueTree::Expression(_) => ValueTree::Literal(ScalarValue::null()),
        ValueTree::Object(values) => ValueTree::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), literal_skeleton(value)))
                .collect(),
        ),
        ValueTree::List(values) => ValueTree::List(values.iter().map(literal_skeleton).collect()),
    }
}

fn body<'a>(
    graph: &'a AdmittedSchemaGraph,
    core: &'a UseSiteCore,
) -> Result<&'a Body, ValidationError> {
    let index = graph.0.lookup.get(&core.target).ok_or_else(custody_error)?;
    Ok(&graph.0.graph.definitions[index.0].body)
}

#[derive(Clone, Copy)]
struct PreparationScope {
    inherited_forbidden: bool,
    evaluated: bool,
    protection: ValueProtection,
}

impl Default for PreparationScope {
    fn default() -> Self {
        Self {
            inherited_forbidden: false,
            evaluated: false,
            protection: ValueProtection::Public,
        }
    }
}

fn prepare<'a>(
    graph: &'a AdmittedSchemaGraph,
    mut core: Option<&'a UseSiteCore>,
    mut value: AuthoredValue,
    path: &ValuePath,
    mut scope: PreparationScope,
    expressions: &mut Vec<ValuePath>,
    symbolic_paths: &[ValuePath],
) -> Result<CompiledValue, ValidationError> {
    let mut alias_hops = 0_usize;
    let mut occurrence_forbidden = scope.inherited_forbidden;
    let descendants_forbidden = loop {
        let forbidden = scope.inherited_forbidden
            || core.is_none_or(|core| core.expression == ExpressionMode::Forbidden);
        // The outer authoring occurrence controls a root expression locally. Only
        // actual property occurrences carry a prohibition into descendant fields.
        let descendants_forbidden = scope.inherited_forbidden || (path.depth() != 0 && forbidden);
        occurrence_forbidden |= forbidden;
        if !scope.evaluated
            && !matches!(value, ValueTree::Expression(_))
            && core.is_some_and(|core| core.expression == ExpressionMode::Required)
        {
            return Err(ValidationError::builder("expression.required")
                .at(path.clone())
                .message("declared input requires an authored expression")
                .build());
        }
        if let Some(occurrence) = core
            && let Body::Alias(alias) = body(graph, occurrence)?
        {
            alias_hops += 1;
            if alias_hops > graph.definition_count() {
                return Err(custody_error());
            }
            value = prepare_scalar_facets(occurrence, value, path)?;
            scope = PreparationScope {
                inherited_forbidden: descendants_forbidden,
                evaluated: scope.evaluated,
                protection: if occurrence.protection == ValueProtection::Public {
                    scope.protection
                } else {
                    occurrence.protection
                },
            };
            core = Some(&alias.0);
            continue;
        }
        break descendants_forbidden;
    };
    if let ValueTree::Expression(expression) = value {
        if occurrence_forbidden {
            return Err(ValidationError::builder("expression.forbidden")
                .at(path.clone())
                .message("input expression has no permitting declaration")
                .build());
        }
        let program = expression.parse_at(path)?.clone();
        expressions.push(path.clone());
        return Ok(ValueTree::Expression(program));
    }
    let value = if let Some(core) = core {
        prepare_scalar_facets(core, value, path)?
    } else {
        value
    };
    let value = if let Some(core) = core
        && let Body::Union(union) = body(graph, core)?
    {
        normalize_union(union, value)?
    } else {
        value
    };
    match value {
        ValueTree::Literal(mut value) => {
            if let Some(core) = core
                && matches!(body(graph, core)?, Body::Integer(_))
                && let Value::Number(number) = value.as_json()
                && let Some(integer) = crate::validated::exact_integer(number)
            {
                value = ScalarValue::try_from(Value::Number(integer))?;
            }
            Ok(ValueTree::Literal(value))
        },
        ValueTree::Secret(value) => {
            let protection = core
                .filter(|core| core.protection != ValueProtection::Public)
                .map_or(scope.protection, |core| core.protection);
            if protection == ValueProtection::Public {
                return Err(ValidationError::builder("secret.undeclared")
                    .at(path.clone())
                    .message("protected input has no protected declaration")
                    .build());
            }
            if !matches!(
                (protection, &value),
                (ValueProtection::SecretUtf8, SecretValue::String(_))
                    | (ValueProtection::SecretBytes, SecretValue::Bytes(_))
            ) {
                return Err(ValidationError::builder("secret.domain_mismatch")
                    .at(path.clone())
                    .message("protected input has a different declared domain")
                    .build());
            }
            Ok(ValueTree::Secret(value))
        },
        ValueTree::Object(mut values) => {
            if let Some(core) = core
                && let Body::Record { properties, .. } = body(graph, core)?
            {
                for property in properties {
                    let mut selected = values.shift_remove(property.key.as_str());
                    for alias in &property.aliases.read {
                        let alias_value = values.shift_remove(alias.as_str());
                        if selected.is_none() {
                            selected = alias_value;
                        }
                    }
                    if selected.is_none()
                        && !symbolic_paths.contains(&path.push(property.key.as_str()))
                        && let Some(default) = &property.input_default
                    {
                        selected = Some(AuthoredValue::from_data(default.clone())?);
                    }
                    if let Some(selected) = selected {
                        values.insert(property.key.as_str().to_owned(), selected);
                    }
                }
            }
            let body_scope = core.map(|core| body(graph, core)).transpose()?;
            let adjacent = adjacent_payload(body_scope, &values);
            let mut output = indexmap::IndexMap::with_capacity(values.len());
            for (key, value) in values {
                let child = adjacent
                    .filter(|(content, _)| *content == key)
                    .map(|(_, core)| core)
                    .or_else(|| object_child(body_scope, &key));
                let scope = PreparationScope {
                    inherited_forbidden: descendants_forbidden,
                    evaluated: scope.evaluated,
                    protection: ValueProtection::Public,
                };
                output.insert(
                    key.clone(),
                    prepare(
                        graph,
                        child,
                        value,
                        &path.push(&key),
                        scope,
                        expressions,
                        symbolic_paths,
                    )?,
                );
            }
            Ok(ValueTree::Object(output))
        },
        ValueTree::List(values) => {
            let child = match core.map(|core| body(graph, core)).transpose()? {
                Some(Body::Array(array)) => Some(&array.element.0),
                _ => None,
            };
            let scope = PreparationScope {
                inherited_forbidden: descendants_forbidden,
                evaluated: scope.evaluated,
                protection: ValueProtection::Public,
            };
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    prepare(
                        graph,
                        child,
                        value,
                        &path.push(index.to_string()),
                        scope,
                        expressions,
                        symbolic_paths,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ValueTree::List)
        },
        ValueTree::Expression(_) => unreachable_expression(path),
    }
}

fn unreachable_expression(path: &ValuePath) -> Result<CompiledValue, ValidationError> {
    Err(ValidationError::builder("expression.forbidden")
        .at(path.clone())
        .message("unexpected authored expression during input preparation")
        .build())
}

fn prepare_scalar_facets(
    core: &UseSiteCore,
    value: AuthoredValue,
    path: &ValuePath,
) -> Result<AuthoredValue, ValidationError> {
    let (mut data, already_protected) = match value {
        ValueTree::Literal(scalar) => (scalar.into_json(), false),
        ValueTree::Secret(SecretValue::String(secret)) if !core.transformers.is_empty() => {
            (Value::String(secret.expose().to_owned()), true)
        },
        value => return Ok(value),
    };
    for transformer in &core.transformers {
        let next = transformer.apply(&data);
        if let Value::String(text) = &mut data {
            text.zeroize();
        }
        data = next;
    }
    if already_protected {
        return match data {
            Value::String(text) => Ok(ValueTree::Secret(SecretValue::string(text))),
            _ => Err(ValidationError::builder("type_mismatch")
                .at(path.clone())
                .message("protected text transformation changed its domain")
                .build()),
        };
    }
    match (core.protection, data) {
        (ValueProtection::SecretUtf8, Value::String(text)) => {
            Ok(ValueTree::Secret(SecretValue::string(text)))
        },
        (ValueProtection::SecretBytes, Value::String(mut text)) => {
            if !super::admission::is_canonical_base64(&text) {
                text.zeroize();
                return Err(ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message("protected bytes require canonical base64 data")
                    .build());
            }
            let bytes = STANDARD.decode(text.as_bytes()).map_err(|_| {
                ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message("protected bytes require canonical base64 data")
                    .build()
            });
            text.zeroize();
            bytes.map(|bytes| ValueTree::Secret(SecretValue::bytes(bytes)))
        },
        (_, data) => ScalarValue::try_from(data).map(ValueTree::Literal),
    }
}

fn object_child<'a>(body: Option<&'a Body>, key: &str) -> Option<&'a UseSiteCore> {
    match body {
        Some(Body::Record {
            properties,
            additional_properties,
            ..
        }) => properties
            .iter()
            .find(|property| property.key.as_str() == key)
            .map(|property| &property.core)
            .or_else(|| match additional_properties {
                AdditionalProperties::Typed(core) => Some(core.as_ref()),
                _ => None,
            }),
        Some(Body::Union(union)) if union.tagging == SerdeTagging::External => union
            .variants
            .iter()
            .find(|variant| variant.key.as_str() == key)
            .and_then(|variant| variant.payload.as_ref())
            .map(|payload| &payload.0),
        _ => None,
    }
}

fn adjacent_payload<'a, E>(
    body: Option<&'a Body>,
    values: &indexmap::IndexMap<String, ValueTree<E>>,
) -> Option<(&'a str, &'a UseSiteCore)> {
    let Some(Body::Union(union)) = body else {
        return None;
    };
    let SerdeTagging::Adjacent { tag, content } = &union.tagging else {
        return None;
    };
    let ValueTree::Literal(selector) = values.get(tag.as_str())? else {
        return None;
    };
    let selector = selector.as_json().as_str()?;
    let payload = union
        .variants
        .iter()
        .find(|variant| variant.key.as_str() == selector)?
        .payload
        .as_ref()?;
    Some((content.as_str(), &payload.0))
}

fn normalize_union(
    union: &super::model::UnionBody,
    value: AuthoredValue,
) -> Result<AuthoredValue, ValidationError> {
    let canonical = |selector: &str| {
        union
            .selector
            .aliases
            .iter()
            .find(|(alias, _)| alias.as_str() == selector)
            .map(|(_, target)| target.as_str().to_owned())
    };
    match value {
        ValueTree::Literal(value) if union.tagging == SerdeTagging::External => {
            if let Some(selector) = value.as_json().as_str()
                && let Some(target) = canonical(selector)
            {
                return Ok(ValueTree::Literal(ScalarValue::try_from(Value::String(
                    target,
                ))?));
            }
            Ok(ValueTree::Literal(value))
        },
        ValueTree::Object(mut values) => {
            match &union.tagging {
                SerdeTagging::External if values.len() == 1 => {
                    if let Some(key) = values.keys().next().cloned()
                        && let Some(target) = canonical(&key)
                        && let Some(value) = values.shift_remove(&key)
                    {
                        values.insert(target, value);
                    }
                },
                SerdeTagging::Adjacent { tag, .. } => {
                    if let Some(ValueTree::Literal(selector)) = values.get(tag.as_str())
                        && let Some(target) = selector.as_json().as_str().and_then(canonical)
                    {
                        values.insert(
                            tag.as_str().to_owned(),
                            ValueTree::Literal(ScalarValue::try_from(Value::String(target))?),
                        );
                    }
                },
                SerdeTagging::External => {},
            }
            Ok(ValueTree::Object(values))
        },
        value => Ok(value),
    }
}

type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ResolvedValue, ValidationError>> + Send + 'a>>;

fn resolve_node<'a>(
    graph: &'a AdmittedSchemaGraph,
    core: Option<&'a UseSiteCore>,
    value: CompiledValue,
    path: ValuePath,
    context: Option<&'a dyn ExpressionContext>,
) -> ResolveFuture<'a> {
    Box::pin(async move {
        match value {
            ValueTree::Literal(value) => Ok(ValueTree::Literal(value)),
            ValueTree::Secret(value) => Ok(ValueTree::Secret(value)),
            ValueTree::Expression(program) => {
                let context = context.ok_or_else(|| {
                    ValidationError::builder("expression.forbidden")
                        .at(path.clone())
                        .message("data-only resolution cannot evaluate programs")
                        .build()
                })?;
                let data = context.evaluate(&program).await.map_err(|cause| {
                    ValidationError::builder("expression.runtime")
                        .at(path.clone())
                        .message("input expression evaluation failed")
                        .private_source(cause)
                        .build()
                })?;
                let value = AuthoredValue::from_data(data)?;
                let value = prepare(
                    graph,
                    core,
                    value,
                    &path,
                    PreparationScope {
                        evaluated: true,
                        ..PreparationScope::default()
                    },
                    &mut Vec::new(),
                    &[],
                )?;
                Ok(literal_skeleton(&value))
            },
            ValueTree::Object(values) => {
                let mut scope = core.map(|core| body(graph, core)).transpose()?;
                while let Some(Body::Alias(alias)) = scope {
                    scope = Some(body(graph, &alias.0)?);
                }
                let adjacent = adjacent_payload(scope, &values);
                let mut output = indexmap::IndexMap::with_capacity(values.len());
                for (key, value) in values {
                    let child = adjacent
                        .filter(|(content, _)| *content == key)
                        .map(|(_, core)| core)
                        .or_else(|| object_child(scope, &key));
                    output.insert(
                        key.clone(),
                        resolve_node(graph, child, value, path.push(&key), context).await?,
                    );
                }
                Ok(ValueTree::Object(output))
            },
            ValueTree::List(values) => {
                let mut scope = core.map(|core| body(graph, core)).transpose()?;
                while let Some(Body::Alias(alias)) = scope {
                    scope = Some(body(graph, &alias.0)?);
                }
                let child = match scope {
                    Some(Body::Array(array)) => Some(&array.element.0),
                    _ => None,
                };
                let mut output = Vec::with_capacity(values.len());
                for (index, value) in values.into_iter().enumerate() {
                    output.push(
                        resolve_node(graph, child, value, path.push(index.to_string()), context)
                            .await?,
                    );
                }
                Ok(ValueTree::List(output))
            },
        }
    })
}
