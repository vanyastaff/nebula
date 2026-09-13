use super::{
    admission::{AdmissionSeal, AdmittedBodyRef, AdmittedDefinition, ResolvedRef},
    graph::{DefinitionKey, EdgeRole, GraphDiagnostic},
};

pub(super) const PROTOTYPE_DOMAIN: &[u8] = b"nebula-property-graph-prototype";
pub(super) const PROTOTYPE_VERSION: u16 = 1;

pub(super) const TAG_ROOT_EDGE: u8 = 0x10;
pub(super) const TAG_REQUIRED_PROPERTY_EDGE: u8 = 0x11;
pub(super) const TAG_OPTIONAL_PROPERTY_EDGE: u8 = 0x12;
pub(super) const TAG_ARRAY_ITEM_EDGE: u8 = 0x13;
pub(super) const TAG_UNION_PAYLOAD_EDGE: u8 = 0x14;
pub(super) const TAG_UNION_UNIT: u8 = 0x15;
pub(super) const TAG_ALIAS_EDGE: u8 = 0x16;

const TAG_STRING: u8 = 0x20;
const TAG_RECORD: u8 = 0x21;
const TAG_ARRAY: u8 = 0x22;
const TAG_UNION: u8 = 0x23;
const TAG_ALIAS: u8 = 0x24;
const TAG_DEFINITION: u8 = 0x30;
const TAG_OPTION_NONE: u8 = 0x00;
const TAG_OPTION_SOME: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GraphCommitment([u8; 32]);

impl GraphCommitment {
    pub(super) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Private prototype grammar, with no production wire allocation or compatibility claim:
///
/// `domain || version:u16be || root-edge || definition-count:u32be || definitions*`.
/// Each sorted definition is `definition-tag || key:string || body`; strings are
/// `length:u16be || UTF-8`, counts and array cardinalities are `u32be`, and bool/option
/// values use one-byte tags. Body tags distinguish string, record, array, union, and alias;
/// an alias is `alias-body-tag || alias-edge-tag || target-key`.
/// Edge tags distinguish root, required property, optional property, array item, union payload,
/// union unit, and alias. Definitions, properties, and variants are already sorted by admission.
/// Every reference encodes its authored `DefinitionKey`, never its admitted lookup index.
/// The completed finite buffer is hashed exactly once; no node or recursive content hashes exist.
pub(super) fn encode_graph(
    _seal: &AdmissionSeal,
    root: &ResolvedRef,
    definitions: &[AdmittedDefinition],
) -> Result<(Vec<u8>, GraphCommitment), GraphDiagnostic> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(PROTOTYPE_DOMAIN);
    bytes.extend_from_slice(&PROTOTYPE_VERSION.to_be_bytes());
    write_reference(&mut bytes, root)?;
    write_count(&mut bytes, definitions.len())?;
    for definition in definitions {
        bytes.push(TAG_DEFINITION);
        write_key(&mut bytes, definition.key())?;
        write_body(&mut bytes, definition.body())?;
    }
    let commitment = GraphCommitment(*blake3::hash(&bytes).as_bytes());
    Ok((bytes, commitment))
}

fn write_body(bytes: &mut Vec<u8>, body: AdmittedBodyRef<'_>) -> Result<(), GraphDiagnostic> {
    match body {
        AdmittedBodyRef::String => bytes.push(TAG_STRING),
        AdmittedBodyRef::Record(properties) => {
            bytes.push(TAG_RECORD);
            write_count(bytes, properties.len())?;
            for property in properties {
                write_reference(bytes, property.target())?;
                write_field_key(bytes, property.key())?;
            }
        },
        AdmittedBodyRef::Array {
            item,
            min_items,
            max_items,
            unique,
        } => {
            bytes.push(TAG_ARRAY);
            write_reference(bytes, item)?;
            bytes.extend_from_slice(&min_items.to_be_bytes());
            match max_items {
                None => bytes.push(TAG_OPTION_NONE),
                Some(maximum) => {
                    bytes.push(TAG_OPTION_SOME);
                    bytes.extend_from_slice(&maximum.to_be_bytes());
                },
            }
            bytes.push(u8::from(unique));
        },
        AdmittedBodyRef::Union(variants) => {
            bytes.push(TAG_UNION);
            write_count(bytes, variants.len())?;
            for variant in variants {
                match variant.target() {
                    Some(target) => write_reference(bytes, target)?,
                    None => bytes.push(TAG_UNION_UNIT),
                }
                write_field_key(bytes, variant.key())?;
            }
        },
        AdmittedBodyRef::Alias(target) => {
            bytes.push(TAG_ALIAS);
            write_reference(bytes, target)?;
        },
    }
    Ok(())
}

fn write_reference(bytes: &mut Vec<u8>, reference: &ResolvedRef) -> Result<(), GraphDiagnostic> {
    bytes.push(match reference.role() {
        EdgeRole::Root => TAG_ROOT_EDGE,
        EdgeRole::RequiredProperty => TAG_REQUIRED_PROPERTY_EDGE,
        EdgeRole::OptionalProperty => TAG_OPTIONAL_PROPERTY_EDGE,
        EdgeRole::ArrayItem => TAG_ARRAY_ITEM_EDGE,
        EdgeRole::UnionPayload => TAG_UNION_PAYLOAD_EDGE,
        EdgeRole::UnionUnit => TAG_UNION_UNIT,
        EdgeRole::Alias => TAG_ALIAS_EDGE,
    });
    write_key(bytes, reference.target().target_key())
}

fn write_key(bytes: &mut Vec<u8>, key: &DefinitionKey) -> Result<(), GraphDiagnostic> {
    write_string(bytes, key.as_str())
}

fn write_field_key(bytes: &mut Vec<u8>, key: &crate::FieldKey) -> Result<(), GraphDiagnostic> {
    write_string(bytes, key.as_str())
}

fn write_string(bytes: &mut Vec<u8>, value: &str) -> Result<(), GraphDiagnostic> {
    let length =
        u16::try_from(value.len()).map_err(|_| GraphDiagnostic::root("graph.index_overflow"))?;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn write_count(bytes: &mut Vec<u8>, value: usize) -> Result<(), GraphDiagnostic> {
    let count = u32::try_from(value).map_err(|_| GraphDiagnostic::root("graph.index_overflow"))?;
    bytes.extend_from_slice(&count.to_be_bytes());
    Ok(())
}
