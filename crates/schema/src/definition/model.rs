use std::{cmp::Ordering, collections::BTreeMap, fmt};

use nebula_validator::Rule;
use serde_json::{Map, Number, Value};

use crate::{ExpressionMode, FieldKey, SerdeTagging, Transformer};

/// A checked authored anchor for one graph definition.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DefinitionKey(FieldKey);

impl DefinitionKey {
    /// Parses a checked definition anchor.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key` when the value is not a valid schema identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_schema::DefinitionKey;
    ///
    /// let key = DefinitionKey::new("invoice")?;
    /// assert_eq!(key.as_str(), "invoice");
    /// # Ok::<(), nebula_schema::ValidationError>(())
    /// ```
    pub fn new(value: impl AsRef<str>) -> Result<Self, crate::ValidationError> {
        FieldKey::new(value).map(Self)
    }

    /// Borrows the identifier bytes.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(super) fn parse(value: &Value) -> Result<Self, AdmissionIssue> {
        value
            .as_str()
            .ok_or(AdmissionIssue::InvalidIdentifier)
            .and_then(|raw| Self::new(raw).map_err(|_| AdmissionIssue::InvalidIdentifier))
    }
}

impl fmt::Debug for DefinitionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DefinitionKey(<redacted>)")
    }
}

impl PartialOrd for DefinitionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DefinitionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct DefinitionIndex(pub(super) usize);

#[derive(Debug, Clone)]
pub(super) struct DraftGraph {
    pub(super) root: RootUse,
    pub(super) definitions: Vec<Definition>,
}

#[derive(Debug, Clone)]
pub(super) struct Definition {
    pub(super) key: DefinitionKey,
    pub(super) body: Body,
}

#[derive(Debug, Clone)]
pub(super) enum Body {
    Any,
    Null,
    Boolean {
        intrinsic_rules: Vec<Rule>,
    },
    Integer(NumericBody),
    Number(NumericBody),
    String {
        intrinsic_rules: Vec<Rule>,
    },
    Bytes,
    Record {
        properties: Vec<PropertyUse>,
        additional_properties: AdditionalProperties,
        intrinsic_rules: Vec<Rule>,
    },
    Array(ArrayBody),
    Union(UnionBody),
    Alias(AliasUse),
}

#[derive(Debug, Clone)]
pub(super) struct NumericBody {
    pub(super) minimum: Option<Number>,
    pub(super) maximum: Option<Number>,
    pub(super) intrinsic_rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub(super) struct ArrayBody {
    pub(super) element: ElementUse,
    pub(super) min_items: u32,
    pub(super) max_items: Option<u32>,
    pub(super) unique: bool,
    pub(super) intrinsic_rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub(super) struct UnionBody {
    pub(super) variants: Vec<VariantUse>,
    pub(super) tagging: SerdeTagging,
    pub(super) selector: SelectorNormalization,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SelectorNormalization {
    pub(super) default_variant: Option<FieldKey>,
    pub(super) aliases: Vec<(FieldKey, FieldKey)>,
}

#[derive(Debug, Clone)]
pub(super) struct UseSiteCore {
    pub(super) target: DefinitionKey,
    pub(super) null: NullPolicy,
    pub(super) empty_string: EmptyPolicy,
    pub(super) empty_collection: EmptyPolicy,
    pub(super) expression: ExpressionMode,
    pub(super) protection: ValueProtection,
    pub(super) accepted_domain: AcceptedDomain,
    pub(super) rules: Vec<Rule>,
    pub(super) transformers: Vec<Transformer>,
}

#[derive(Debug, Clone)]
pub(super) struct RootUse(pub(super) UseSiteCore);

#[derive(Debug, Clone)]
pub(super) struct PropertyUse {
    pub(super) key: FieldKey,
    pub(super) presence: PresencePolicy,
    pub(super) aliases: DirectionalAliases,
    pub(super) input_default: Option<Value>,
    pub(super) core: UseSiteCore,
}

#[derive(Debug, Clone)]
pub(super) enum AdditionalProperties {
    Open,
    Closed,
    Typed(Box<UseSiteCore>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ValueProtection {
    #[default]
    Public,
    SecretUtf8,
    SecretBytes,
}

#[derive(Debug, Clone, Default)]
pub(super) enum AcceptedDomain {
    #[default]
    Open,
    Closed(Vec<Value>),
}

#[derive(Debug, Clone)]
pub(super) struct ElementUse(pub(super) UseSiteCore);

#[derive(Debug, Clone)]
pub(super) struct VariantUse {
    pub(super) key: FieldKey,
    pub(super) payload: Option<PayloadUse>,
}

#[derive(Debug, Clone)]
pub(super) struct PayloadUse(pub(super) UseSiteCore);

#[derive(Debug, Clone)]
pub(super) struct AliasUse(pub(super) UseSiteCore);

#[derive(Debug, Clone, Default)]
pub(super) struct DirectionalAliases {
    pub(super) read: Vec<FieldKey>,
    pub(super) write: Option<FieldKey>,
}

#[derive(Debug, Clone)]
pub(super) enum PresencePolicy {
    Required,
    Optional,
    RequiredWhen(Rule),
}

#[derive(Debug, Clone)]
pub(super) enum NullPolicy {
    Allow,
    Reject,
    RejectWhen(Rule),
}

#[derive(Debug, Clone)]
pub(super) enum EmptyPolicy {
    Allow,
    Reject,
    RejectWhen(Rule),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EdgeRole {
    Root = 0,
    Alias = 1,
    Property = 2,
    Element = 3,
    VariantPayload = 4,
    /// Typed dynamic record values have a frozen tag distinct from their canonical sort rank.
    AdditionalProperty = 5,
}

#[derive(Debug, Clone)]
pub(super) struct Edge {
    pub(super) role: EdgeRole,
    pub(super) local_key: Option<FieldKey>,
    pub(super) ordinal: u32,
    pub(super) target: DefinitionKey,
}

impl Edge {
    pub(super) fn compare(left: &Self, right: &Self) -> Ordering {
        left.role
            .canonical_rank()
            .cmp(&right.role.canonical_rank())
            .then_with(|| {
                left.local_key
                    .as_ref()
                    .map(FieldKey::as_str)
                    .cmp(&right.local_key.as_ref().map(FieldKey::as_str))
            })
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    }
}

impl EdgeRole {
    const fn canonical_rank(self) -> u8 {
        match self {
            Self::Root => 0,
            Self::Alias => 1,
            Self::Property => 2,
            Self::AdditionalProperty => 3,
            Self::Element => 4,
            Self::VariantPayload => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionIssue {
    InvalidDocument,
    UnsupportedVersion,
    UnknownMember,
    UnknownBody,
    UnknownRequiredExtension,
    InvalidIdentifier,
    DuplicateDefinition,
    DuplicateLocalKey,
    DefinitionLimit,
    ReferenceLimit,
    IdentifierBytesLimit,
    BudgetOverflow,
    DanglingReference,
    UnreachableDefinition,
    NonproductiveDefinition,
    InvalidBounds,
    InvalidRule,
    InvalidTransformer,
    InapplicableFacet,
    InvalidDefault,
    CanonicalBytesLimit,
    DiagnosticsLimit,
    IndexOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionLocation {
    Root,
    Definition {
        ordinal: u32,
    },
    Use {
        definition: u32,
        role: EdgeRole,
        ordinal: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdmissionDiagnostic {
    pub(super) issue: AdmissionIssue,
    pub(super) location: AdmissionLocation,
}

impl AdmissionDiagnostic {
    pub(super) const fn root(issue: AdmissionIssue) -> Self {
        Self {
            issue,
            location: AdmissionLocation::Root,
        }
    }
}

pub(super) fn object(value: &Value) -> Result<&Map<String, Value>, AdmissionIssue> {
    value.as_object().ok_or(AdmissionIssue::InvalidDocument)
}

pub(super) fn check_members(
    object: &Map<String, Value>,
    known: &[&str],
) -> Result<(), AdmissionIssue> {
    for key in object.keys() {
        if !known.contains(&key.as_str()) && !key.starts_with("x-") {
            return Err(AdmissionIssue::UnknownMember);
        }
    }
    Ok(())
}

pub(super) fn parse_rules(value: Option<&Value>) -> Result<Vec<Rule>, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let rules = value.as_array().ok_or(AdmissionIssue::InvalidRule)?;
    rules
        .iter()
        .map(|rule| {
            let rule: Rule =
                serde_json::from_value(rule.clone()).map_err(|_| AdmissionIssue::InvalidRule)?;
            rule.check_limits()
                .map_err(|_| AdmissionIssue::InvalidRule)?;
            Ok(rule)
        })
        .collect()
}

pub(super) fn parse_transformers(
    value: Option<&Value>,
) -> Result<Vec<Transformer>, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or(AdmissionIssue::InvalidTransformer)?;
    values
        .iter()
        .map(|item| {
            serde_json::from_value(item.clone()).map_err(|_| AdmissionIssue::InvalidTransformer)
        })
        .collect()
}

pub(super) fn parse_field_key(value: &Value) -> Result<FieldKey, AdmissionIssue> {
    value
        .as_str()
        .ok_or(AdmissionIssue::InvalidIdentifier)
        .and_then(|raw| FieldKey::new(raw).map_err(|_| AdmissionIssue::InvalidIdentifier))
}

pub(super) type DefinitionLookup = BTreeMap<DefinitionKey, DefinitionIndex>;
