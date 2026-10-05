//! Bounded runtime traversal of admitted graphs. Output never runs preparation.

use std::cmp::Ordering;

use nebula_validator::{DiagnosticDisclosure, ExecutionMode, PredicateContext, Rule};
use serde_json::Value;

use super::{
    AdmittedSchemaGraph,
    model::{
        AcceptedDomain, AdditionalProperties, Body, EmptyPolicy, NullPolicy, PresencePolicy,
        ValueProtection,
    },
    number::compare_numbers,
};
use crate::{
    MAX_VALUE_DEPTH, MAX_VALUE_NODES, MAX_VALUE_TEXT_BYTES, PendingValidation, SerdeTagging,
    ValidationError, ValidationReport, ValuePath,
};

impl AdmittedSchemaGraph {
    /// Reject protected output occurrences, including optional/inactive branches.
    ///
    /// # Errors
    /// Returns payload-free protected-domain diagnostics before handler execution.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn ensure_public_output_domain(&self) -> Result<(), ValidationReport> {
        let mut pending = vec![&self.0.graph.root.0];
        let mut visited = vec![false; self.0.graph.definitions.len()];
        while let Some(core) = pending.pop() {
            if core.protection != ValueProtection::Public {
                return Err(error("schema.output.protected_domain", &ValuePath::root()));
            }
            let index = self
                .0
                .lookup
                .get(&core.target)
                .ok_or_else(|| error("schema.graph.dangling_reference", &ValuePath::root()))?;
            if visited[index.0] {
                continue;
            }
            visited[index.0] = true;
            let definition = &self.0.graph.definitions[index.0];
            for edge in definition
                .edges()
                .map_err(|_| error("schema.graph.invalid_document", &ValuePath::root()))?
            {
                pending.push(
                    definition.use_for_edge(&edge).ok_or_else(|| {
                        error("schema.graph.invalid_document", &ValuePath::root())
                    })?,
                );
            }
        }
        Ok(())
    }
}

pub(super) fn validate_output(
    graph: &AdmittedSchemaGraph,
    candidate: &Value,
) -> Result<(), ValidationReport> {
    check_value_budget(candidate)?;
    let context = PredicateContext::from_json(candidate.clone());
    validate_literal(graph, candidate, &context, true, true, &[]).map(|_| ())
}

pub(super) fn check_value_budget(root: &Value) -> Result<(), ValidationReport> {
    let mut pending = vec![(root, 0_u16)];
    let mut nodes = 0_usize;
    let mut text = 0_usize;
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > MAX_VALUE_NODES || depth > u16::from(MAX_VALUE_DEPTH) {
            return Err(error("value.limit_exceeded", &ValuePath::root()));
        }
        match value {
            Value::String(value) => text = text.saturating_add(value.len()),
            Value::Array(values) => {
                if nodes
                    .saturating_add(pending.len())
                    .saturating_add(values.len())
                    > MAX_VALUE_NODES
                {
                    return Err(error("value.limit_exceeded", &ValuePath::root()));
                }
                pending.extend(values.iter().map(|value| (value, depth.saturating_add(1))));
            },
            Value::Object(values) => {
                if nodes
                    .saturating_add(pending.len())
                    .saturating_add(values.len())
                    > MAX_VALUE_NODES
                {
                    return Err(error("value.limit_exceeded", &ValuePath::root()));
                }
                for (key, value) in values {
                    text = text.saturating_add(key.len());
                    pending.push((value, depth.saturating_add(1)));
                }
            },
            _ => {},
        }
        if text > MAX_VALUE_TEXT_BYTES {
            return Err(error("value.limit_exceeded", &ValuePath::root()));
        }
    }
    Ok(())
}

pub(super) fn validate_literal(
    graph: &AdmittedSchemaGraph,
    candidate: &Value,
    context: &PredicateContext,
    outbound: bool,
    full: bool,
    expression_paths: &[ValuePath],
) -> Result<Vec<PendingValidation>, ValidationReport> {
    let mut pending = vec![(&graph.0.graph.root.0, candidate, ValuePath::root())];
    let mut obligations = Vec::new();
    let mut steps = 0_usize;
    while let Some((core, value, path)) = pending.pop() {
        steps = steps.saturating_add(1);
        if steps > MAX_VALUE_NODES.saturating_mul(graph.definition_count().saturating_add(1)) {
            return Err(error("value.limit_exceeded", &path));
        }
        if expression_paths.contains(&path) {
            obligations.push(PendingValidation::Value { path });
            continue;
        }
        rules(
            &core.rules,
            value,
            context,
            &path,
            full,
            &mut obligations,
            expression_paths,
        )?;
        let incomplete = !full
            && expression_paths
                .iter()
                .any(|unavailable| unavailable.starts_with(&path));
        if incomplete && matches!(&core.accepted_domain, AcceptedDomain::Closed(_)) {
            obligations.push(PendingValidation::Policy { path: path.clone() });
        } else if matches!(&core.accepted_domain, AcceptedDomain::Closed(values) if !values.contains(value))
        {
            return Err(error("option.invalid", &path));
        }
        if value.is_null() {
            let rejected = match &core.null {
                NullPolicy::Allow => false,
                NullPolicy::Reject => true,
                NullPolicy::RejectWhen(rule) => {
                    matches(rule, context, &path, full, &mut obligations)?
                },
            };
            if rejected {
                return Err(error("value.null_rejected", &path));
            }
            let index = graph
                .0
                .lookup
                .get(&core.target)
                .ok_or_else(|| error("schema.graph.dangling_reference", &path))?;
            if let Body::Alias(alias) = &graph.0.graph.definitions[index.0].body {
                pending.push((&alias.0, value, path));
            }
            continue;
        }
        if value.as_str() == Some("")
            && empty_rejected(&core.empty_string, context, &path, full, &mut obligations)?
        {
            return Err(error("value.empty_rejected", &path));
        }
        if (value.as_array().is_some_and(Vec::is_empty)
            || value.as_object().is_some_and(serde_json::Map::is_empty))
            && empty_rejected(
                &core.empty_collection,
                context,
                &path,
                full,
                &mut obligations,
            )?
        {
            return Err(error("value.empty_rejected", &path));
        }
        let index = graph
            .0
            .lookup
            .get(&core.target)
            .ok_or_else(|| error("schema.graph.dangling_reference", &path))?;
        match &graph.0.graph.definitions[index.0].body {
            Body::Any => {},
            Body::Null => return Err(error("type_mismatch", &path)),
            Body::Boolean { intrinsic_rules } => {
                if !value.is_boolean() {
                    return Err(error("type_mismatch", &path));
                }
                rules(
                    intrinsic_rules,
                    value,
                    context,
                    &path,
                    full,
                    &mut obligations,
                    expression_paths,
                )?;
            },
            Body::String { intrinsic_rules } => {
                if !value.is_string() {
                    return Err(error("type_mismatch", &path));
                }
                rules(
                    intrinsic_rules,
                    value,
                    context,
                    &path,
                    full,
                    &mut obligations,
                    expression_paths,
                )?;
            },
            Body::Bytes => {
                if !value
                    .as_str()
                    .is_some_and(super::admission::is_canonical_base64)
                {
                    return Err(error("type_mismatch", &path));
                }
            },
            Body::Integer(number) | Body::Number(number) => {
                let Some(actual) = value.as_number() else {
                    return Err(error("type_mismatch", &path));
                };
                if matches!(&graph.0.graph.definitions[index.0].body, Body::Integer(_))
                    && !actual.is_i64()
                    && !actual.is_u64()
                {
                    return Err(error("type_mismatch", &path));
                }
                if number.minimum.as_ref().is_some_and(|minimum| {
                    compare_numbers(actual, minimum).is_ok_and(|order| order == Ordering::Less)
                }) {
                    return Err(error("min", &path));
                }
                if number.maximum.as_ref().is_some_and(|maximum| {
                    compare_numbers(actual, maximum).is_ok_and(|order| order == Ordering::Greater)
                }) {
                    return Err(error("max", &path));
                }
                rules(
                    &number.intrinsic_rules,
                    value,
                    context,
                    &path,
                    full,
                    &mut obligations,
                    expression_paths,
                )?;
            },
            Body::Alias(alias) => pending.push((&alias.0, value, path)),
            Body::Array(array) => {
                let Some(values) = value.as_array() else {
                    return Err(error("type_mismatch", &path));
                };
                if values.len() < array.min_items as usize
                    || array
                        .max_items
                        .is_some_and(|max| values.len() > max as usize)
                {
                    return Err(error("value.array_bounds", &path));
                }
                if array.unique && incomplete {
                    obligations.push(PendingValidation::Policy { path: path.clone() });
                } else if array.unique
                    && values
                        .iter()
                        .enumerate()
                        .any(|(index, value)| values[..index].contains(value))
                {
                    return Err(error("value.array_unique", &path));
                }
                rules(
                    &array.intrinsic_rules,
                    value,
                    context,
                    &path,
                    full,
                    &mut obligations,
                    expression_paths,
                )?;
                pending.extend(
                    values.iter().enumerate().map(|(index, value)| {
                        (&array.element.0, value, path.push(index.to_string()))
                    }),
                );
            },
            Body::Record {
                properties,
                additional_properties,
                intrinsic_rules,
            } => {
                let Some(values) = value.as_object() else {
                    return Err(error("type_mismatch", &path));
                };
                rules(
                    intrinsic_rules,
                    value,
                    context,
                    &path,
                    full,
                    &mut obligations,
                    expression_paths,
                )?;
                for property in properties {
                    let key = if outbound {
                        property.aliases.write.as_ref().unwrap_or(&property.key)
                    } else {
                        &property.key
                    };
                    let child_path = path.push(key.as_str());
                    if expression_paths.contains(&child_path) {
                        obligations.push(PendingValidation::Value { path: child_path });
                        continue;
                    }
                    if let Some(value) = values.get(key.as_str()) {
                        pending.push((&property.core, value, child_path));
                    } else {
                        let required = match &property.presence {
                            PresencePolicy::Required => true,
                            PresencePolicy::Optional => false,
                            PresencePolicy::RequiredWhen(rule) => {
                                matches(rule, context, &child_path, full, &mut obligations)?
                            },
                        };
                        if required {
                            return Err(error("required", &child_path));
                        }
                    }
                }
                for (key, value) in values {
                    if properties.iter().any(|property| {
                        let canonical = if outbound {
                            property.aliases.write.as_ref().unwrap_or(&property.key)
                        } else {
                            &property.key
                        };
                        canonical.as_str() == key
                    }) {
                        continue;
                    }
                    match additional_properties {
                        AdditionalProperties::Open => {},
                        AdditionalProperties::Closed => {
                            return Err(error("value.undeclared", &path.push(key)));
                        },
                        AdditionalProperties::Typed(core) => {
                            pending.push((core, value, path.push(key)));
                        },
                    }
                }
            },
            Body::Union(union) => {
                let (selector, payload) = match &union.tagging {
                    SerdeTagging::External => match value {
                        Value::String(selector) => (selector.as_str(), None),
                        Value::Object(values) if values.len() == 1 => {
                            let Some((selector, value)) = values.iter().next() else {
                                return Err(error("union.malformed", &path));
                            };
                            (selector.as_str(), Some((value, path.push(selector))))
                        },
                        _ => return Err(error("union.malformed", &path)),
                    },
                    SerdeTagging::Adjacent { tag, content } => {
                        let Some(values) = value.as_object() else {
                            return Err(error("union.malformed", &path));
                        };
                        let Some(selector) = values.get(tag).and_then(Value::as_str) else {
                            return Err(error("union.malformed", &path));
                        };
                        if values.len() != 1 + usize::from(values.contains_key(content)) {
                            return Err(error("union.malformed", &path));
                        }
                        (
                            selector,
                            values.get(content).map(|value| (value, path.push(content))),
                        )
                    },
                };
                let Some(variant) = union
                    .variants
                    .iter()
                    .find(|variant| variant.key.as_str() == selector)
                else {
                    return Err(error("union.unknown_variant", &path));
                };
                match (&variant.payload, payload) {
                    (Some(core), Some((value, path))) => pending.push((&core.0, value, path)),
                    (None, None) => {},
                    _ => return Err(error("union.malformed", &path)),
                }
            },
        }
    }
    Ok(obligations)
}

fn empty_rejected(
    policy: &EmptyPolicy,
    context: &PredicateContext,
    path: &ValuePath,
    full: bool,
    obligations: &mut Vec<PendingValidation>,
) -> Result<bool, ValidationReport> {
    match policy {
        EmptyPolicy::Allow => Ok(false),
        EmptyPolicy::Reject => Ok(true),
        EmptyPolicy::RejectWhen(rule) => matches(rule, context, path, full, obligations),
    }
}

fn matches(
    rule: &Rule,
    context: &PredicateContext,
    path: &ValuePath,
    full: bool,
    obligations: &mut Vec<PendingValidation>,
) -> Result<bool, ValidationReport> {
    match rule.matches(context) {
        Ok(value) => Ok(value),
        Err(cause)
            if !full
                && cause.kind()
                    == nebula_validator::foundation::ValidationErrorKind::Unavailable =>
        {
            obligations.push(PendingValidation::Policy { path: path.clone() });
            Ok(false)
        },
        Err(cause) => Err(native_error(cause, path)),
    }
}

fn rules(
    rules: &[Rule],
    value: &Value,
    context: &PredicateContext,
    path: &ValuePath,
    full: bool,
    obligations: &mut Vec<PendingValidation>,
    unavailable: &[ValuePath],
) -> Result<(), ValidationReport> {
    if !full
        && !rules.is_empty()
        && unavailable
            .iter()
            .any(|unavailable| unavailable.starts_with(path))
    {
        obligations.push(PendingValidation::Policy { path: path.clone() });
        return Ok(());
    }
    for rule in rules {
        let outcome = rule
            .validate(
                value,
                Some(context),
                if full {
                    ExecutionMode::Full
                } else {
                    ExecutionMode::StaticOnly
                },
                DiagnosticDisclosure::OmitValue,
            )
            .map_err(|cause| native_error(cause, path))?;
        match outcome {
            nebula_validator::EvaluationOutcome::Satisfied => {},
            nebula_validator::EvaluationOutcome::Deferred(reasons) if !full => {
                obligations.extend(reasons.into_iter().map(|reason| PendingValidation::Rule {
                    path: path.clone(),
                    reason,
                }));
            },
            _ => return Err(error("validation.incomplete", path)),
        }
    }
    Ok(())
}

fn native_error(
    cause: nebula_validator::foundation::ValidationError,
    path: &ValuePath,
) -> ValidationReport {
    ValidationError::builder(cause.code.to_string())
        .at(path.clone())
        .message("schema value rule or condition failed")
        .private_source(cause)
        .build()
        .into()
}

pub(super) fn error(code: &'static str, path: &ValuePath) -> ValidationReport {
    ValidationError::builder(code)
        .at(path.clone())
        .message("schema graph value admission failed")
        .build()
        .into()
}
