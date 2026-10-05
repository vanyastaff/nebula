//! Bounded coinductive output-to-input compatibility over admitted graphs.
//!
//! Each use-site is checked before memoizing its definition pair. This keeps
//! occurrence constraints distinct even when definitions are shared or recursive.

use std::{cmp::Ordering, collections::BTreeSet, fmt};

use nebula_validator::{Rule, RuleView};

use crate::{Assignability, SchemaIncompat, SerdeTagging, UnknownReason, ValuePath};

use super::{
    MAX_GRAPH_DIAGNOSTICS, MAX_GRAPH_REFERENCES,
    admission::AdmittedSchemaGraph,
    canonical::exact_json_bytes,
    model::{
        AcceptedDomain, AdditionalProperties, Body, EmptyPolicy, NullPolicy, NumericBody,
        PresencePolicy, PropertyUse, UseSiteCore, ValueProtection,
    },
    number::compare_numbers,
};

/// Maximum use pairs inspected by one graph comparison or reference lookup.
pub const MAX_GRAPH_COMPARISON_STEPS: usize = MAX_GRAPH_REFERENCES * 4;

/// Compare an admitted producer output with an admitted consumer input.
///
/// Recursive definitions use a coinductive pair worklist, never recursive Rust
/// calls or legacy-schema lowering. Unproven rules and exhausted budgets return
/// `Unknown`; incompatible structure takes precedence over uncertainty.
/// This relation compares resolved values. Expression authoring permission is
/// enforced by the input preparation contract, not a runtime value domain.
#[must_use]
#[tracing::instrument(level = "trace", name = "schema.graph.assignability", skip_all)]
pub fn explain_graph_assignable(
    producer: &AdmittedSchemaGraph,
    consumer: &AdmittedSchemaGraph,
) -> Assignability {
    compare_uses(
        producer,
        &producer.0.graph.root.0,
        consumer,
        &consumer.0.graph.root.0,
        ConsumerNames::Input,
    )
}

/// Compare a successor output against a previous output's emitted contract.
/// Read aliases cannot make an output revision compatible: only emitted names
/// are compared here, unlike input assignment.
#[must_use]
#[tracing::instrument(level = "trace", name = "schema.graph.successor", skip_all)]
pub fn explain_graph_successor(
    successor: &AdmittedSchemaGraph,
    previous: &AdmittedSchemaGraph,
) -> Assignability {
    compare_uses(
        successor,
        &successor.0.graph.root.0,
        previous,
        &previous.0.graph.root.0,
        ConsumerNames::Output,
    )
}

#[derive(Clone, Copy)]
enum ConsumerNames {
    Input,
    Output,
}

struct Comparison<'a> {
    producer: &'a AdmittedSchemaGraph,
    consumer_names: ConsumerNames,
    pending: Vec<(&'a UseSiteCore, &'a UseSiteCore)>,
    visited: BTreeSet<(usize, usize)>,
    incompatible: Vec<SchemaIncompat>,
    unknown: Vec<UnknownReason>,
}

fn compare_uses<'a>(
    producer: &'a AdmittedSchemaGraph,
    producer_use: &'a UseSiteCore,
    consumer: &'a AdmittedSchemaGraph,
    consumer_use: &'a UseSiteCore,
    consumer_names: ConsumerNames,
) -> Assignability {
    let mut comparison = Comparison {
        producer,
        consumer_names,
        pending: vec![(producer_use, consumer_use)],
        visited: BTreeSet::new(),
        incompatible: Vec::new(),
        unknown: Vec::new(),
    };
    let mut steps = 0;
    while let Some((output, input)) = comparison.pending.pop() {
        steps += 1;
        if steps > MAX_GRAPH_COMPARISON_STEPS {
            comparison.unproven("comparison_budget");
            break;
        }
        let Ok((output_contract, output_index)) = normalize_alias_use(producer, output) else {
            comparison.unproven("alias_intersection");
            continue;
        };
        let Ok((input_contract, input_index)) = normalize_alias_use(consumer, input) else {
            comparison.unproven("alias_intersection");
            continue;
        };
        comparison.check_use(&output_contract, &input_contract);
        if comparison.visited.insert((output_index, input_index)) {
            comparison.check_body(
                &producer.0.graph.definitions[output_index].body,
                &consumer.0.graph.definitions[input_index].body,
                output,
            );
        }
    }
    if !comparison.incompatible.is_empty() {
        Assignability::No(comparison.incompatible)
    } else if !comparison.unknown.is_empty() {
        Assignability::Unknown(comparison.unknown)
    } else {
        Assignability::Yes
    }
}

// An alias is an intersection of occurrence constraints, not a rename or a
// fresh unconstrained use. Normalize that intersection before comparing bodies.
fn normalize_alias_use(
    graph: &AdmittedSchemaGraph,
    root: &UseSiteCore,
) -> Result<(UseSiteCore, usize), ()> {
    let mut effective = root.clone();
    let mut current = root;
    let mut visited = BTreeSet::new();
    loop {
        let index = graph.0.lookup.get(&current.target).ok_or(())?.0;
        if !visited.insert(index) {
            return Err(());
        }
        let body = &graph.0.graph.definitions[index].body;
        let Body::Alias(alias) = body else {
            if matches!(body, Body::Record { properties, .. }
                if properties.iter().any(|property| matches!(property.presence, PresencePolicy::Required)))
                || matches!(body, Body::Array(array) if array.min_items > 0)
            {
                effective.empty_collection = EmptyPolicy::Reject;
            }
            return Ok((effective, index));
        };
        let next = &alias.0;
        effective.null = match (&effective.null, &next.null) {
            (NullPolicy::Reject, _) | (_, NullPolicy::Reject) => NullPolicy::Reject,
            (NullPolicy::Allow, next) => next.clone(),
            (current, _) => current.clone(),
        };
        effective.empty_string = intersect_empty(&effective.empty_string, &next.empty_string);
        effective.empty_collection =
            intersect_empty(&effective.empty_collection, &next.empty_collection);
        match (effective.protection, next.protection) {
            (ValueProtection::Public, next) => effective.protection = next,
            (_, ValueProtection::Public) => {},
            (current, next) if current == next => {},
            _ => return Err(()),
        }
        effective.accepted_domain = match (&effective.accepted_domain, &next.accepted_domain) {
            (AcceptedDomain::Open, next) => next.clone(),
            (current, AcceptedDomain::Open) => current.clone(),
            (AcceptedDomain::Closed(current), AcceptedDomain::Closed(next)) => {
                let accepted = next
                    .iter()
                    .map(exact_json_bytes)
                    .collect::<Result<BTreeSet<_>, _>>()
                    .map_err(|_| ())?;
                let mut retained = Vec::new();
                for value in current {
                    if accepted.contains(&exact_json_bytes(value).map_err(|_| ())?) {
                        retained.push(value.clone());
                    }
                }
                if retained.is_empty() {
                    return Err(());
                }
                AcceptedDomain::Closed(retained)
            },
        };
        effective.rules.extend(next.rules.iter().cloned());
        effective
            .transformers
            .extend(next.transformers.iter().cloned());
        current = next;
    }
}

fn intersect_empty(current: &EmptyPolicy, next: &EmptyPolicy) -> EmptyPolicy {
    match (current, next) {
        (EmptyPolicy::Reject, _) | (_, EmptyPolicy::Reject) => EmptyPolicy::Reject,
        (EmptyPolicy::Allow, next) => next.clone(),
        (current, _) => current.clone(),
    }
}

impl<'a> Comparison<'a> {
    fn accepts_name(&self, property: &PropertyUse, name: &str) -> bool {
        match self.consumer_names {
            ConsumerNames::Input => accepts_name(property, name),
            ConsumerNames::Output => emitted_name(property) == name,
        }
    }
    fn mismatch(&mut self, code: &'static str) {
        if self.incompatible.len() < MAX_GRAPH_DIAGNOSTICS {
            self.incompatible
                .push(SchemaIncompat::GraphConstraintMismatch { code });
        }
    }

    fn unproven(&mut self, code: &'static str) {
        if self.unknown.len() < MAX_GRAPH_DIAGNOSTICS {
            self.unknown
                .push(UnknownReason::UnprovenGraphConstraint { code });
        }
    }

    fn enqueue(&mut self, output: &'a UseSiteCore, input: &'a UseSiteCore) {
        if self.pending.len() >= MAX_GRAPH_COMPARISON_STEPS {
            self.unproven("comparison_budget");
        } else {
            self.pending.push((output, input));
        }
    }

    fn check_use(&mut self, output: &UseSiteCore, input: &UseSiteCore) {
        if output.protection != input.protection {
            self.mismatch("protection");
        }
        let null_excluded = matches!(&output.accepted_domain, AcceptedDomain::Closed(values)
            if values.iter().all(|value| !value.is_null()));
        match (&output.null, &input.null) {
            _ if null_excluded => {},
            (_, NullPolicy::Allow) | (NullPolicy::Reject, _) => {},
            (NullPolicy::Allow, NullPolicy::Reject) => self.mismatch("null"),
            _ => self.unproven("conditional_null"),
        }
        if !matches!(&output.accepted_domain, AcceptedDomain::Closed(values)
            if values.iter().all(|value| value.as_str() != Some("")))
        {
            self.check_empty(&output.empty_string, &input.empty_string, "empty_string");
        }
        if !matches!(&output.accepted_domain, AcceptedDomain::Closed(values)
            if values.iter().all(|value| !value.as_array().is_some_and(Vec::is_empty)
                && !value.as_object().is_some_and(serde_json::Map::is_empty)))
        {
            self.check_empty(
                &output.empty_collection,
                &input.empty_collection,
                "empty_collection",
            );
        }
        match (&output.accepted_domain, &input.accepted_domain) {
            (_, AcceptedDomain::Open) => {},
            (AcceptedDomain::Closed(values), AcceptedDomain::Closed(accepted)) => {
                let accepted = accepted
                    .iter()
                    .map(exact_json_bytes)
                    .collect::<Result<BTreeSet<_>, _>>();
                let Ok(accepted) = accepted else {
                    self.unproven("admitted_domain_invariant");
                    return;
                };
                for value in values {
                    let Ok(encoded) = exact_json_bytes(value) else {
                        self.unproven("admitted_domain_invariant");
                        break;
                    };
                    if !accepted.contains(&encoded) {
                        self.mismatch("accepted_domain");
                        break;
                    }
                }
            },
            (AcceptedDomain::Open, AcceptedDomain::Closed(_)) => {
                self.unproven("accepted_domain");
            },
        }
        self.check_rules(&output.rules, &input.rules);
        // Input transforms can alter the value before its constraints apply;
        // equality alone is not an implication proof of the transformed domain.
        if !input.transformers.is_empty() {
            self.unproven("input_transformers");
        }
    }

    fn check_empty(&mut self, output: &EmptyPolicy, input: &EmptyPolicy, code: &'static str) {
        match (output, input) {
            (_, EmptyPolicy::Allow) | (EmptyPolicy::Reject, _) => {},
            (EmptyPolicy::Allow, EmptyPolicy::Reject) => self.mismatch(code),
            _ => self.unproven("conditional_empty"),
        }
    }

    fn check_rules(&mut self, output: &[Rule], input: &[Rule]) {
        if input
            .iter()
            .any(|rule| !context_free(rule) || !output.contains(rule))
        {
            self.unproven("rules");
        }
    }

    fn check_numeric(&mut self, output: &NumericBody, input: &NumericBody) {
        for (produced, accepted, direction) in [
            (&output.minimum, &input.minimum, Ordering::Less),
            (&output.maximum, &input.maximum, Ordering::Greater),
        ] {
            if let Some(accepted) = accepted {
                match produced
                    .as_ref()
                    .map(|value| compare_numbers(value, accepted))
                {
                    Some(Ok(order)) if order != direction => {},
                    Some(Err(_)) => self.unproven("numeric_comparison"),
                    _ => self.unproven("numeric_bounds"),
                }
            }
        }
        self.check_rules(&output.intrinsic_rules, &input.intrinsic_rules);
    }

    fn check_body(&mut self, output: &'a Body, input: &'a Body, output_use: &'a UseSiteCore) {
        match (output, input) {
            (_, Body::Any) => {
                if protected_subtree(self.producer, output_use) {
                    self.mismatch("protected_subtree_to_any");
                }
            },
            (Body::Any, _) => self.unproven("opaque_producer"),
            (Body::Null, Body::Null) | (Body::Bytes, Body::Bytes) => {},
            (
                Body::Boolean {
                    intrinsic_rules: produced,
                },
                Body::Boolean {
                    intrinsic_rules: accepted,
                },
            )
            | (
                Body::String {
                    intrinsic_rules: produced,
                },
                Body::String {
                    intrinsic_rules: accepted,
                },
            ) => {
                self.check_rules(produced, accepted);
            },
            (Body::Integer(produced), Body::Integer(accepted) | Body::Number(accepted))
            | (Body::Number(produced), Body::Number(accepted)) => {
                self.check_numeric(produced, accepted);
            },
            (Body::Number(produced), Body::Integer(accepted)) => {
                self.unproven("number_to_integer");
                self.check_numeric(produced, accepted);
            },
            (
                Body::Record {
                    properties: produced,
                    additional_properties: extra_output,
                    intrinsic_rules: output_rules,
                },
                Body::Record {
                    properties: accepted,
                    additional_properties: extra_input,
                    intrinsic_rules: input_rules,
                },
            ) => {
                self.check_rules(output_rules, input_rules);
                self.check_record(produced, extra_output, accepted, extra_input);
            },
            (Body::Array(produced), Body::Array(accepted)) => {
                if produced.min_items < accepted.min_items
                    || accepted
                        .max_items
                        .is_some_and(|limit| produced.max_items.is_none_or(|max| max > limit))
                    || (accepted.unique && !produced.unique)
                {
                    self.unproven("array_constraints");
                }
                self.check_rules(&produced.intrinsic_rules, &accepted.intrinsic_rules);
                self.enqueue(&produced.element.0, &accepted.element.0);
            },
            (Body::Union(produced), Body::Union(accepted)) => {
                if produced.tagging != accepted.tagging {
                    self.mismatch("union_tagging");
                }
                for variant in &produced.variants {
                    let matching = accepted.variants.iter().find(|candidate| {
                        candidate.key == variant.key
                            || (matches!(self.consumer_names, ConsumerNames::Input)
                                && accepted.selector.aliases.iter().any(|(alias, key)| {
                                    alias == &variant.key && key == &candidate.key
                                }))
                    });
                    match matching.map(|candidate| (&variant.payload, &candidate.payload)) {
                        Some((Some(output), Some(input))) => self.enqueue(&output.0, &input.0),
                        Some((None, None)) => {},
                        Some(_) => self.mismatch("union_payload"),
                        None => self.mismatch("union_variant"),
                    }
                }
            },
            _ => self.mismatch("body_kind"),
        }
    }

    fn check_record(
        &mut self,
        produced: &'a [PropertyUse],
        extra_output: &'a AdditionalProperties,
        accepted: &'a [PropertyUse],
        extra_input: &'a AdditionalProperties,
    ) {
        for input in accepted {
            let accepts_default = matches!(self.consumer_names, ConsumerNames::Input)
                && input.input_default.is_some();
            let matching = produced
                .iter()
                .filter(|output| self.accepts_name(input, emitted_name(output)))
                .collect::<Vec<_>>();
            if matching.is_empty() {
                if !matches!(input.presence, PresencePolicy::Optional) && !accepts_default {
                    match (&input.presence, extra_output) {
                        (PresencePolicy::Required, AdditionalProperties::Closed) => {
                            self.mismatch("missing_required_property");
                        },
                        _ => self.unproven("missing_required_property"),
                    }
                }
                // Extra producer keys can populate optional named consumer
                // properties too; their value domains must still fit.
                match extra_output {
                    AdditionalProperties::Typed(output) => self.enqueue(output, &input.core),
                    AdditionalProperties::Open => self.unproven("named_property_from_open_record"),
                    AdditionalProperties::Closed => {},
                }
            } else {
                for output in matching {
                    match (&output.presence, &input.presence) {
                        (PresencePolicy::Required, _) | (_, PresencePolicy::Optional) => {},
                        _ if accepts_default => {},
                        (PresencePolicy::Optional, PresencePolicy::Required) => {
                            self.mismatch("required_presence");
                        },
                        _ => self.unproven("conditional_presence"),
                    }
                    self.enqueue(&output.core, &input.core);
                }
            }
            // A declared optional spelling does not constrain additional keys
            // that can feed this same consumer field through a read alias.
            // Only a required canonical spelling proves those aliases lose.
            let canonical_guaranteed = produced.iter().any(|output| {
                emitted_name(output) == input.key.as_str()
                    && matches!(output.presence, PresencePolicy::Required)
            });
            let extra_spelling = std::iter::once(input.key.as_str())
                .chain(input.aliases.read.iter().map(crate::FieldKey::as_str))
                .filter(|_| matches!(self.consumer_names, ConsumerNames::Input))
                .any(|name| !produced.iter().any(|output| emitted_name(output) == name));
            if !canonical_guaranteed && extra_spelling {
                match extra_output {
                    AdditionalProperties::Typed(output) => self.enqueue(output, &input.core),
                    AdditionalProperties::Open => self.unproven("alias_from_open_record"),
                    AdditionalProperties::Closed => {},
                }
            }
        }
        for output in produced {
            if !accepted
                .iter()
                .any(|input| self.accepts_name(input, emitted_name(output)))
            {
                match extra_input {
                    AdditionalProperties::Closed => self.mismatch("additional_property"),
                    AdditionalProperties::Typed(input) => self.enqueue(&output.core, input),
                    AdditionalProperties::Open => {
                        if protected_subtree(self.producer, &output.core) {
                            self.mismatch("protected_additional_property");
                        }
                    },
                }
            }
        }
        match (extra_output, extra_input) {
            (AdditionalProperties::Typed(output), AdditionalProperties::Open) => {
                if protected_subtree(self.producer, output) {
                    self.mismatch("protected_additional_properties");
                }
            },
            (AdditionalProperties::Closed, _)
            | (AdditionalProperties::Open, AdditionalProperties::Open) => {},
            (AdditionalProperties::Typed(output), AdditionalProperties::Typed(input)) => {
                self.enqueue(output, input);
            },
            (_, AdditionalProperties::Closed) => self.mismatch("additional_properties"),
            (AdditionalProperties::Open, AdditionalProperties::Typed(_)) => {
                self.unproven("additional_properties");
            },
        }
    }
}

fn emitted_name(property: &PropertyUse) -> &str {
    property
        .aliases
        .write
        .as_ref()
        .unwrap_or(&property.key)
        .as_str()
}

fn accepts_name(property: &PropertyUse, name: &str) -> bool {
    property.key.as_str() == name
        || property
            .aliases
            .read
            .iter()
            .any(|alias| alias.as_str() == name)
}

fn context_free(rule: &Rule) -> bool {
    let mut pending = vec![rule.root()];
    while let Some(rule) = pending.pop() {
        match rule.view() {
            RuleView::Value(_) => {},
            RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
            RuleView::Not(child) | RuleView::Described { inner: child, .. } => pending.push(child),
            _ => return false,
        }
    }
    true
}

fn protected_subtree(graph: &AdmittedSchemaGraph, root: &UseSiteCore) -> bool {
    let mut pending = vec![root];
    let mut visited = BTreeSet::new();
    while let Some(core) = pending.pop() {
        if core.protection != ValueProtection::Public {
            return true;
        }
        let Some(index) = graph.0.lookup.get(&core.target) else {
            return true;
        };
        if !visited.insert(index.0) {
            continue;
        }
        match &graph.0.graph.definitions[index.0].body {
            Body::Record {
                properties,
                additional_properties,
                ..
            } => {
                pending.extend(properties.iter().map(|property| &property.core));
                if let AdditionalProperties::Typed(core) = additional_properties {
                    pending.push(core);
                }
            },
            Body::Array(array) => pending.push(&array.element.0),
            Body::Union(union) => pending.extend(
                union
                    .variants
                    .iter()
                    .filter_map(|variant| variant.payload.as_ref().map(|payload| &payload.0)),
            ),
            Body::Alias(alias) => pending.push(&alias.0),
            _ => {},
        }
    }
    false
}

/// A borrowed exact occurrence within one admitted graph, without exposing
/// mutable definitions, graph indices, or protected payloads.
pub struct GraphReference<'a> {
    pub(super) graph: &'a AdmittedSchemaGraph,
    pub(super) core: &'a UseSiteCore,
    optional: bool,
    ancestor_obligations: bool,
    canonical_path: ValuePath,
}

impl fmt::Debug for GraphReference<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphReference")
            .field("optional", &self.optional)
            .finish_non_exhaustive()
    }
}

impl GraphReference<'_> {
    /// Selected path with input read aliases resolved to canonical field names.
    /// Output selection retains the emitted field names.
    #[must_use]
    pub const fn canonical_path(&self) -> &ValuePath {
        &self.canonical_path
    }

    /// Whether supplied literal data populates a protected occurrence.
    #[must_use]
    pub fn data_populates_protected(&self, value: &serde_json::Value) -> bool {
        super::sensitive::data_populates_protected(self.graph, self.core, value)
    }
    /// Admitted graph retaining this occurrence's custody.
    #[must_use]
    pub const fn graph(&self) -> &AdmittedSchemaGraph {
        self.graph
    }

    /// Whether this occurrence or any reachable descendant carries protection.
    /// Hosts can reject plaintext literal capture without exposing declarations.
    #[must_use]
    pub fn is_protected_or_contains_protected(&self) -> bool {
        protected_subtree(self.graph, self.core)
    }

    /// Compare this producer occurrence against a consumer occurrence.
    #[must_use]
    #[tracing::instrument(
        level = "trace",
        name = "schema.graph.reference_assignability",
        skip_all
    )]
    pub fn explain_assignable_to(&self, consumer: &Self) -> Assignability {
        let verdict = compare_uses(
            self.graph,
            self.core,
            consumer.graph,
            consumer.core,
            ConsumerNames::Input,
        );
        if (self.optional || consumer.ancestor_obligations) && matches!(verdict, Assignability::Yes)
        {
            Assignability::Unknown(vec![UnknownReason::UnprovenGraphConstraint {
                code: if self.optional {
                    "reference_presence"
                } else {
                    "reference_ancestor_constraints"
                },
            }])
        } else {
            verdict
        }
    }
}

/// A bounded, payload-free reference path failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("schema graph reference cannot be resolved: {code}")]
pub struct GraphReferenceError {
    /// Stable diagnostic category; never the authored path or a protected value.
    pub code: &'static str,
}

impl AdmittedSchemaGraph {
    /// Classifies the concrete root body independently of its nullable policy.
    /// Alias-only cycles retain an explicit unresolved classification.
    #[must_use]
    pub fn root_kind(&self) -> GraphRootKind {
        let mut core = &self.0.graph.root.0;
        let mut visited = BTreeSet::new();
        loop {
            let Some(index) = self.0.lookup.get(&core.target) else {
                return GraphRootKind::UnresolvedAlias;
            };
            if !visited.insert(index.0) {
                return GraphRootKind::UnresolvedAlias;
            }
            return match &self.0.graph.definitions[index.0].body {
                Body::Alias(alias) => {
                    core = &alias.0;
                    continue;
                },
                Body::Any => GraphRootKind::Any,
                Body::Null => GraphRootKind::Null,
                Body::Boolean { .. } => GraphRootKind::Boolean,
                Body::Integer(_) => GraphRootKind::Integer,
                Body::Number(_) => GraphRootKind::Number,
                Body::String { .. } => GraphRootKind::String,
                Body::Bytes => GraphRootKind::Bytes,
                Body::Record { .. } => GraphRootKind::Record,
                Body::Array(_) => GraphRootKind::Array,
                Body::Union(_) => GraphRootKind::Union,
            };
        }
    }
    /// Select an output occurrence using an RFC6901 data path.
    ///
    /// # Errors
    /// Rejects missing, ambiguous, protected-descendant, nullable-descendant,
    /// and budget-exhausted paths rather than inventing an opaque schema.
    #[tracing::instrument(level = "trace", name = "schema.graph.output_reference", skip_all, err)]
    pub fn reference_at(
        &self,
        path: &ValuePath,
    ) -> Result<GraphReference<'_>, GraphReferenceError> {
        walk_reference(self, path, false)
    }

    /// Select a consumer occurrence using its input property names/read aliases.
    ///
    /// # Errors
    /// Returns a payload-free error for a path with no exact admitted occurrence.
    #[tracing::instrument(level = "trace", name = "schema.graph.input_reference", skip_all, err)]
    pub fn input_reference_at(
        &self,
        path: &ValuePath,
    ) -> Result<GraphReference<'_>, GraphReferenceError> {
        walk_reference(self, path, true)
    }
}

/// Concrete admitted root-body classification, separate from occurrence policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GraphRootKind {
    /// An explicitly authored opaque JSON body.
    Any,
    /// Null body.
    Null,
    /// Boolean body.
    Boolean,
    /// Integer body.
    Integer,
    /// Number body.
    Number,
    /// String body.
    String,
    /// Bytes body.
    Bytes,
    /// Named object body.
    Record,
    /// Array body.
    Array,
    /// Tagged union body.
    Union,
    /// No concrete body is reachable through a bounded alias chain.
    UnresolvedAlias,
}

fn walk_reference<'a>(
    graph: &'a AdmittedSchemaGraph,
    path: &ValuePath,
    input: bool,
) -> Result<GraphReference<'a>, GraphReferenceError> {
    walk_reference_from(graph, &graph.0.graph.root.0, path, input)
}

pub(super) fn walk_reference_from<'a>(
    graph: &'a AdmittedSchemaGraph,
    root: &'a UseSiteCore,
    path: &ValuePath,
    input: bool,
) -> Result<GraphReference<'a>, GraphReferenceError> {
    let error = |code| GraphReferenceError { code };
    if path.as_str().len() > super::MAX_GRAPH_IDENTIFIER_BYTES {
        return Err(error("path_budget"));
    }
    let mut core = root;
    let mut optional = false;
    let mut ancestor_obligations = false;
    let mut canonical_segments = Vec::new();
    let mut steps = 0;
    for encoded in path.as_str().split('/').skip(1) {
        let segment = encoded.replace("~1", "/").replace("~0", "~");
        let mut canonical_segment = segment.clone();
        steps += 1;
        if steps > MAX_GRAPH_COMPARISON_STEPS {
            return Err(error("path_budget"));
        }
        let (effective, index) =
            normalize_alias_use(graph, core).map_err(|()| error("alias_intersection"))?;
        if input {
            ancestor_obligations |= !matches!(effective.accepted_domain, AcceptedDomain::Open)
                || !effective.rules.is_empty()
                || !effective.transformers.is_empty();
            ancestor_obligations |= match &graph.0.graph.definitions[index].body {
                Body::Record {
                    intrinsic_rules, ..
                } => !intrinsic_rules.is_empty(),
                Body::Array(array) => array.unique || !array.intrinsic_rules.is_empty(),
                _ => false,
            };
        }
        if effective.protection != ValueProtection::Public {
            return Err(error("protected_descendant"));
        }
        if !input
            && !matches!(effective.null, NullPolicy::Reject)
            && !matches!(&effective.accepted_domain, AcceptedDomain::Closed(values)
                    if values.iter().all(|value| !value.is_null()))
        {
            return Err(error("nullable_descendant"));
        }
        match &graph.0.graph.definitions[index].body {
            Body::Record {
                properties,
                additional_properties,
                ..
            } => {
                if let Some(property) = properties.iter().find(|property| {
                    if input {
                        accepts_name(property, &segment)
                    } else {
                        emitted_name(property) == segment
                    }
                }) {
                    canonical_segment = if input {
                        property.key.as_str().to_owned()
                    } else {
                        emitted_name(property).to_owned()
                    };
                    optional |= !matches!(property.presence, PresencePolicy::Required)
                        && (!input || property.input_default.is_none());
                    core = &property.core;
                } else if let AdditionalProperties::Typed(additional) = additional_properties {
                    core = additional;
                    optional = true;
                } else {
                    return Err(error("missing_property"));
                }
            },
            Body::Array(array) => {
                if segment.is_empty()
                    || (segment.len() > 1 && segment.starts_with('0'))
                    || !segment.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(error("array_index"));
                }
                let index = segment.parse::<u32>().map_err(|_| error("array_index"))?;
                if array.max_items.is_some_and(|max| index >= max) {
                    return Err(error("array_index"));
                }
                optional |= index >= array.min_items;
                core = &array.element.0;
            },
            Body::Union(union) => {
                let payload = match &union.tagging {
                    SerdeTagging::External => union
                        .variants
                        .iter()
                        .find(|variant| {
                            variant.key.as_str() == segment
                                || (input
                                    && union.selector.aliases.iter().any(|(alias, key)| {
                                        alias.as_str() == segment && key == &variant.key
                                    }))
                        })
                        .inspect(|variant| {
                            variant.key.as_str().clone_into(&mut canonical_segment);
                        })
                        .and_then(|variant| variant.payload.as_ref()),
                    SerdeTagging::Adjacent { content, .. } if content == &segment => {
                        let mut payloads = union
                            .variants
                            .iter()
                            .filter_map(|variant| variant.payload.as_ref());
                        let payload = payloads.next();
                        if payloads.next().is_some() {
                            return Err(error("ambiguous_union"));
                        }
                        payload
                    },
                    _ => None,
                }
                .ok_or_else(|| error("missing_union_payload"))?;
                optional |= union.variants.len() != 1;
                core = &payload.0;
            },
            _ => return Err(error("non_container")),
        }
        canonical_segments.push(canonical_segment);
    }
    Ok(GraphReference {
        graph,
        core,
        optional,
        ancestor_obligations,
        canonical_path: ValuePath::from_segments(canonical_segments),
    })
}
