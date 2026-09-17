//! Intrinsic-rule, condition, numeric, selector, numbering, and address checks.

use std::collections::VecDeque;

use serde_json::Value;

use nebula_validator::{Rule, RuleView, ValueRule};

use crate::FieldKey;

use super::super::{
    model::{
        AdditionalProperties, AdmissionIssue, Body, DefinitionIndex, DefinitionLookup, DraftGraph,
        Edge, NumericBody, UnionBody,
    },
    number::compare_numbers,
};

use super::facets::is_canonical_base64;
use super::keys::DeclarationUse;

pub(super) fn check_condition(rule: &Rule) -> Result<(), AdmissionIssue> {
    let mut pending = vec![rule.root()];
    while let Some(current) = pending.pop() {
        match current.view() {
            RuleView::Predicate(_) => {},
            RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
            RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
            RuleView::Value(_) | RuleView::Deferred(_) => {
                return Err(AdmissionIssue::InapplicableFacet);
            },
            _ => return Err(AdmissionIssue::InapplicableFacet),
        }
    }
    Ok(())
}

pub(super) fn check_intrinsic_rules(body: &Body) -> Result<(), AdmissionIssue> {
    let rules = match body {
        Body::Boolean { intrinsic_rules } | Body::String { intrinsic_rules } => intrinsic_rules,
        Body::Integer(number) | Body::Number(number) => &number.intrinsic_rules,
        Body::Record {
            intrinsic_rules, ..
        } => intrinsic_rules,
        Body::Array(array) => &array.intrinsic_rules,
        Body::Any | Body::Null | Body::Bytes | Body::Union(_) | Body::Alias(_) => return Ok(()),
    };
    check_rules(rules, body, true)
}

pub(super) fn check_rules(
    rules: &[Rule],
    target: &Body,
    context_free: bool,
) -> Result<(), AdmissionIssue> {
    for rule in rules {
        let mut pending = vec![rule.root()];
        while let Some(current) = pending.pop() {
            match current.view() {
                RuleView::Value(value) if value_rule_applies(value, target) => {},
                RuleView::Value(_) => return Err(AdmissionIssue::InapplicableFacet),
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Predicate(_) | RuleView::Deferred(_) if context_free => {
                    return Err(AdmissionIssue::InapplicableFacet);
                },
                RuleView::Predicate(_) | RuleView::Deferred(_) => {},
                _ => return Err(AdmissionIssue::InapplicableFacet),
            }
        }
    }
    Ok(())
}

fn value_rule_applies(rule: &ValueRule, target: &Body) -> bool {
    match (target, rule) {
        (
            Body::String { .. },
            ValueRule::MinLength(_)
            | ValueRule::MaxLength(_)
            | ValueRule::Pattern(_)
            | ValueRule::Email
            | ValueRule::Url,
        ) => true,
        (
            Body::Integer(_) | Body::Number(_),
            ValueRule::Min(_)
            | ValueRule::Max(_)
            | ValueRule::GreaterThan(_)
            | ValueRule::LessThan(_),
        ) => true,
        (Body::Array(_), ValueRule::MinItems(_) | ValueRule::MaxItems(_)) => true,
        (body, ValueRule::OneOf(values)) => {
            values.iter().all(|value| value_matches_body(value, body))
        },
        _ => false,
    }
}

fn value_matches_body(value: &Value, body: &Body) -> bool {
    match body {
        Body::Any => true,
        Body::Null => value.is_null(),
        Body::Boolean { .. } => value.is_boolean(),
        Body::Integer(_) => value
            .as_number()
            .is_some_and(|number| number.is_i64() || number.is_u64()),
        Body::Number(_) => value.is_number(),
        Body::String { .. } => value.is_string(),
        Body::Bytes => value.as_str().is_some_and(is_canonical_base64),
        Body::Record { .. } => value.is_object(),
        Body::Array(_) => value.is_array(),
        Body::Union(_) | Body::Alias(_) => true,
    }
}

pub(super) fn check_numeric(number: &NumericBody, integer: bool) -> Result<(), AdmissionIssue> {
    if integer
        && number
            .minimum
            .iter()
            .chain(number.maximum.iter())
            .any(|value| !value.is_i64() && !value.is_u64())
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    if let (Some(minimum), Some(maximum)) = (&number.minimum, &number.maximum)
        && compare_numbers(minimum, maximum)?.is_gt()
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    Ok(())
}

pub(super) fn check_selector(union: &UnionBody) -> Result<(), AdmissionIssue> {
    let has_variant = |key: &FieldKey| union.variants.iter().any(|variant| &variant.key == key);
    if union
        .selector
        .default_variant
        .as_ref()
        .is_some_and(|key| !has_variant(key))
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    if union
        .selector
        .aliases
        .iter()
        .any(|(_, target)| !has_variant(target))
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    Ok(())
}

pub(super) fn canonical_numbering(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<Vec<u32>, AdmissionIssue> {
    let root = *lookup
        .get(&graph.root.0.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut numbers = vec![u32::MAX; graph.definitions.len()];
    numbers[root.0] = 0;
    let mut next = 1u32;
    let mut pending = VecDeque::from([root]);
    while let Some(index) = pending.pop_front() {
        let mut edges = graph.definitions[index.0].edges()?;
        edges.sort_unstable_by(Edge::compare);
        for edge in edges {
            let target = *lookup
                .get(&edge.target)
                .ok_or(AdmissionIssue::DanglingReference)?;
            if numbers[target.0] == u32::MAX {
                numbers[target.0] = next;
                next = next.checked_add(1).ok_or(AdmissionIssue::IndexOverflow)?;
                pending.push_back(target);
            }
        }
    }
    Ok(numbers)
}

pub(super) fn address_exists(
    graph: &DraftGraph,
    definition: DefinitionIndex,
    use_site: &DeclarationUse,
) -> bool {
    match use_site {
        DeclarationUse::Root => graph.root.0.target == graph.definitions[definition.0].key,
        DeclarationUse::Alias => matches!(graph.definitions[definition.0].body, Body::Alias(_)),
        DeclarationUse::Property(key) => {
            matches!(&graph.definitions[definition.0].body, Body::Record { properties, .. } if properties.iter().any(|property| property.key.as_str() == key.as_str()))
        },
        DeclarationUse::AdditionalProperty => {
            matches!(
                &graph.definitions[definition.0].body,
                Body::Record {
                    additional_properties: AdditionalProperties::Typed(_),
                    ..
                }
            )
        },
        DeclarationUse::Element => matches!(graph.definitions[definition.0].body, Body::Array(_)),
        DeclarationUse::Variant(key) => {
            matches!(&graph.definitions[definition.0].body, Body::Union(union) if union.variants.iter().any(|variant| variant.key.as_str() == key.as_str()))
        },
        DeclarationUse::VariantPayload(key) => {
            matches!(&graph.definitions[definition.0].body, Body::Union(union) if union.variants.iter().any(|variant| variant.key.as_str() == key.as_str() && variant.payload.is_some()))
        },
    }
}
