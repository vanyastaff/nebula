use std::{fmt, sync::Arc};

use serde_json::Value;

use crate::{ValidationError, ValidationReport};

use super::{
    SCHEMA_GRAPH_WIRE_VERSION,
    canonical::{AddressSpaceCommitment, SemanticCommitment, commitments},
    document::SchemaGraphDocument,
    error::SchemaAdmissionError,
    model::{
        AdditionalProperties, AdmissionDiagnostic, AdmissionIssue, AdmissionLocation, Body,
        Definition, DefinitionIndex, DefinitionLookup, DraftGraph, Edge, EdgeRole, UseSiteCore,
    },
};

mod check;
mod facets;
mod keys;
mod parse;
mod rules;
mod shape;

use check::{
    check_budgets_and_build_lookup, check_dangling, check_duplicate_definitions,
    check_productivity, check_reachability, normalize_and_check_local,
};
use facets::check_facet_applicability;
pub use keys::{DeclarationAddress, DeclarationUse, DefinitionMemberKey};
use parse::parse_document;
use rules::{address_exists, canonical_numbering};

/// An admitted declaration address bound to one graph admission.
#[derive(Clone)]
pub struct AdmittedDeclarationAddress {
    graph: Arc<AdmittedGraph>,
    definition: DefinitionIndex,
    use_site: DeclarationUse,
}

impl fmt::Debug for AdmittedDeclarationAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedDeclarationAddress")
            .field("use_site", &self.use_site)
            .finish_non_exhaustive()
    }
}

impl AdmittedDeclarationAddress {
    /// Borrows the admitted role without exposing graph indices.
    #[must_use]
    pub const fn use_site(&self) -> &DeclarationUse {
        &self.use_site
    }

    /// Returns whether this address came from the same admission instance.
    #[must_use]
    pub fn belongs_to(&self, graph: &AdmittedSchemaGraph) -> bool {
        Arc::ptr_eq(&self.graph, &graph.0)
    }

    /// Returns whether both handles name the same declaration in one admission.
    #[must_use]
    pub fn same_declaration(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.graph, &other.graph)
            && self.definition == other.definition
            && self.use_site == other.use_site
    }
}

pub(super) struct AdmittedGraph {
    pub(super) graph: DraftGraph,
    pub(super) lookup: DefinitionLookup,
    pub(super) canonical_numbers: Vec<u32>,
    semantic_commitment: SemanticCommitment,
    address_space_commitment: AddressSpaceCommitment,
    reference_count: usize,
}

/// An opaque graph admitted for future `ValidSchema` custody.
#[derive(Clone)]
pub struct AdmittedSchemaGraph(pub(super) Arc<AdmittedGraph>);

impl fmt::Debug for AdmittedSchemaGraph {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedSchemaGraph")
            .field("definition_count", &self.0.graph.definitions.len())
            .field("reference_count", &self.0.reference_count)
            .finish_non_exhaustive()
    }
}

impl AdmittedSchemaGraph {
    /// Commitment to executable semantics, invariant under definition alpha-renaming.
    #[must_use]
    pub fn semantic_commitment(&self) -> &SemanticCommitment {
        &self.0.semantic_commitment
    }

    /// Commitment to authored definition and declaration addresses.
    #[must_use]
    pub fn address_space_commitment(&self) -> &AddressSpaceCommitment {
        &self.0.address_space_commitment
    }

    /// Number of root-reachable definitions in this admitted graph.
    #[must_use]
    pub fn definition_count(&self) -> usize {
        self.0.graph.definitions.len()
    }

    /// Number of admitted reference edges, including the root use.
    #[must_use]
    pub fn reference_count(&self) -> usize {
        self.0.reference_count
    }

    /// Resolves a key-based companion-document address under this graph's address space.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_schema::{DeclarationAddress, DeclarationUse, DefinitionKey, SchemaGraphDocument};
    /// use serde_json::json;
    ///
    /// let document: SchemaGraphDocument = serde_json::from_value(json!({
    ///     "version": 3,
    ///     "root": { "target": "root" },
    ///     "definitions": [{ "key": "root", "body": { "kind": "string" } }]
    /// }))?;
    /// let graph = document.admit()?;
    /// let address = DeclarationAddress::new(DefinitionKey::new("root")?, DeclarationUse::Root);
    /// assert!(graph.resolve_address(&address).is_some());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn resolve_address(
        &self,
        address: &DeclarationAddress,
    ) -> Option<AdmittedDeclarationAddress> {
        let definition = *self.0.lookup.get(address.definition())?;
        if !address_exists(&self.0.graph, definition, address.use_site()) {
            return None;
        }
        Some(AdmittedDeclarationAddress {
            graph: Arc::clone(&self.0),
            definition,
            use_site: address.use_site().clone(),
        })
    }
}

#[tracing::instrument(
    level = "trace",
    name = "schema.definition.admit",
    skip_all,
    err,
    fields(wire_version = SCHEMA_GRAPH_WIRE_VERSION)
)]
pub(super) fn admit(
    document: SchemaGraphDocument,
) -> Result<AdmittedSchemaGraph, SchemaAdmissionError> {
    match admit_inner(document.raw()) {
        Ok(graph) => {
            tracing::trace!(
                definition_count = graph.graph.definitions.len(),
                reference_count = graph.reference_count,
                "schema graph admitted"
            );
            Ok(AdmittedSchemaGraph(Arc::new(graph)))
        },
        Err(failure) => {
            let mut report = ValidationReport::new();
            report.extend(
                failure
                    .into_diagnostics()
                    .into_iter()
                    .map(AdmissionDiagnostic::into_validation_error),
            );
            Err(SchemaAdmissionError::new(document, report))
        },
    }
}

struct AdmissionFailure(Vec<AdmissionDiagnostic>);

impl AdmissionFailure {
    fn into_diagnostics(self) -> Vec<AdmissionDiagnostic> {
        self.0
    }
}

impl From<AdmissionIssue> for AdmissionFailure {
    fn from(issue: AdmissionIssue) -> Self {
        Self(vec![AdmissionDiagnostic::root(issue)])
    }
}

fn admit_inner(raw: &Value) -> Result<AdmittedGraph, AdmissionFailure> {
    let mut graph = parse_document(raw)?;
    graph
        .definitions
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    check_duplicate_definitions(&graph.definitions)?;
    normalize_and_check_local(&mut graph.definitions)?;
    let (lookup, reference_count) = check_budgets_and_build_lookup(&graph)?;
    check_dangling(&graph, &lookup)?;
    check_reachability(&graph, &lookup)?;
    check_productivity(&graph, &lookup)?;
    check_facet_applicability(&graph, &lookup)?;
    let canonical_numbers = canonical_numbering(&graph, &lookup)?;
    let (semantic_commitment, address_space_commitment) =
        commitments(&graph, &lookup, &canonical_numbers)?;
    Ok(AdmittedGraph {
        graph,
        lookup,
        canonical_numbers,
        semantic_commitment,
        address_space_commitment,
        reference_count,
    })
}

impl Definition {
    pub(super) fn edges(&self) -> Result<Vec<Edge>, AdmissionIssue> {
        let mut edges = Vec::new();
        match &self.body {
            Body::Record {
                properties,
                additional_properties,
                ..
            } => {
                for (ordinal, property) in properties.iter().enumerate() {
                    edges.push(Edge {
                        role: EdgeRole::Property,
                        local_key: Some(property.key.clone()),
                        ordinal: u32::try_from(ordinal)
                            .map_err(|_| AdmissionIssue::IndexOverflow)?,
                        target: property.core.target.clone(),
                    });
                }
                if let AdditionalProperties::Typed(core) = additional_properties {
                    edges.push(Edge {
                        role: EdgeRole::AdditionalProperty,
                        local_key: None,
                        ordinal: 0,
                        target: core.target.clone(),
                    });
                }
            },
            Body::Array(array) => edges.push(Edge {
                role: EdgeRole::Element,
                local_key: None,
                ordinal: 0,
                target: array.element.0.target.clone(),
            }),
            Body::Union(union) => {
                for (ordinal, variant) in union.variants.iter().enumerate() {
                    if let Some(payload) = &variant.payload {
                        edges.push(Edge {
                            role: EdgeRole::VariantPayload,
                            local_key: Some(variant.key.clone()),
                            ordinal: u32::try_from(ordinal)
                                .map_err(|_| AdmissionIssue::IndexOverflow)?,
                            target: payload.0.target.clone(),
                        });
                    }
                }
            },
            Body::Alias(alias) => edges.push(Edge {
                role: EdgeRole::Alias,
                local_key: None,
                ordinal: 0,
                target: alias.0.target.clone(),
            }),
            _ => {},
        }
        Ok(edges)
    }

    pub(super) fn use_for_edge(&self, edge: &Edge) -> Option<&UseSiteCore> {
        match (&self.body, edge.role) {
            (Body::Record { properties, .. }, EdgeRole::Property) => properties
                .iter()
                .find(|property| {
                    edge.local_key
                        .as_ref()
                        .is_some_and(|key| key == &property.key)
                })
                .map(|property| &property.core),
            (
                Body::Record {
                    additional_properties: AdditionalProperties::Typed(core),
                    ..
                },
                EdgeRole::AdditionalProperty,
            ) => Some(core),
            (Body::Array(array), EdgeRole::Element) => Some(&array.element.0),
            (Body::Union(union), EdgeRole::VariantPayload) => union
                .variants
                .iter()
                .find(|variant| {
                    edge.local_key
                        .as_ref()
                        .is_some_and(|key| key == &variant.key)
                })
                .and_then(|variant| variant.payload.as_ref())
                .map(|payload| &payload.0),
            (Body::Alias(alias), EdgeRole::Alias) => Some(&alias.0),
            _ => None,
        }
    }
}

impl AdmissionDiagnostic {
    fn into_validation_error(self) -> ValidationError {
        let (code, message) = match self.issue {
            AdmissionIssue::InvalidDocument => (
                "schema.graph.invalid_document",
                "schema graph document is malformed",
            ),
            AdmissionIssue::UnsupportedVersion => (
                "schema.graph.unsupported_version",
                "schema graph wire version is unsupported",
            ),
            AdmissionIssue::UnknownMember => (
                "schema.graph.unknown_facet",
                "schema graph contains an unknown mandatory member",
            ),
            AdmissionIssue::UnknownBody => (
                "schema.graph.unknown_body",
                "schema graph contains an unknown semantic body",
            ),
            AdmissionIssue::UnknownRequiredExtension => (
                "schema.graph.unknown_required_extension",
                "schema graph requires an unsupported extension",
            ),
            AdmissionIssue::InvalidIdentifier => (
                "schema.graph.invalid_identifier",
                "schema graph contains an invalid identifier",
            ),
            AdmissionIssue::DuplicateDefinition => (
                "schema.graph.duplicate_definition",
                "schema graph contains a duplicate definition",
            ),
            AdmissionIssue::DuplicateLocalKey => (
                "schema.graph.duplicate_local_key",
                "schema graph contains a duplicate local declaration",
            ),
            AdmissionIssue::DefinitionLimit => (
                "schema.graph.definition_limit",
                "schema graph exceeds the definition limit",
            ),
            AdmissionIssue::ReferenceLimit => (
                "schema.graph.reference_limit",
                "schema graph exceeds the reference limit",
            ),
            AdmissionIssue::IdentifierBytesLimit => (
                "schema.graph.identifier_bytes_limit",
                "schema graph exceeds the identifier byte limit",
            ),
            AdmissionIssue::BudgetOverflow => (
                "schema.graph.budget_overflow",
                "schema graph budget arithmetic overflowed",
            ),
            AdmissionIssue::DanglingReference => (
                "schema.graph.dangling_reference",
                "schema graph contains a dangling reference",
            ),
            AdmissionIssue::UnreachableDefinition => (
                "schema.graph.unreachable_definition",
                "schema graph contains an unreachable definition",
            ),
            AdmissionIssue::NonproductiveDefinition => (
                "schema.graph.nonproductive_definition",
                "schema graph has no finite productive shape",
            ),
            AdmissionIssue::InvalidBounds => (
                "schema.graph.invalid_bounds",
                "schema graph contains invalid bounds or selector normalization",
            ),
            AdmissionIssue::InvalidRule => (
                "schema.graph.invalid_rule",
                "schema graph contains a rule rejected by validator admission",
            ),
            AdmissionIssue::InvalidTransformer => (
                "schema.graph.invalid_transformer",
                "schema graph contains an invalid transformer",
            ),
            AdmissionIssue::InapplicableFacet => (
                "schema.graph.inapplicable_facet",
                "schema graph applies a facet to an incompatible body",
            ),
            AdmissionIssue::InvalidDefault => (
                "schema.graph.invalid_default",
                "schema graph contains an invalid input default",
            ),
            AdmissionIssue::CanonicalBytesLimit => (
                "schema.graph.canonical_bytes_limit",
                "schema graph exceeds the canonical byte limit",
            ),
            AdmissionIssue::DiagnosticsLimit => (
                "schema.graph.diagnostic_limit",
                "schema graph diagnostics were truncated at the fixed limit",
            ),
            AdmissionIssue::IndexOverflow => (
                "schema.graph.index_overflow",
                "schema graph index cannot be represented",
            ),
        };
        let builder = ValidationError::builder(code).message(message);
        match self.location {
            AdmissionLocation::Root => builder.param("location", "root"),
            AdmissionLocation::Definition { ordinal } => builder
                .param("location", "definition")
                .param("definition_ordinal", ordinal),
            AdmissionLocation::Use {
                definition,
                role,
                ordinal,
            } => builder
                .param("location", "use")
                .param("definition_ordinal", definition)
                .param("role", role.as_str())
                .param("use_ordinal", ordinal),
        }
        .build()
    }
}

impl EdgeRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Alias => "alias",
            Self::Property => "property",
            Self::AdditionalProperty => "additional_property",
            Self::Element => "element",
            Self::VariantPayload => "variant_payload",
        }
    }
}
