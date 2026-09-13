use serde::{Deserialize, Serialize};

use crate::{FieldKey, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct DefinitionKey(FieldKey);

impl DefinitionKey {
    pub(super) fn new(value: impl AsRef<str>) -> Result<Self, ValidationError> {
        FieldKey::new(value).map(Self)
    }

    pub(super) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialOrd for DefinitionKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DefinitionKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct TypeRef(DefinitionKey);

impl TypeRef {
    pub(super) const fn new(target: DefinitionKey) -> Self {
        Self(target)
    }

    pub(super) const fn target_key(&self) -> &DefinitionKey {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct DefinitionDraft {
    pub(super) key: DefinitionKey,
    pub(super) body: DefinitionBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SchemaDraft {
    pub(super) root: TypeRef,
    pub(super) definitions: Vec<DefinitionDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DefinitionBody {
    String,
    Record(Vec<PropertyEdge>),
    Array {
        item: TypeRef,
        min_items: u32,
        max_items: Option<u32>,
        unique: bool,
    },
    Union(Vec<VariantEdge>),
    Alias(TypeRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Presence {
    Required,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PropertyEdge {
    pub(super) key: FieldKey,
    pub(super) presence: Presence,
    pub(super) target: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct VariantEdge {
    pub(super) key: FieldKey,
    pub(super) target: Option<TypeRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EdgeRole {
    Root,
    RequiredProperty,
    OptionalProperty,
    ArrayItem,
    UnionPayload,
    UnionUnit,
    Alias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GraphLocation {
    Root,
    Definition {
        index: usize,
    },
    Edge {
        definition: usize,
        ordinal: u32,
        role: EdgeRole,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GraphDiagnostic {
    pub(super) code: &'static str,
    pub(super) location: GraphLocation,
}

impl GraphDiagnostic {
    pub(super) const fn root(code: &'static str) -> Self {
        Self {
            code,
            location: GraphLocation::Root,
        }
    }

    pub(super) const fn definition(code: &'static str, index: usize) -> Self {
        Self {
            code,
            location: GraphLocation::Definition { index },
        }
    }

    pub(super) const fn edge(
        code: &'static str,
        definition: usize,
        ordinal: u32,
        role: EdgeRole,
    ) -> Self {
        Self {
            code,
            location: GraphLocation::Edge {
                definition,
                ordinal,
                role,
            },
        }
    }
}
