//! Facet applicability and literal matching against resolved bodies.

use std::cmp::Ordering;

use serde_json::Value;

use nebula_validator::{DiagnosticDisclosure, ExecutionMode, Rule, RuleView};

use crate::{ExpressionMode, SerdeTagging};

use super::super::{
    model::{
        AcceptedDomain, AdditionalProperties, AdmissionIssue, ArrayBody, Body, DefinitionKey,
        DefinitionLookup, DraftGraph, EmptyPolicy, NullPolicy, NumericBody, PresencePolicy,
        PropertyUse, UnionBody, UseSiteCore, ValueProtection,
    },
    number::compare_numbers,
};

use super::shape::presence_is_always_required;

use super::rules::{
    check_condition, check_intrinsic_rules, check_numeric, check_rules, check_selector,
};

pub(super) fn check_facet_applicability(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<(), AdmissionIssue> {
    check_use_facets(graph, lookup, &graph.root.0)?;
    for definition in &graph.definitions {
        check_intrinsic_rules(&definition.body)?;
        if let Body::Record { properties, .. } = &definition.body {
            for property in properties {
                if let PresencePolicy::RequiredWhen(rule) = &property.presence {
                    check_condition(rule)?;
                }
                if let Some(default) = &property.input_default {
                    check_input_default(graph, lookup, property, default)?;
                }
            }
        }
        match &definition.body {
            Body::Integer(number) => {
                check_numeric(number, true)?;
            },
            Body::Number(number) => {
                check_numeric(number, false)?;
            },
            Body::Array(array)
                if array
                    .max_items
                    .is_some_and(|maximum| array.min_items > maximum) =>
            {
                return Err(AdmissionIssue::InvalidBounds);
            },
            Body::Union(union) => check_selector(union)?,
            _ => {},
        }
        for edge in definition.edges()? {
            let core = definition
                .use_for_edge(&edge)
                .ok_or(AdmissionIssue::InvalidDocument)?;
            check_use_facets(graph, lookup, core)?;
        }
    }
    Ok(())
}

fn resolved_body<'a>(
    graph: &'a DraftGraph,
    lookup: &DefinitionLookup,
    target: &DefinitionKey,
) -> Result<&'a Body, AdmissionIssue> {
    let mut current = *lookup
        .get(target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    for _ in 0..graph.definitions.len() {
        let body = &graph.definitions[current.0].body;
        match body {
            Body::Alias(alias) => {
                current = *lookup
                    .get(&alias.0.target)
                    .ok_or(AdmissionIssue::DanglingReference)?;
            },
            _ => return Ok(body),
        }
    }
    Err(AdmissionIssue::NonproductiveDefinition)
}

fn check_use_facets(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    core: &UseSiteCore,
) -> Result<(), AdmissionIssue> {
    if let NullPolicy::RejectWhen(rule) = &core.null {
        check_condition(rule)?;
    }
    if let EmptyPolicy::RejectWhen(rule) = &core.empty_string {
        check_condition(rule)?;
    }
    if let EmptyPolicy::RejectWhen(rule) = &core.empty_collection {
        check_condition(rule)?;
    }
    let needs_target = !matches!(core.empty_string, EmptyPolicy::Allow)
        || !matches!(core.empty_collection, EmptyPolicy::Allow)
        || !core.transformers.is_empty()
        || !core.rules.is_empty()
        || core.protection != ValueProtection::Public
        || matches!(core.accepted_domain, AcceptedDomain::Closed(_));
    if !needs_target {
        return Ok(());
    }
    let target = resolved_body(graph, lookup, &core.target)?;
    match core.protection {
        ValueProtection::Public => {},
        ValueProtection::SecretUtf8 if matches!(target, Body::String { .. }) => {},
        ValueProtection::SecretBytes if matches!(target, Body::Bytes) => {},
        ValueProtection::SecretUtf8 | ValueProtection::SecretBytes => {
            return Err(AdmissionIssue::InapplicableFacet);
        },
    }
    if let AcceptedDomain::Closed(values) = &core.accepted_domain {
        if occurrence_contains_protected(graph, lookup, core)? || matches!(target, Body::Bytes) {
            return Err(AdmissionIssue::InapplicableFacet);
        }
        for value in values {
            if !literal_matches_use(graph, lookup, core, value, LiteralPurpose::AcceptedDomain)? {
                return Err(AdmissionIssue::InapplicableFacet);
            }
        }
    }
    if matches!(
        core.empty_string,
        EmptyPolicy::Reject | EmptyPolicy::RejectWhen(_)
    ) && !matches!(target, Body::String { .. } | Body::Bytes)
    {
        return Err(AdmissionIssue::InapplicableFacet);
    }
    if matches!(
        core.empty_collection,
        EmptyPolicy::Reject | EmptyPolicy::RejectWhen(_)
    ) && !matches!(target, Body::Record { .. } | Body::Array(_))
    {
        return Err(AdmissionIssue::InapplicableFacet);
    }
    if !core.transformers.is_empty() && !matches!(target, Body::String { .. }) {
        return Err(AdmissionIssue::InapplicableFacet);
    }
    check_rules(&core.rules, target, false)?;
    Ok(())
}

fn check_input_default(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    property: &PropertyUse,
    default: &Value,
) -> Result<(), AdmissionIssue> {
    if property.core.expression == ExpressionMode::Required
        || occurrence_contains_protected(graph, lookup, &property.core)?
        || !literal_matches_use(
            graph,
            lookup,
            &property.core,
            default,
            LiteralPurpose::InputDefault,
        )?
    {
        return Err(AdmissionIssue::InvalidDefault);
    }
    Ok(())
}

fn occurrence_contains_protected(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    core: &UseSiteCore,
) -> Result<bool, AdmissionIssue> {
    if core.protection != ValueProtection::Public {
        return Ok(true);
    }
    let root = *lookup
        .get(&core.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut visited = vec![false; graph.definitions.len()];
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if visited[index.0] {
            continue;
        }
        visited[index.0] = true;
        for edge in graph.definitions[index.0].edges()? {
            let edge_core = graph.definitions[index.0]
                .use_for_edge(&edge)
                .ok_or(AdmissionIssue::InvalidDocument)?;
            if edge_core.protection != ValueProtection::Public {
                return Ok(true);
            }
            pending.push(
                *lookup
                    .get(&edge.target)
                    .ok_or(AdmissionIssue::DanglingReference)?,
            );
        }
    }
    Ok(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiteralPurpose {
    AcceptedDomain,
    InputDefault,
}

fn literal_matches_use(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    core: &UseSiteCore,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let mut current_value = literal_with_use_transformers(core, value, purpose);
    if !literal_matches_use_facets(core, &current_value, purpose)? {
        return Ok(false);
    }
    let mut current = *lookup
        .get(&core.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut visited = vec![false; graph.definitions.len()];
    loop {
        if visited[current.0] {
            return Err(AdmissionIssue::NonproductiveDefinition);
        }
        visited[current.0] = true;
        let body = &graph.definitions[current.0].body;
        if let Body::Alias(alias) = body {
            current_value = literal_with_use_transformers(&alias.0, &current_value, purpose);
            if !literal_matches_use_facets(&alias.0, &current_value, purpose)? {
                return Ok(false);
            }
            current = *lookup
                .get(&alias.0.target)
                .ok_or(AdmissionIssue::DanglingReference)?;
            continue;
        }
        return literal_matches_body(graph, lookup, body, &current_value, purpose);
    }
}

fn literal_with_use_transformers(
    core: &UseSiteCore,
    value: &Value,
    purpose: LiteralPurpose,
) -> Value {
    if purpose != LiteralPurpose::InputDefault {
        return value.clone();
    }
    core.transformers
        .iter()
        .fold(value.clone(), |current, transformer| {
            transformer.apply(&current)
        })
}

fn literal_matches_use_facets(
    core: &UseSiteCore,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    Ok(
        !(purpose == LiteralPurpose::InputDefault && core.expression == ExpressionMode::Required)
            && literal_matches_occurrence_policies(core, value)?
            && !matches!(&core.accepted_domain, AcceptedDomain::Closed(values) if !values.contains(value))
            && rules_accept(&core.rules, value)?,
    )
}

fn literal_matches_occurrence_policies(
    core: &UseSiteCore,
    value: &Value,
) -> Result<bool, AdmissionIssue> {
    if value.is_null() {
        return Ok(matches!(core.null, NullPolicy::Allow));
    }
    if value.as_str() == Some("") && !matches!(core.empty_string, EmptyPolicy::Allow) {
        return Ok(false);
    }
    if matches!(value, Value::Array(values) if values.is_empty())
        || matches!(value, Value::Object(values) if values.is_empty())
    {
        return Ok(matches!(core.empty_collection, EmptyPolicy::Allow));
    }
    Ok(true)
}

fn literal_matches_body(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    body: &Body,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let shape_matches = match body {
        Body::Any => true,
        Body::Null => value.is_null(),
        Body::Boolean { .. } => value.is_boolean(),
        Body::Integer(number) => {
            value
                .as_number()
                .is_some_and(|value| value.is_i64() || value.is_u64())
                && numeric_value_in_bounds(value, number)?
        },
        Body::Number(number) => value.is_number() && numeric_value_in_bounds(value, number)?,
        Body::String { .. } => value.is_string(),
        Body::Bytes => value.as_str().is_some_and(is_canonical_base64),
        Body::Record {
            properties,
            additional_properties,
            ..
        } => record_literal_matches(
            graph,
            lookup,
            properties,
            additional_properties,
            value,
            purpose,
        )?,
        Body::Array(array) => array_literal_matches(graph, lookup, array, value, purpose)?,
        Body::Union(union) => union_literal_matches(graph, lookup, union, value, purpose)?,
        Body::Alias(_) => return Err(AdmissionIssue::InvalidDocument),
    };
    Ok(shape_matches && rules_accept(intrinsic_rules(body), value)?)
}

fn intrinsic_rules(body: &Body) -> &[Rule] {
    match body {
        Body::Boolean { intrinsic_rules } | Body::String { intrinsic_rules } => intrinsic_rules,
        Body::Integer(number) | Body::Number(number) => &number.intrinsic_rules,
        Body::Record {
            intrinsic_rules, ..
        } => intrinsic_rules,
        Body::Array(array) => &array.intrinsic_rules,
        Body::Any | Body::Null | Body::Bytes | Body::Union(_) | Body::Alias(_) => &[],
    }
}

fn numeric_value_in_bounds(value: &Value, body: &NumericBody) -> Result<bool, AdmissionIssue> {
    let Some(value) = value.as_number() else {
        return Ok(false);
    };
    if let Some(minimum) = &body.minimum
        && compare_numbers(value, minimum)? == Ordering::Less
    {
        return Ok(false);
    }
    if let Some(maximum) = &body.maximum
        && compare_numbers(value, maximum)? == Ordering::Greater
    {
        return Ok(false);
    }
    Ok(true)
}

fn record_literal_matches(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    properties: &[PropertyUse],
    additional_properties: &AdditionalProperties,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let Some(object) = value.as_object() else {
        return Ok(false);
    };
    for property in properties {
        match object.get(property.key.as_str()) {
            Some(value) if !literal_matches_use(graph, lookup, &property.core, value, purpose)? => {
                return Ok(false);
            },
            None if presence_is_always_required(&property.presence)?
                && property.input_default.is_none() =>
            {
                return Ok(false);
            },
            Some(_) | None => {},
        }
    }
    for (key, value) in object {
        if properties
            .iter()
            .any(|property| property.key.as_str() == key)
        {
            continue;
        }
        match additional_properties {
            AdditionalProperties::Open => {},
            AdditionalProperties::Closed => return Ok(false),
            AdditionalProperties::Typed(core)
                if literal_matches_use(graph, lookup, core, value, purpose)? => {},
            AdditionalProperties::Typed(_) => return Ok(false),
        }
    }
    Ok(true)
}

fn array_literal_matches(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    array: &ArrayBody,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let Some(values) = value.as_array() else {
        return Ok(false);
    };
    let len = u64::try_from(values.len()).map_err(|_| AdmissionIssue::IndexOverflow)?;
    if len < u64::from(array.min_items)
        || array
            .max_items
            .is_some_and(|maximum| len > u64::from(maximum))
        || array.unique
            && values
                .iter()
                .enumerate()
                .any(|(index, value)| values[..index].contains(value))
    {
        return Ok(false);
    }
    for value in values {
        if !literal_matches_use(graph, lookup, &array.element.0, value, purpose)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn union_literal_matches(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    union: &UnionBody,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    match &union.tagging {
        SerdeTagging::External => match value {
            Value::String(selector) => Ok(union
                .variants
                .iter()
                .any(|variant| variant.key.as_str() == selector && variant.payload.is_none())),
            Value::Object(object) if object.len() == 1 => {
                let Some((selector, payload_value)) = object.iter().next() else {
                    return Ok(false);
                };
                let Some(payload) = union
                    .variants
                    .iter()
                    .find(|variant| variant.key.as_str() == selector)
                    .and_then(|variant| variant.payload.as_ref())
                else {
                    return Ok(false);
                };
                literal_matches_use(graph, lookup, &payload.0, payload_value, purpose)
            },
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => Ok(false),
        },
        SerdeTagging::Adjacent { tag, content } => {
            let Some(object) = value.as_object() else {
                return Ok(false);
            };
            let Some(selector) = object.get(tag).and_then(Value::as_str) else {
                return Ok(false);
            };
            let Some(variant) = union
                .variants
                .iter()
                .find(|variant| variant.key.as_str() == selector)
            else {
                return Ok(false);
            };
            match &variant.payload {
                Some(payload) => {
                    if object.len() != 2 {
                        return Ok(false);
                    }
                    let Some(value) = object.get(content) else {
                        return Ok(false);
                    };
                    literal_matches_use(graph, lookup, &payload.0, value, purpose)
                },
                None => Ok(object.len() == 1 && !object.contains_key(content)),
            }
        },
    }
}

fn rules_accept(rules: &[Rule], value: &Value) -> Result<bool, AdmissionIssue> {
    for rule in rules {
        let mut pending = vec![rule.root()];
        while let Some(current) = pending.pop() {
            match current.view() {
                RuleView::Value(_) => {},
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Predicate(_) | RuleView::Deferred(_) => return Ok(false),
                _ => return Err(AdmissionIssue::InvalidRule),
            }
        }
        if rule
            .validate(
                value,
                None,
                ExecutionMode::StaticOnly,
                DiagnosticDisclosure::OmitValue,
            )
            .and_then(nebula_validator::EvaluationOutcome::require_satisfied)
            .is_err()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn is_canonical_base64(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    if bytes.is_empty() {
        return true;
    }
    let padding = usize::from(bytes.ends_with(b"=")) + usize::from(bytes.ends_with(b"=="));
    let payload_len = bytes.len() - padding;
    if bytes[..payload_len]
        .iter()
        .any(|byte| base64_value(*byte).is_none())
        || bytes[payload_len..].iter().any(|byte| *byte != b'=')
    {
        return false;
    }
    match padding {
        0 => true,
        1 => base64_value(bytes[payload_len - 1]).is_some_and(|value| value.trailing_zeros() >= 2),
        2 => base64_value(bytes[payload_len - 1]).is_some_and(|value| value.trailing_zeros() >= 4),
        _ => false,
    }
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
