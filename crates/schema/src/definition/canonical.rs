use std::{collections::BTreeMap, fmt};

use serde::Serialize;
use serde_json::Value;

use super::{
    MAX_GRAPH_CANONICAL_BYTES,
    model::{AdmissionIssue, Body, Definition, DefinitionIndex, DraftGraph, Edge, EdgeRole},
    number::{write_canonical_number, write_equality_number},
    rule_canonical::{write_rule, write_rules},
};

const SEMANTIC_DOMAIN: &[u8] = b"nebula-schema-graph-semantic";
const ADDRESS_DOMAIN: &[u8] = b"nebula-schema-graph-address-space";
const COMMITMENT_VERSION: u16 = 1;

/// Opaque commitment to executable graph semantics.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticCommitment([u8; 32]);

impl SemanticCommitment {
    /// Returns the versioned commitment digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SemanticCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SemanticCommitment(<opaque>)")
    }
}

/// Opaque commitment to authored definition and declaration addresses.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AddressSpaceCommitment([u8; 32]);

impl AddressSpaceCommitment {
    /// Returns the versioned commitment digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for AddressSpaceCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AddressSpaceCommitment(<opaque>)")
    }
}

pub(super) fn commitments(
    graph: &DraftGraph,
    lookup: &BTreeMap<super::DefinitionKey, DefinitionIndex>,
    canonical_numbers: &[u32],
) -> Result<(SemanticCommitment, AddressSpaceCommitment), AdmissionIssue> {
    let semantic = semantic_bytes(graph, lookup, canonical_numbers)?;
    let address = address_bytes(graph)?;
    Ok((
        SemanticCommitment(*blake3::hash(&semantic).as_bytes()),
        AddressSpaceCommitment(*blake3::hash(&address).as_bytes()),
    ))
}

fn semantic_bytes(
    graph: &DraftGraph,
    lookup: &BTreeMap<super::DefinitionKey, DefinitionIndex>,
    canonical_numbers: &[u32],
) -> Result<Vec<u8>, AdmissionIssue> {
    let mut bytes = Writer::new(SEMANTIC_DOMAIN);
    bytes.u16(COMMITMENT_VERSION)?;
    write_use(
        &mut bytes,
        EdgeRole::Root,
        &graph.root.0,
        lookup,
        canonical_numbers,
    )?;
    bytes.count(graph.definitions.len())?;
    let mut ordered = graph.definitions.iter().enumerate().collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|(index, _)| canonical_numbers[*index]);
    for (index, definition) in ordered {
        bytes.u32(canonical_numbers[index])?;
        write_body(&mut bytes, definition, lookup, canonical_numbers)?;
    }
    Ok(bytes.finish())
}

fn write_body(
    bytes: &mut Writer,
    definition: &Definition,
    lookup: &BTreeMap<super::DefinitionKey, DefinitionIndex>,
    canonical_numbers: &[u32],
) -> Result<(), AdmissionIssue> {
    match &definition.body {
        Body::Any => bytes.u8(0x10)?,
        Body::Null => bytes.u8(0x11)?,
        Body::Boolean { intrinsic_rules } => {
            bytes.u8(0x12)?;
            write_rules(bytes, intrinsic_rules)?;
        },
        Body::Integer(number) => {
            bytes.u8(0x13)?;
            write_number(bytes, number)?;
        },
        Body::Number(number) => {
            bytes.u8(0x14)?;
            write_number(bytes, number)?;
        },
        Body::String { intrinsic_rules } => {
            bytes.u8(0x15)?;
            write_rules(bytes, intrinsic_rules)?;
        },
        Body::Record {
            properties,
            intrinsic_rules,
        } => {
            bytes.u8(0x16)?;
            write_rules(bytes, intrinsic_rules)?;
            bytes.count(properties.len())?;
            for property in properties {
                bytes.string(property.key.as_str())?;
                write_presence(bytes, &property.presence)?;
                write_aliases(bytes, &property.aliases)?;
                write_use(
                    bytes,
                    EdgeRole::Property,
                    &property.core,
                    lookup,
                    canonical_numbers,
                )?;
            }
        },
        Body::Array(array) => {
            bytes.u8(0x17)?;
            bytes.u32(array.min_items)?;
            bytes.option_u32(array.max_items)?;
            bytes.u8(u8::from(array.unique))?;
            write_rules(bytes, &array.intrinsic_rules)?;
            write_use(
                bytes,
                EdgeRole::Element,
                &array.element.0,
                lookup,
                canonical_numbers,
            )?;
        },
        Body::Union(union) => {
            bytes.u8(0x18)?;
            bytes.serializable(&union.tagging)?;
            match &union.selector.default_variant {
                Some(key) => {
                    bytes.u8(1)?;
                    bytes.string(key.as_str())?;
                },
                None => bytes.u8(0)?,
            }
            bytes.count(union.selector.aliases.len())?;
            for (alias, target) in &union.selector.aliases {
                bytes.string(alias.as_str())?;
                bytes.string(target.as_str())?;
            }
            bytes.count(union.variants.len())?;
            for variant in &union.variants {
                bytes.string(variant.key.as_str())?;
                match &variant.payload {
                    Some(payload) => {
                        bytes.u8(1)?;
                        write_use(
                            bytes,
                            EdgeRole::VariantPayload,
                            &payload.0,
                            lookup,
                            canonical_numbers,
                        )?;
                    },
                    None => bytes.u8(0)?,
                }
            }
        },
        Body::Alias(alias) => {
            bytes.u8(0x19)?;
            write_use(bytes, EdgeRole::Alias, &alias.0, lookup, canonical_numbers)?;
        },
    }
    Ok(())
}

fn write_number(
    bytes: &mut Writer,
    number: &super::model::NumericBody,
) -> Result<(), AdmissionIssue> {
    bytes.option_number(number.minimum.as_ref())?;
    bytes.option_number(number.maximum.as_ref())?;
    write_rules(bytes, &number.intrinsic_rules)
}

fn write_aliases(
    bytes: &mut Writer,
    aliases: &super::model::DirectionalAliases,
) -> Result<(), AdmissionIssue> {
    bytes.count(aliases.read.len())?;
    for alias in &aliases.read {
        bytes.string(alias.as_str())?;
    }
    bytes.option_string(aliases.write.as_ref().map(crate::FieldKey::as_str))
}

fn write_use(
    bytes: &mut Writer,
    role: EdgeRole,
    use_site: &super::model::UseSiteCore,
    lookup: &BTreeMap<super::DefinitionKey, DefinitionIndex>,
    canonical_numbers: &[u32],
) -> Result<(), AdmissionIssue> {
    bytes.u8(role as u8)?;
    let target = lookup
        .get(&use_site.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    bytes.u32(canonical_numbers[target.0])?;
    write_null_policy(bytes, &use_site.null)?;
    write_empty_policy(bytes, &use_site.empty_string)?;
    write_empty_policy(bytes, &use_site.empty_collection)?;
    bytes.serializable(&use_site.expression)?;
    write_rules(bytes, &use_site.rules)?;
    bytes.serializable(&use_site.transformers)
}

fn write_presence(
    bytes: &mut Writer,
    policy: &super::model::PresencePolicy,
) -> Result<(), AdmissionIssue> {
    match policy {
        super::model::PresencePolicy::Optional => bytes.u8(0),
        super::model::PresencePolicy::Required => bytes.u8(1),
        super::model::PresencePolicy::RequiredWhen(rule) => {
            bytes.u8(2)?;
            write_rule(bytes, rule.root())
        },
    }
}

fn write_null_policy(
    bytes: &mut Writer,
    policy: &super::model::NullPolicy,
) -> Result<(), AdmissionIssue> {
    match policy {
        super::model::NullPolicy::Allow => bytes.u8(0),
        super::model::NullPolicy::Reject => bytes.u8(1),
        super::model::NullPolicy::RejectWhen(rule) => {
            bytes.u8(2)?;
            write_rule(bytes, rule.root())
        },
    }
}

fn write_empty_policy(
    bytes: &mut Writer,
    policy: &super::model::EmptyPolicy,
) -> Result<(), AdmissionIssue> {
    match policy {
        super::model::EmptyPolicy::Allow => bytes.u8(0),
        super::model::EmptyPolicy::Reject => bytes.u8(1),
        super::model::EmptyPolicy::RejectWhen(rule) => {
            bytes.u8(2)?;
            write_rule(bytes, rule.root())
        },
    }
}

fn address_bytes(graph: &DraftGraph) -> Result<Vec<u8>, AdmissionIssue> {
    let mut bytes = Writer::new(ADDRESS_DOMAIN);
    bytes.u16(COMMITMENT_VERSION)?;
    bytes.u8(EdgeRole::Root as u8)?;
    bytes.string(graph.root.0.target.as_str())?;
    bytes.count(graph.definitions.len())?;
    let mut definitions = graph.definitions.iter().collect::<Vec<_>>();
    definitions.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    for definition in definitions {
        bytes.string(definition.key.as_str())?;
        let mut edges = definition.edges()?;
        edges.sort_unstable_by(Edge::compare);
        bytes.count(edges.len())?;
        for edge in edges {
            bytes.u8(edge.role as u8)?;
            bytes.option_string(edge.local_key.as_ref().map(crate::FieldKey::as_str))?;
            bytes.u32(edge.ordinal)?;
            bytes.string(edge.target.as_str())?;
        }
        if let Body::Union(union) = &definition.body {
            for (ordinal, variant) in union.variants.iter().enumerate() {
                bytes.u8(0x80)?;
                bytes.string(variant.key.as_str())?;
                bytes.u32(u32::try_from(ordinal).map_err(|_| AdmissionIssue::IndexOverflow)?)?;
            }
        }
    }
    Ok(bytes.finish())
}

pub(super) struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new(domain: &[u8]) -> Self {
        Self {
            bytes: domain.to_vec(),
        }
    }
    fn reserve(&self, additional: usize) -> Result<(), AdmissionIssue> {
        let total = self
            .bytes
            .len()
            .checked_add(additional)
            .ok_or(AdmissionIssue::BudgetOverflow)?;
        if total > MAX_GRAPH_CANONICAL_BYTES {
            return Err(AdmissionIssue::CanonicalBytesLimit);
        }
        Ok(())
    }
    pub(super) fn u8(&mut self, value: u8) -> Result<(), AdmissionIssue> {
        self.reserve(1)?;
        self.bytes.push(value);
        Ok(())
    }
    fn u16(&mut self, value: u16) -> Result<(), AdmissionIssue> {
        self.reserve(2)?;
        self.bytes.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }
    fn u32(&mut self, value: u32) -> Result<(), AdmissionIssue> {
        self.reserve(4)?;
        self.bytes.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub(super) fn u64(&mut self, value: u64) -> Result<(), AdmissionIssue> {
        self.reserve(8)?;
        self.bytes.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub(super) fn count(&mut self, value: usize) -> Result<(), AdmissionIssue> {
        self.u32(u32::try_from(value).map_err(|_| AdmissionIssue::IndexOverflow)?)
    }
    pub(super) fn string(&mut self, value: &str) -> Result<(), AdmissionIssue> {
        self.count(value.len())?;
        self.reserve(value.len())?;
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }
    fn option_string(&mut self, value: Option<&str>) -> Result<(), AdmissionIssue> {
        match value {
            Some(value) => {
                self.u8(1)?;
                self.string(value)
            },
            None => self.u8(0),
        }
    }
    fn option_u32(&mut self, value: Option<u32>) -> Result<(), AdmissionIssue> {
        match value {
            Some(value) => {
                self.u8(1)?;
                self.u32(value)
            },
            None => self.u8(0),
        }
    }
    fn option_number(&mut self, value: Option<&serde_json::Number>) -> Result<(), AdmissionIssue> {
        match value {
            Some(value) => {
                self.u8(1)?;
                let mut encoded = Vec::new();
                write_canonical_number(value, &mut encoded)?;
                self.count(encoded.len())?;
                self.reserve(encoded.len())?;
                self.bytes.extend_from_slice(&encoded);
                Ok(())
            },
            None => self.u8(0),
        }
    }
    pub(super) fn number(&mut self, value: &serde_json::Number) -> Result<(), AdmissionIssue> {
        let mut encoded = Vec::new();
        write_canonical_number(value, &mut encoded)?;
        self.count(encoded.len())?;
        self.reserve(encoded.len())?;
        self.bytes.extend_from_slice(&encoded);
        Ok(())
    }
    pub(super) fn exact_json(&mut self, value: &Value) -> Result<(), AdmissionIssue> {
        let mut encoded = Vec::new();
        canonical_json_mode(value, &mut encoded, false)?;
        self.count(encoded.len())?;
        self.reserve(encoded.len())?;
        self.bytes.extend_from_slice(&encoded);
        Ok(())
    }
    fn serializable(&mut self, value: &impl Serialize) -> Result<(), AdmissionIssue> {
        let value = serde_json::to_value(value).map_err(|_| AdmissionIssue::InvalidDocument)?;
        let mut encoded = Vec::new();
        canonical_json(&value, &mut encoded)?;
        self.count(encoded.len())?;
        self.reserve(encoded.len())?;
        self.bytes.extend_from_slice(&encoded);
        Ok(())
    }
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

fn canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<(), AdmissionIssue> {
    canonical_json_mode(value, output, true)
}

fn canonical_json_mode(
    value: &Value,
    output: &mut Vec<u8>,
    normalize_numbers: bool,
) -> Result<(), AdmissionIssue> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(value) if normalize_numbers => write_canonical_number(value, output)?,
        Value::Number(value) => write_equality_number(value, output)?,
        Value::String(value) => output.extend_from_slice(
            &serde_json::to_vec(value).map_err(|_| AdmissionIssue::InvalidDocument)?,
        ),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                canonical_json_mode(value, output, normalize_numbers)?;
            }
            output.push(b']');
        },
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    &serde_json::to_vec(key).map_err(|_| AdmissionIssue::InvalidDocument)?,
                );
                output.push(b':');
                canonical_json_mode(value, output, normalize_numbers)?;
            }
            output.push(b'}');
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_writer_accepts_exact_limit_and_rejects_one_over() {
        let exact = Writer {
            bytes: vec![0; MAX_GRAPH_CANONICAL_BYTES],
        };
        assert!(exact.reserve(0).is_ok());

        let mut over = Writer {
            bytes: vec![0; MAX_GRAPH_CANONICAL_BYTES],
        };
        assert_eq!(over.u8(0), Err(AdmissionIssue::CanonicalBytesLimit));
    }

    #[test]
    fn canonical_edge_role_tags_have_the_frozen_rank_order() {
        assert_eq!(
            [
                EdgeRole::Root as u8,
                EdgeRole::Alias as u8,
                EdgeRole::Property as u8,
                EdgeRole::Element as u8,
                EdgeRole::VariantPayload as u8,
            ],
            [0, 1, 2, 3, 4]
        );
    }
}
