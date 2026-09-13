//! Authoritative root shapes; field indexes are derived, never a second shape.

use nebula_validator::Rule;

use crate::{Field, RequiredMode, ValidationError, ValidationReport};

use super::{ScalarSchema, SchemaKind, SerdeTagging};

/// The complete structural contract of a schema root.
#[derive(Debug, Clone, PartialEq)]
pub enum RootShape {
    /// Deliberately unknown JSON data, with no expression admission.
    Any,
    /// A scalar JSON value with an explicit domain.
    Scalar(ScalarSchema),
    /// An object with declared fields and root rules.
    Record(RecordShape),
    /// A tagged sum type with its recorded serde wire convention.
    Union(UnionShape),
}

/// Checked record contents. Construction belongs to the schema builder.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordShape {
    fields: Vec<Field>,
    root_rules: Vec<Rule>,
}

/// Checked union contents. The single field is always a required mode.
#[derive(Debug, Clone, PartialEq)]
pub struct UnionShape {
    field: Box<Field>,
    tagging: SerdeTagging,
}

impl RootShape {
    pub(crate) fn record(fields: Vec<Field>, root_rules: Vec<Rule>) -> Self {
        Self::Record(RecordShape { fields, root_rules })
    }

    pub(crate) fn union(
        mut fields: Vec<Field>,
        tagging: SerdeTagging,
    ) -> Result<Self, ValidationReport> {
        if fields.len() != 1 {
            return Err(invalid_union());
        }
        let Some(field @ Field::Mode(_)) = fields.pop() else {
            return Err(invalid_union());
        };
        if field.required() != &RequiredMode::Always {
            return Err(invalid_union());
        }
        Ok(Self::Union(UnionShape {
            field: Box::new(field),
            tagging,
        }))
    }

    /// Classification derived from the owned shape.
    #[must_use]
    pub const fn kind(&self) -> SchemaKind {
        match self {
            Self::Any => SchemaKind::Any,
            Self::Scalar(_) => SchemaKind::Scalar,
            Self::Record(_) => SchemaKind::Record,
            Self::Union(_) => SchemaKind::Union,
        }
    }

    /// Declared fields. Scalar and unknown roots do not have properties.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        match self {
            Self::Record(record) => record.fields(),
            Self::Union(union) => std::slice::from_ref(union.field.as_ref()),
            Self::Any | Self::Scalar(_) => &[],
        }
    }

    /// Rules on the complete submitted value, without synthetic property keys.
    #[must_use]
    pub fn root_rules(&self) -> &[Rule] {
        match self {
            Self::Record(record) => record.root_rules(),
            Self::Scalar(scalar) => scalar.root_rules(),
            Self::Any | Self::Union(_) => &[],
        }
    }

    /// Wire tagging exists only for a union.
    #[must_use]
    pub const fn serde_tagging(&self) -> Option<&SerdeTagging> {
        match self {
            Self::Union(union) => Some(union.tagging()),
            _ => None,
        }
    }
}

impl RecordShape {
    /// The ordered record declarations.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Rules on the whole object.
    #[must_use]
    pub fn root_rules(&self) -> &[Rule] {
        &self.root_rules
    }
}

impl UnionShape {
    /// The checked root mode declaration.
    #[must_use]
    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The enum's serde tagging convention.
    #[must_use]
    pub const fn tagging(&self) -> &SerdeTagging {
        &self.tagging
    }
}

fn invalid_union() -> ValidationReport {
    ValidationError::builder("union.invalid_root")
        .message("a union requires exactly one required mode field")
        .build()
        .into()
}
