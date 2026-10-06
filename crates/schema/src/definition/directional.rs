//! Directional executable contracts and evidence-only durable snapshots.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AdmittedSchemaGraph, SchemaGraphDocument};
use crate::{SchemaDirection, ValidationError, ValidationReport};

/// Version of schema-owned directional codec semantics.
pub const GRAPH_CODEC_VERSION: u16 = 1;
/// Version of recorded directional contract evidence.
pub const SCHEMA_CONTRACT_EVIDENCE_VERSION: u16 = 1;

#[derive(Debug, thiserror::Error)]
enum UnsupportedDirectionalFacet {
    #[error("named conditions require the inbound codec direction")]
    NamedConditions,
}

fn contract_error() -> ValidationReport {
    ValidationError::builder("schema.condition.invalid_table")
        .message("directional schema condition table admission failed")
        .build()
        .into()
}

/// Commitment binding graph semantics, codec direction and codec epoch.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DirectionalCommitment([u8; 32]);

impl DirectionalCommitment {
    /// Opaque commitment bytes for exact retained-contract comparisons.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for DirectionalCommitment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectionalCommitment(<opaque>)")
    }
}

/// Retained canonical inbound contract. It is created only by graph admission.
#[derive(Debug, Clone)]
pub struct InputContract {
    pub(super) graph: AdmittedSchemaGraph,
    commitment: DirectionalCommitment,
    pub(super) codec_type: Option<std::any::TypeId>,
}

/// Retained public literal outbound contract.
#[derive(Debug, Clone)]
pub struct OutputContract {
    pub(super) graph: AdmittedSchemaGraph,
    commitment: DirectionalCommitment,
}

impl InputContract {
    /// Compile this exact graph for the schema-owned input codec epoch.
    ///
    /// # Errors
    /// Reserved for checked input contract compilation diagnostics.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn from_graph(graph: &AdmittedSchemaGraph) -> Result<Self, ValidationReport> {
        let mut contract = Self {
            graph: graph.clone(),
            commitment: directional_commitment(graph, SchemaDirection::Input),
            codec_type: None,
        };
        let inventory = super::conditions::checked_condition_inventory(&contract)?;
        if !inventory.is_empty() {
            let mut hash = blake3::Hasher::new();
            hash.update(b"nebula-directional-named-conditions");
            hash.update(contract.commitment.as_bytes());
            hash.update(&1_u16.to_le_bytes());
            let mut entries = inventory
                .into_iter()
                .map(|(scope, name, condition)| {
                    let scope = if scope.is_empty() {
                        None
                    } else {
                        let anchor =
                            super::DefinitionKey::new(&scope).map_err(|_| contract_error())?;
                        let index = graph.0.lookup.get(&anchor).ok_or_else(contract_error)?;
                        Some(
                            *graph
                                .0
                                .canonical_numbers
                                .get(index.0)
                                .ok_or_else(contract_error)?,
                        )
                    };
                    Ok((scope, name, condition))
                })
                .collect::<Result<Vec<_>, ValidationReport>>()?;
            entries.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
            for (scope, name, condition) in entries {
                let value = serde_json::to_value(condition).map_err(|_| contract_error())?;
                let bytes =
                    super::canonical::exact_json_bytes(&value).map_err(|_| contract_error())?;
                hash.update(&[u8::from(scope.is_some())]);
                if let Some(scope) = scope {
                    hash.update(&scope.to_le_bytes());
                }
                hash.update(&(name.len() as u64).to_le_bytes());
                hash.update(name.as_bytes());
                hash.update(&(bytes.len() as u64).to_le_bytes());
                hash.update(&bytes);
            }
            contract.commitment = DirectionalCommitment(*hash.finalize().as_bytes());
        }
        Ok(contract)
    }
}

impl OutputContract {
    /// Reject protected declarations and compile the exact outbound contract.
    ///
    /// # Errors
    /// Returns payload-free protected-domain or graph diagnostics.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn from_graph(graph: &AdmittedSchemaGraph) -> Result<Self, ValidationReport> {
        if graph.0.document.raw().get("x-nebula-conditions").is_some()
            || graph
                .0
                .document
                .raw()
                .get("x-nebula-local-conditions")
                .is_some()
        {
            return Err(
                ValidationError::builder("schema.direction.unsupported_facet")
                    .message("outbound codec cannot execute named condition declarations")
                    .private_source(UnsupportedDirectionalFacet::NamedConditions)
                    .build()
                    .into(),
            );
        }
        graph.ensure_public_output_domain()?;
        Ok(Self {
            graph: graph.clone(),
            commitment: directional_commitment(graph, SchemaDirection::Output),
        })
    }

    /// Admit one type-owned emitted contract for a retained leaf factory.
    ///
    /// # Errors
    /// Returns declaration or protected-domain diagnostics.
    pub fn for_type<T: crate::OutputCodec>() -> Result<Self, ValidationReport> {
        Self::from_graph(&T::output_definition()?)
    }
}

fn directional_commitment(
    graph: &AdmittedSchemaGraph,
    direction: SchemaDirection,
) -> DirectionalCommitment {
    let mut hash = blake3::Hasher::new();
    hash.update(b"nebula-directional-schema-contract");
    hash.update(&SCHEMA_CONTRACT_EVIDENCE_VERSION.to_le_bytes());
    hash.update(&GRAPH_CODEC_VERSION.to_le_bytes());
    hash.update(&super::SCHEMA_GRAPH_WIRE_VERSION.to_le_bytes());
    hash.update(&[match direction {
        SchemaDirection::Input => 0,
        SchemaDirection::Output => 1,
    }]);
    hash.update(graph.semantic_commitment().as_bytes());
    DirectionalCommitment(*hash.finalize().as_bytes())
}

macro_rules! contract_evidence {
    ($contract:ty, $direction:expr) => {
        impl $contract {
            /// Immutable graph admitted for this contract.
            #[must_use]
            pub const fn graph(&self) -> &AdmittedSchemaGraph {
                &self.graph
            }

            /// Exact directional semantic identity.
            #[must_use]
            pub const fn semantic_commitment(&self) -> &DirectionalCommitment {
                &self.commitment
            }

            /// Record evidence without serializing executable authority.
            #[must_use]
            pub fn record(&self) -> RecordedSchemaContract {
                RecordedSchemaContract {
                    version: SCHEMA_CONTRACT_EVIDENCE_VERSION,
                    codec_version: GRAPH_CODEC_VERSION,
                    graph_wire_version: super::SCHEMA_GRAPH_WIRE_VERSION,
                    direction: $direction,
                    document: self.graph.to_document(),
                    graph_semantic: *self.graph.semantic_commitment().as_bytes(),
                    graph_address: *self.graph.address_space_commitment().as_bytes(),
                    directional_commitment: self.commitment.0,
                }
            }
        }
    };
}
contract_evidence!(InputContract, SchemaDirection::Input);
contract_evidence!(OutputContract, SchemaDirection::Output);

impl OutputContract {
    /// Compare this emitted domain with a successor's retained output domain.
    #[must_use]
    pub fn explain_successor_of(&self, previous: &Self) -> crate::Assignability {
        super::explain_graph_successor(&self.graph, &previous.graph)
    }

    /// Compare this emitted domain with an inbound consumer domain.
    #[must_use]
    pub fn explain_assignable_to(&self, consumer: &InputContract) -> crate::Assignability {
        super::explain_graph_assignable(&self.graph, &consumer.graph)
    }

    /// Select one emitted occurrence without constructing a detached schema.
    ///
    /// # Errors
    /// Returns a bounded diagnostic for absent or ambiguous paths.
    pub fn reference_at(
        &self,
        path: &crate::ValuePath,
    ) -> Result<super::GraphReference<'_>, super::GraphReferenceError> {
        self.graph.reference_at(path)
    }
    /// Validate a candidate as literal outbound data without repairing its value.
    ///
    /// # Errors
    /// Returns payload-free shape, occurrence-policy and native rule diagnostics.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn validate_data(&self, candidate: &Value) -> Result<(), ValidationReport> {
        super::runtime::validate_output(&self.graph, candidate)
    }
}

impl InputContract {
    /// Admit a type-owned codec once and retain its private runtime custody.
    ///
    /// Type identity never enters persisted evidence or graph commitments.
    ///
    /// # Errors
    /// Rejects malformed type declarations or conditions.
    pub fn for_type<T: crate::InputCodec>() -> Result<Self, ValidationReport> {
        let mut contract = Self::from_graph(&T::input_definition()?)?;
        contract.codec_type = Some(std::any::TypeId::of::<T>());
        Ok(contract)
    }
    /// Classify the admitted root without projecting a legacy schema.
    #[must_use]
    pub fn root_kind(&self) -> super::GraphRootKind {
        self.graph.root_kind()
    }

    /// Select a canonical input field or a declared read alias.
    ///
    /// # Errors
    /// Returns a bounded diagnostic for absent or ambiguous fields.
    pub fn field(
        &self,
        key: &str,
    ) -> Result<super::GraphReference<'_>, super::GraphReferenceError> {
        self.graph
            .input_reference_at(&crate::ValuePath::root().push(key))
    }
}

/// Durable evidence. Deserialization never creates graph or codec authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedSchemaContract {
    version: u16,
    codec_version: u16,
    graph_wire_version: u16,
    direction: SchemaDirection,
    document: SchemaGraphDocument,
    graph_semantic: [u8; 32],
    graph_address: [u8; 32],
    directional_commitment: [u8; 32],
}

impl RecordedSchemaContract {
    /// Readmit retained inbound evidence under this exact supported epoch.
    ///
    /// # Errors
    /// Rejects unsupported epochs, polarity and any altered commitment.
    pub fn readmit_input(&self) -> Result<InputContract, ValidationReport> {
        InputContract::from_graph(&self.readmit(SchemaDirection::Input)?)
    }

    /// Readmit retained outbound evidence, including protected-domain admission.
    ///
    /// # Errors
    /// Rejects unsupported epochs, polarity and any altered commitment.
    pub fn readmit_output(&self) -> Result<OutputContract, ValidationReport> {
        OutputContract::from_graph(&self.readmit(SchemaDirection::Output)?)
    }

    fn readmit(&self, direction: SchemaDirection) -> Result<AdmittedSchemaGraph, ValidationReport> {
        if self.version != SCHEMA_CONTRACT_EVIDENCE_VERSION
            || self.codec_version != GRAPH_CODEC_VERSION
            || self.graph_wire_version != super::SCHEMA_GRAPH_WIRE_VERSION
            || self.direction != direction
        {
            return Err(evidence_error());
        }
        let graph = self
            .document
            .clone()
            .admit()
            .map_err(|error| error.report().clone())?;
        let actual = match direction {
            SchemaDirection::Input => *InputContract::from_graph(&graph)?.semantic_commitment(),
            SchemaDirection::Output => *OutputContract::from_graph(&graph)?.semantic_commitment(),
        };
        if graph.semantic_commitment().as_bytes() != &self.graph_semantic
            || graph.address_space_commitment().as_bytes() != &self.graph_address
            || actual.as_bytes() != &self.directional_commitment
        {
            return Err(evidence_error());
        }
        Ok(graph)
    }
}

fn evidence_error() -> ValidationReport {
    ValidationError::builder("schema.contract.evidence_mismatch")
        .message("retained schema codec evidence does not match the supported contract")
        .build()
        .into()
}
