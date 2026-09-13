use std::{cmp::Ordering, sync::Arc};

use super::{
    canonical::{GraphCommitment, encode_graph},
    graph::{
        DefinitionBody, DefinitionDraft, DefinitionKey, EdgeRole, GraphDiagnostic, Presence,
        PropertyEdge, SchemaDraft, TypeRef, VariantEdge,
    },
};
use crate::FieldKey;

pub(super) const MAX_DEFINITIONS: usize = 1_024;
pub(super) const MAX_REFERENCE_EDGES: usize = 4_096;
pub(super) const MAX_IDENTIFIER_UTF8_BYTES: usize = 128 * 1_024;

pub(super) struct AdmissionSeal {
    _private: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct DefinitionIndex(usize);

impl DefinitionIndex {
    const fn new(position: usize) -> Self {
        Self(position)
    }

    const fn position(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedRef {
    target: TypeRef,
    index: DefinitionIndex,
    role: EdgeRole,
}

impl ResolvedRef {
    const fn new(target: TypeRef, index: DefinitionIndex, role: EdgeRole) -> Self {
        Self {
            target,
            index,
            role,
        }
    }

    pub(super) const fn target(&self) -> &TypeRef {
        &self.target
    }

    pub(super) const fn index_position(&self) -> usize {
        self.index.position()
    }

    pub(super) const fn role(&self) -> EdgeRole {
        self.role
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdmittedProperty {
    key: FieldKey,
    presence: Presence,
    target: ResolvedRef,
}

impl AdmittedProperty {
    pub(super) const fn key(&self) -> &FieldKey {
        &self.key
    }

    pub(super) const fn target(&self) -> &ResolvedRef {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdmittedVariant {
    key: FieldKey,
    target: Option<ResolvedRef>,
}

impl AdmittedVariant {
    pub(super) const fn key(&self) -> &FieldKey {
        &self.key
    }

    pub(super) const fn target(&self) -> Option<&ResolvedRef> {
        self.target.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AdmittedBody {
    String,
    Record(Vec<AdmittedProperty>),
    Array {
        item: ResolvedRef,
        min_items: u32,
        max_items: Option<u32>,
        unique: bool,
    },
    Union(Vec<AdmittedVariant>),
    Alias(ResolvedRef),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AdmittedBodyRef<'a> {
    String,
    Record(&'a [AdmittedProperty]),
    Array {
        item: &'a ResolvedRef,
        min_items: u32,
        max_items: Option<u32>,
        unique: bool,
    },
    Union(&'a [AdmittedVariant]),
    Alias(&'a ResolvedRef),
}

impl AdmittedBody {
    fn as_ref(&self) -> AdmittedBodyRef<'_> {
        match self {
            Self::String => AdmittedBodyRef::String,
            Self::Record(properties) => AdmittedBodyRef::Record(properties),
            Self::Array {
                item,
                min_items,
                max_items,
                unique,
            } => AdmittedBodyRef::Array {
                item,
                min_items: *min_items,
                max_items: *max_items,
                unique: *unique,
            },
            Self::Union(variants) => AdmittedBodyRef::Union(variants),
            Self::Alias(target) => AdmittedBodyRef::Alias(target),
        }
    }

    fn for_each_reference(&self, mut visit: impl FnMut(&ResolvedRef)) {
        match self {
            Self::String => {},
            Self::Record(properties) => {
                for property in properties {
                    visit(&property.target);
                }
            },
            Self::Array { item, .. } | Self::Alias(item) => visit(item),
            Self::Union(variants) => {
                for target in variants
                    .iter()
                    .filter_map(|variant| variant.target.as_ref())
                {
                    visit(target);
                }
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdmittedDefinition {
    key: DefinitionKey,
    body: AdmittedBody,
}

impl AdmittedDefinition {
    pub(super) const fn key(&self) -> &DefinitionKey {
        &self.key
    }

    pub(super) fn body(&self) -> AdmittedBodyRef<'_> {
        self.body.as_ref()
    }
}

#[derive(Debug)]
pub(super) struct AdmittedGraph {
    root: ResolvedRef,
    definitions: Vec<AdmittedDefinition>,
    canonical_bytes: Vec<u8>,
    commitment: GraphCommitment,
}

#[derive(Debug, Clone)]
pub(super) struct AdmittedSchema(Arc<AdmittedGraph>);

impl AdmittedSchema {
    pub(super) fn root(&self) -> &ResolvedRef {
        &self.0.root
    }

    pub(super) fn definitions(&self) -> &[AdmittedDefinition] {
        &self.0.definitions
    }

    pub(super) fn definition_names(&self) -> Vec<&str> {
        self.0
            .definitions
            .iter()
            .map(|definition| definition.key.as_str())
            .collect()
    }

    pub(super) fn canonical_bytes(&self) -> &[u8] {
        &self.0.canonical_bytes
    }

    pub(super) fn commitment(&self) -> GraphCommitment {
        self.0.commitment
    }

    pub(super) fn identity(&self) -> &Arc<AdmittedGraph> {
        &self.0
    }
}

#[derive(Debug, Default)]
struct GraphBudget {
    reference_edges: usize,
    identifier_bytes: usize,
}

impl GraphBudget {
    fn add_reference(&mut self, target: &TypeRef) -> Result<(), GraphDiagnostic> {
        self.reference_edges = checked_budget_add(self.reference_edges, 1)?;
        self.add_identifier(target.target_key().as_str())
    }

    fn add_identifier(&mut self, identifier: &str) -> Result<(), GraphDiagnostic> {
        self.identifier_bytes = checked_budget_add(self.identifier_bytes, identifier.len())?;
        Ok(())
    }

    fn finish(self) -> Result<(), GraphDiagnostic> {
        if self.reference_edges > MAX_REFERENCE_EDGES {
            return Err(GraphDiagnostic::root("graph.reference_limit"));
        }
        if self.identifier_bytes > MAX_IDENTIFIER_UTF8_BYTES {
            return Err(GraphDiagnostic::root("graph.identifier_bytes_limit"));
        }
        Ok(())
    }
}

#[tracing::instrument(
    level = "trace",
    name = "schema.property_prototype.graph.admit",
    skip_all,
    err(Debug),
    fields(prototype_version = super::canonical::PROTOTYPE_VERSION)
)]
pub(super) fn admit(mut draft: SchemaDraft) -> Result<AdmittedSchema, GraphDiagnostic> {
    let seal = AdmissionSeal { _private: () };
    if draft.definitions.is_empty() {
        return Err(GraphDiagnostic::root("graph.empty_definitions"));
    }
    if draft.definitions.len() > MAX_DEFINITIONS {
        return Err(GraphDiagnostic::root("graph.definition_limit"));
    }

    draft
        .definitions
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    check_duplicate_definitions(&draft.definitions)?;
    normalize_and_check_local_bodies(&mut draft.definitions)?;
    check_aggregate_budgets(&draft)?;

    let root_index = find_definition(&draft.definitions, draft.root.target_key())?
        .ok_or_else(|| GraphDiagnostic::root("graph.dangling_root"))?;
    let root = ResolvedRef::new(draft.root, root_index, EdgeRole::Root);
    let definitions = resolve_definitions(draft.definitions)?;
    check_reachability(root_index, &definitions)?;
    check_finite_shape_productivity(&definitions)?;
    let (canonical_bytes, commitment) = encode_graph(&seal, &root, &definitions)?;

    Ok(AdmittedSchema(Arc::new(AdmittedGraph {
        root,
        definitions,
        canonical_bytes,
        commitment,
    })))
}

pub(super) fn checked_budget_add(left: usize, right: usize) -> Result<usize, GraphDiagnostic> {
    left.checked_add(right)
        .ok_or_else(|| GraphDiagnostic::root("graph.budget_overflow"))
}

fn edge_ordinal(value: usize) -> Result<u32, GraphDiagnostic> {
    u32::try_from(value).map_err(|_| GraphDiagnostic::root("graph.index_overflow"))
}

fn check_duplicate_definitions(definitions: &[DefinitionDraft]) -> Result<(), GraphDiagnostic> {
    for (position, pair) in definitions.windows(2).enumerate() {
        if pair[0].key == pair[1].key {
            let index = DefinitionIndex::new(position + 1);
            return Err(GraphDiagnostic::definition(
                "graph.duplicate_definition",
                index.position(),
            ));
        }
    }
    Ok(())
}

fn normalize_and_check_local_bodies(
    definitions: &mut [DefinitionDraft],
) -> Result<(), GraphDiagnostic> {
    for (position, definition) in definitions.iter_mut().enumerate() {
        let definition_index = DefinitionIndex::new(position);
        match &mut definition.body {
            DefinitionBody::String | DefinitionBody::Alias(_) => {},
            DefinitionBody::Record(properties) => {
                properties.sort_unstable_by(compare_properties);
                check_duplicate_properties(definition_index, properties)?;
            },
            DefinitionBody::Array {
                min_items,
                max_items,
                ..
            } => {
                if max_items.is_some_and(|maximum| *min_items > maximum) {
                    return Err(GraphDiagnostic::definition(
                        "graph.invalid_array_bounds",
                        definition_index.position(),
                    ));
                }
            },
            DefinitionBody::Union(variants) => {
                if variants.is_empty() {
                    return Err(GraphDiagnostic::definition(
                        "graph.empty_union",
                        definition_index.position(),
                    ));
                }
                variants.sort_unstable_by(compare_variants);
                check_duplicate_variants(definition_index, variants)?;
            },
        }
    }
    Ok(())
}

fn compare_properties(left: &PropertyEdge, right: &PropertyEdge) -> Ordering {
    left.key
        .as_str()
        .cmp(right.key.as_str())
        .then_with(|| presence_rank(left.presence).cmp(&presence_rank(right.presence)))
        .then_with(|| {
            left.target
                .target_key()
                .as_str()
                .cmp(right.target.target_key().as_str())
        })
}

const fn presence_rank(presence: Presence) -> u8 {
    match presence {
        Presence::Required => 0,
        Presence::Optional => 1,
    }
}

fn compare_variants(left: &VariantEdge, right: &VariantEdge) -> Ordering {
    left.key
        .as_str()
        .cmp(right.key.as_str())
        .then_with(|| match (&left.target, &right.target) {
            (Some(left), Some(right)) => {
                left.target_key().as_str().cmp(right.target_key().as_str())
            },
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
}

fn check_duplicate_properties(
    definition: DefinitionIndex,
    properties: &[PropertyEdge],
) -> Result<(), GraphDiagnostic> {
    for (position, pair) in properties.windows(2).enumerate() {
        if pair[0].key == pair[1].key {
            let duplicate = &pair[1];
            return Err(GraphDiagnostic::edge(
                "graph.duplicate_property",
                definition.position(),
                edge_ordinal(position + 1)?,
                property_role(duplicate.presence),
            ));
        }
    }
    Ok(())
}

fn check_duplicate_variants(
    definition: DefinitionIndex,
    variants: &[VariantEdge],
) -> Result<(), GraphDiagnostic> {
    for (position, pair) in variants.windows(2).enumerate() {
        if pair[0].key == pair[1].key {
            let duplicate = &pair[1];
            return Err(GraphDiagnostic::edge(
                "graph.duplicate_variant",
                definition.position(),
                edge_ordinal(position + 1)?,
                variant_role(duplicate),
            ));
        }
    }
    Ok(())
}

fn check_aggregate_budgets(draft: &SchemaDraft) -> Result<(), GraphDiagnostic> {
    let mut budget = GraphBudget::default();
    budget.add_reference(&draft.root)?;
    for definition in &draft.definitions {
        budget.add_identifier(definition.key.as_str())?;
        match &definition.body {
            DefinitionBody::String => {},
            DefinitionBody::Record(properties) => {
                for property in properties {
                    budget.add_identifier(property.key.as_str())?;
                    budget.add_reference(&property.target)?;
                }
            },
            DefinitionBody::Array { item, .. } | DefinitionBody::Alias(item) => {
                budget.add_reference(item)?;
            },
            DefinitionBody::Union(variants) => {
                for variant in variants {
                    budget.add_identifier(variant.key.as_str())?;
                    if let Some(target) = &variant.target {
                        budget.add_reference(target)?;
                    }
                }
            },
        }
    }
    budget.finish()
}

fn find_definition(
    definitions: &[DefinitionDraft],
    key: &DefinitionKey,
) -> Result<Option<DefinitionIndex>, GraphDiagnostic> {
    match definitions.binary_search_by(|definition| definition.key.cmp(key)) {
        Ok(position) => Ok(Some(DefinitionIndex::new(position))),
        Err(_) => Ok(None),
    }
}

fn resolve_definitions(
    definitions: Vec<DefinitionDraft>,
) -> Result<Vec<AdmittedDefinition>, GraphDiagnostic> {
    let keys: Vec<DefinitionKey> = definitions
        .iter()
        .map(|definition| definition.key.clone())
        .collect();
    definitions
        .into_iter()
        .enumerate()
        .map(|(position, definition)| {
            let definition_index = DefinitionIndex::new(position);
            let body = resolve_body(definition.body, definition_index, &keys)?;
            Ok(AdmittedDefinition {
                key: definition.key,
                body,
            })
        })
        .collect()
}

fn resolve_body(
    body: DefinitionBody,
    definition: DefinitionIndex,
    keys: &[DefinitionKey],
) -> Result<AdmittedBody, GraphDiagnostic> {
    match body {
        DefinitionBody::String => Ok(AdmittedBody::String),
        DefinitionBody::Record(properties) => properties
            .into_iter()
            .enumerate()
            .map(|(position, property)| {
                let role = property_role(property.presence);
                let target = resolve_reference(
                    property.target,
                    keys,
                    "graph.dangling_property",
                    definition,
                    edge_ordinal(position)?,
                    role,
                )?;
                Ok(AdmittedProperty {
                    key: property.key,
                    presence: property.presence,
                    target,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(AdmittedBody::Record),
        DefinitionBody::Array {
            item,
            min_items,
            max_items,
            unique,
        } => Ok(AdmittedBody::Array {
            item: resolve_reference(
                item,
                keys,
                "graph.dangling_array_item",
                definition,
                0,
                EdgeRole::ArrayItem,
            )?,
            min_items,
            max_items,
            unique,
        }),
        DefinitionBody::Union(variants) => variants
            .into_iter()
            .enumerate()
            .map(|(position, variant)| {
                let target = variant
                    .target
                    .map(|target| {
                        resolve_reference(
                            target,
                            keys,
                            "graph.dangling_union_payload",
                            definition,
                            edge_ordinal(position)?,
                            EdgeRole::UnionPayload,
                        )
                    })
                    .transpose()?;
                Ok(AdmittedVariant {
                    key: variant.key,
                    target,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(AdmittedBody::Union),
        DefinitionBody::Alias(target) => resolve_reference(
            target,
            keys,
            "graph.dangling_alias",
            definition,
            0,
            EdgeRole::Alias,
        )
        .map(AdmittedBody::Alias),
    }
}

fn resolve_reference(
    target: TypeRef,
    keys: &[DefinitionKey],
    code: &'static str,
    definition: DefinitionIndex,
    ordinal: u32,
    role: EdgeRole,
) -> Result<ResolvedRef, GraphDiagnostic> {
    let index = match keys.binary_search(target.target_key()) {
        Ok(position) => DefinitionIndex::new(position),
        Err(_) => {
            return Err(GraphDiagnostic::edge(
                code,
                definition.position(),
                ordinal,
                role,
            ));
        },
    };
    Ok(ResolvedRef::new(target, index, role))
}

const fn property_role(presence: Presence) -> EdgeRole {
    match presence {
        Presence::Required => EdgeRole::RequiredProperty,
        Presence::Optional => EdgeRole::OptionalProperty,
    }
}

const fn variant_role(variant: &VariantEdge) -> EdgeRole {
    if variant.target.is_some() {
        EdgeRole::UnionPayload
    } else {
        EdgeRole::UnionUnit
    }
}

fn check_reachability(
    root: DefinitionIndex,
    definitions: &[AdmittedDefinition],
) -> Result<(), GraphDiagnostic> {
    let mut reachable = vec![false; definitions.len()];
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        let position = index.position();
        if reachable[position] {
            continue;
        }
        reachable[position] = true;
        // Every index is minted only while resolving this sorted definition table.
        definitions[position]
            .body
            .for_each_reference(|reference| pending.push(reference.index));
    }
    if let Some(position) = reachable.iter().position(|is_reachable| !is_reachable) {
        return Err(GraphDiagnostic::definition(
            "graph.unreachable_definition",
            position,
        ));
    }
    Ok(())
}

// This fixed point proves only that every reachable definition can produce a finite shape.
// It intentionally ignores uniqueness, maximum cardinality, and current or future rule
// satisfiability. Worst case is O(D * (D + E)); D and E are admission-bounded.
fn check_finite_shape_productivity(
    definitions: &[AdmittedDefinition],
) -> Result<(), GraphDiagnostic> {
    let mut productive = vec![false; definitions.len()];
    loop {
        let mut changed = false;
        for (position, definition) in definitions.iter().enumerate() {
            if !productive[position] && body_is_productive(&definition.body, &productive) {
                productive[position] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    if let Some(position) = productive.iter().position(|is_productive| !is_productive) {
        return Err(GraphDiagnostic::definition(
            "graph.nonproductive_recursion",
            position,
        ));
    }
    Ok(())
}

fn body_is_productive(body: &AdmittedBody, productive: &[bool]) -> bool {
    match body {
        AdmittedBody::String => true,
        AdmittedBody::Record(properties) => properties.iter().all(|property| {
            property.presence == Presence::Optional
                || target_is_productive(&property.target, productive)
        }),
        AdmittedBody::Array {
            item, min_items, ..
        } => *min_items == 0 || target_is_productive(item, productive),
        AdmittedBody::Union(variants) => variants.iter().any(|variant| {
            variant
                .target
                .as_ref()
                .is_none_or(|target| target_is_productive(target, productive))
        }),
        AdmittedBody::Alias(target) => target_is_productive(target, productive),
    }
}

fn target_is_productive(target: &ResolvedRef, productive: &[bool]) -> bool {
    // Resolved references can only carry indices minted against this admitted table.
    productive[target.index.position()]
}
