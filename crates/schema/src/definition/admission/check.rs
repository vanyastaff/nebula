//! Duplicate, local-key, budget, dangling-edge, reachability, and productivity checks.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use nebula_validator::Rule;

use crate::SerdeTagging;

use super::super::{
    MAX_GRAPH_DIAGNOSTICS, MAX_GRAPH_IDENTIFIER_BYTES, MAX_GRAPH_REFERENCES,
    model::{
        AdmissionDiagnostic, AdmissionIssue, AdmissionLocation, Body, Definition, DefinitionIndex,
        DefinitionLookup, DraftGraph, check_members, object,
    },
};

use super::AdmissionFailure;
use super::shape::{StructuralShape, body_shape, occurrence_shape};

pub(super) enum RejectionRule {
    Allow,
    Reject,
    RejectWhen(Rule),
}

pub(super) fn parse_rejection_rule(value: Option<&Value>) -> Result<RejectionRule, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(RejectionRule::Allow);
    };
    match value.as_str() {
        Some("allow") => Ok(RejectionRule::Allow),
        Some("reject") => Ok(RejectionRule::Reject),
        Some(_) => Err(AdmissionIssue::InvalidDocument),
        None => {
            let object = object(value)?;
            check_members(object, &["reject_when"])?;
            let rule = object
                .get("reject_when")
                .ok_or(AdmissionIssue::InvalidDocument)?;
            parse_rule(rule).map(RejectionRule::RejectWhen)
        },
    }
}

pub(super) fn parse_rule(value: &Value) -> Result<Rule, AdmissionIssue> {
    let rule: Rule =
        serde_json::from_value(value.clone()).map_err(|_| AdmissionIssue::InvalidRule)?;
    rule.check_limits()
        .map_err(|_| AdmissionIssue::InvalidRule)?;
    Ok(rule)
}

pub(super) fn check_duplicate_definitions(
    definitions: &[Definition],
) -> Result<(), AdmissionFailure> {
    if let Some(ordinal) = definitions
        .windows(2)
        .position(|pair| pair[0].key == pair[1].key)
    {
        return Err(AdmissionFailure(vec![AdmissionDiagnostic {
            issue: AdmissionIssue::DuplicateDefinition,
            location: AdmissionLocation::Definition {
                ordinal: u32::try_from(ordinal + 1).map_err(|_| AdmissionIssue::IndexOverflow)?,
            },
        }]));
    }
    Ok(())
}

pub(super) fn normalize_and_check_local(
    definitions: &mut [Definition],
) -> Result<(), AdmissionIssue> {
    for definition in definitions {
        match &mut definition.body {
            Body::Record { properties, .. } => {
                properties
                    .sort_unstable_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
                if properties.windows(2).any(|pair| pair[0].key == pair[1].key) {
                    return Err(AdmissionIssue::DuplicateLocalKey);
                }
                for property in &mut *properties {
                    let mut aliases = HashSet::new();
                    if property
                        .aliases
                        .read
                        .iter()
                        .any(|alias| !aliases.insert(alias.as_str()))
                    {
                        return Err(AdmissionIssue::DuplicateLocalKey);
                    }
                }
                let mut input_names = HashSet::new();
                let mut output_names = HashSet::new();
                for property in &*properties {
                    if !input_names.insert(property.key.as_str())
                        || property
                            .aliases
                            .read
                            .iter()
                            .any(|alias| !input_names.insert(alias.as_str()))
                    {
                        return Err(AdmissionIssue::DuplicateLocalKey);
                    }
                    let output = property.aliases.write.as_ref().unwrap_or(&property.key);
                    if !output_names.insert(output.as_str()) {
                        return Err(AdmissionIssue::DuplicateLocalKey);
                    }
                }
            },
            Body::Union(union) => {
                union
                    .variants
                    .sort_unstable_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
                union
                    .selector
                    .aliases
                    .sort_unstable_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
                if union
                    .variants
                    .windows(2)
                    .any(|pair| pair[0].key == pair[1].key)
                {
                    return Err(AdmissionIssue::DuplicateLocalKey);
                }
                if union
                    .selector
                    .aliases
                    .iter()
                    .any(|(alias, _)| union.variants.iter().any(|variant| variant.key == *alias))
                {
                    return Err(AdmissionIssue::DuplicateLocalKey);
                }
            },
            _ => {},
        }
    }
    Ok(())
}

pub(super) fn check_budgets_and_build_lookup(
    graph: &DraftGraph,
) -> Result<(DefinitionLookup, usize), AdmissionIssue> {
    let mut lookup = BTreeMap::new();
    let mut identifiers = graph.root.0.target.as_str().len();
    let mut references = 1usize;
    for (position, definition) in graph.definitions.iter().enumerate() {
        identifiers = checked_add(identifiers, definition.key.as_str().len())?;
        lookup.insert(definition.key.clone(), DefinitionIndex(position));
        for edge in definition.edges()? {
            references = checked_add(references, 1)?;
            identifiers = checked_add(identifiers, edge.target.as_str().len())?;
            if let Some(key) = edge.local_key {
                identifiers = checked_add(identifiers, key.as_str().len())?;
            }
        }
        identifiers = checked_add(identifiers, additional_identifier_bytes(&definition.body)?)?;
    }
    if references > MAX_GRAPH_REFERENCES {
        return Err(AdmissionIssue::ReferenceLimit);
    }
    if identifiers > MAX_GRAPH_IDENTIFIER_BYTES {
        return Err(AdmissionIssue::IdentifierBytesLimit);
    }
    Ok((lookup, references))
}

fn additional_identifier_bytes(body: &Body) -> Result<usize, AdmissionIssue> {
    let mut bytes = 0usize;
    match body {
        Body::Record { properties, .. } => {
            for property in properties {
                for alias in &property.aliases.read {
                    bytes = checked_add(bytes, alias.as_str().len())?;
                }
                if let Some(write) = &property.aliases.write {
                    bytes = checked_add(bytes, write.as_str().len())?;
                }
            }
        },
        Body::Union(union) => {
            for variant in &union.variants {
                if variant.payload.is_none() {
                    bytes = checked_add(bytes, variant.key.as_str().len())?;
                }
            }
            if let SerdeTagging::Adjacent { tag, content } = &union.tagging {
                bytes = checked_add(bytes, tag.len())?;
                bytes = checked_add(bytes, content.len())?;
            }
            if let Some(default_variant) = &union.selector.default_variant {
                bytes = checked_add(bytes, default_variant.as_str().len())?;
            }
            for (alias, target) in &union.selector.aliases {
                bytes = checked_add(bytes, alias.as_str().len())?;
                bytes = checked_add(bytes, target.as_str().len())?;
            }
        },
        Body::Any
        | Body::Null
        | Body::Boolean { .. }
        | Body::Integer(_)
        | Body::Number(_)
        | Body::String { .. }
        | Body::Bytes
        | Body::Array(_)
        | Body::Alias(_) => {},
    }
    Ok(bytes)
}

fn checked_add(left: usize, right: usize) -> Result<usize, AdmissionIssue> {
    left.checked_add(right)
        .ok_or(AdmissionIssue::BudgetOverflow)
}

pub(super) fn check_dangling(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<(), AdmissionFailure> {
    let mut issues = Vec::new();
    let mut truncated = false;
    let mut push_issue = |diagnostic: AdmissionDiagnostic| {
        if issues.len() < MAX_GRAPH_DIAGNOSTICS {
            issues.push(diagnostic);
        } else {
            truncated = true;
        }
    };
    if !lookup.contains_key(&graph.root.0.target) {
        push_issue(AdmissionDiagnostic {
            issue: AdmissionIssue::DanglingReference,
            location: AdmissionLocation::Root,
        });
    }
    for (definition_ordinal, definition) in graph.definitions.iter().enumerate() {
        for edge in definition.edges().map_err(AdmissionFailure::from)? {
            if !lookup.contains_key(&edge.target) {
                push_issue(AdmissionDiagnostic {
                    issue: AdmissionIssue::DanglingReference,
                    location: AdmissionLocation::Use {
                        definition: u32::try_from(definition_ordinal)
                            .map_err(|_| AdmissionIssue::IndexOverflow)?,
                        role: edge.role,
                        ordinal: edge.ordinal,
                    },
                });
            }
        }
    }
    if truncated {
        issues.truncate(MAX_GRAPH_DIAGNOSTICS - 1);
        issues.push(AdmissionDiagnostic::root(AdmissionIssue::DiagnosticsLimit));
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(AdmissionFailure(issues))
    }
}

pub(super) fn check_reachability(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<(), AdmissionIssue> {
    let root = *lookup
        .get(&graph.root.0.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut reached = vec![false; graph.definitions.len()];
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if reached[index.0] {
            continue;
        }
        reached[index.0] = true;
        for edge in graph.definitions[index.0].edges()? {
            pending.push(
                *lookup
                    .get(&edge.target)
                    .ok_or(AdmissionIssue::DanglingReference)?,
            );
        }
    }
    if reached.iter().any(|reached| !reached) {
        return Err(AdmissionIssue::UnreachableDefinition);
    }
    Ok(())
}

pub(super) fn check_productivity(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<(), AdmissionIssue> {
    let mut shapes = vec![StructuralShape::default(); graph.definitions.len()];
    loop {
        let mut changed = false;
        for (index, definition) in graph.definitions.iter().enumerate() {
            let discovered = body_shape(&definition.body, lookup, &shapes)?;
            let combined = shapes[index].union(discovered);
            if combined != shapes[index] {
                shapes[index] = combined;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // This fixed point proves only conservative structural inhabitation.
    // General predicate SAT, intrinsic and value-rule satisfiability, and
    // interactions among independent validation bounds remain runtime
    // concerns. Unknown predicates are treated as both possible; only
    // constants proven through Not/All/Any constrain existence.
    if !occurrence_shape(&graph.root.0, lookup, &shapes)?.is_productive() {
        return Err(AdmissionIssue::NonproductiveDefinition);
    }
    Ok(())
}
