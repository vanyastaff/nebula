use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet, VecDeque},
    fmt,
    sync::Arc,
};

use serde_json::{Map, Value};

use nebula_validator::{DiagnosticDisclosure, ExecutionMode, Rule, RuleRef, RuleView, ValueRule};

use crate::{ExpressionMode, FieldKey, SerdeTagging, ValidationError, ValidationReport};

use super::{
    MAX_GRAPH_DEFINITIONS, MAX_GRAPH_DIAGNOSTICS, MAX_GRAPH_IDENTIFIER_BYTES, MAX_GRAPH_REFERENCES,
    SCHEMA_GRAPH_WIRE_VERSION,
    canonical::{AddressSpaceCommitment, SemanticCommitment, commitments, exact_json_bytes},
    document::SchemaGraphDocument,
    error::SchemaAdmissionError,
    model::{
        AcceptedDomain, AdditionalProperties, AdmissionDiagnostic, AdmissionIssue,
        AdmissionLocation, AliasUse, ArrayBody, Body, Definition, DefinitionIndex, DefinitionKey,
        DefinitionLookup, DirectionalAliases, DraftGraph, Edge, EdgeRole, ElementUse, EmptyPolicy,
        NullPolicy, NumericBody, PayloadUse, PresencePolicy, PropertyUse, RootUse,
        SelectorNormalization, UnionBody, UseSiteCore, ValueProtection, VariantUse, check_members,
        object, parse_field_key, parse_rules, parse_transformers,
    },
    number::compare_numbers,
};

mod keys;

pub use keys::{DeclarationAddress, DeclarationUse, DefinitionMemberKey};

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

fn parse_document(raw: &Value) -> Result<DraftGraph, AdmissionIssue> {
    let root_object = object(raw)?;
    check_members(
        root_object,
        &["version", "root", "definitions", "required_extensions"],
    )?;
    let version = root_object
        .get("version")
        .and_then(Value::as_u64)
        .ok_or(AdmissionIssue::InvalidDocument)?;
    if version != u64::from(SCHEMA_GRAPH_WIRE_VERSION) {
        return Err(AdmissionIssue::UnsupportedVersion);
    }
    if root_object
        .get("required_extensions")
        .is_some_and(|value| value.as_array().is_none_or(|items| !items.is_empty()))
    {
        return Err(AdmissionIssue::UnknownRequiredExtension);
    }
    let root = RootUse(parse_use(
        object(
            root_object
                .get("root")
                .ok_or(AdmissionIssue::InvalidDocument)?,
        )?,
        &["target"],
    )?);
    let definitions = root_object
        .get("definitions")
        .and_then(Value::as_array)
        .ok_or(AdmissionIssue::InvalidDocument)?;
    if definitions.is_empty() {
        return Err(AdmissionIssue::InvalidDocument);
    }
    if definitions.len() > MAX_GRAPH_DEFINITIONS {
        return Err(AdmissionIssue::DefinitionLimit);
    }
    let definitions = definitions
        .iter()
        .map(parse_definition)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DraftGraph { root, definitions })
}

fn parse_definition(value: &Value) -> Result<Definition, AdmissionIssue> {
    let definition = object(value)?;
    check_members(definition, &["key", "body"])?;
    let key = DefinitionKey::parse(
        definition
            .get("key")
            .ok_or(AdmissionIssue::InvalidDocument)?,
    )?;
    let body = parse_body(object(
        definition
            .get("body")
            .ok_or(AdmissionIssue::InvalidDocument)?,
    )?)?;
    Ok(Definition { key, body })
}

fn parse_body(body: &Map<String, Value>) -> Result<Body, AdmissionIssue> {
    let kind = body
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(AdmissionIssue::InvalidDocument)?;
    match kind {
        "any" => {
            check_members(body, &["kind"])?;
            Ok(Body::Any)
        },
        "null" => {
            check_members(body, &["kind"])?;
            Ok(Body::Null)
        },
        "boolean" => {
            check_members(body, &["kind", "intrinsic_rules"])?;
            Ok(Body::Boolean {
                intrinsic_rules: parse_rules(body.get("intrinsic_rules"))?,
            })
        },
        "integer" => parse_numeric(body).map(Body::Integer),
        "number" => parse_numeric(body).map(Body::Number),
        "string" => {
            check_members(body, &["kind", "intrinsic_rules"])?;
            Ok(Body::String {
                intrinsic_rules: parse_rules(body.get("intrinsic_rules"))?,
            })
        },
        "bytes" => {
            check_members(body, &["kind", "encoding"])?;
            if body.get("encoding").and_then(Value::as_str) != Some("base64") {
                return Err(AdmissionIssue::InvalidDocument);
            }
            Ok(Body::Bytes)
        },
        "record" => parse_record(body),
        "array" => parse_array(body),
        "union" => parse_union(body),
        "alias" => {
            check_members(body, &["kind", "alias"])?;
            let alias = object(body.get("alias").ok_or(AdmissionIssue::InvalidDocument)?)?;
            Ok(Body::Alias(AliasUse(parse_use(alias, &["target"])?)))
        },
        _ => Err(AdmissionIssue::UnknownBody),
    }
}

fn parse_numeric(body: &Map<String, Value>) -> Result<NumericBody, AdmissionIssue> {
    check_members(body, &["kind", "minimum", "maximum", "intrinsic_rules"])?;
    let minimum = body.get("minimum").map(parse_number).transpose()?;
    let maximum = body.get("maximum").map(parse_number).transpose()?;
    Ok(NumericBody {
        minimum,
        maximum,
        intrinsic_rules: parse_rules(body.get("intrinsic_rules"))?,
    })
}

fn parse_number(value: &Value) -> Result<serde_json::Number, AdmissionIssue> {
    value
        .as_number()
        .cloned()
        .ok_or(AdmissionIssue::InvalidBounds)
}

fn parse_record(body: &Map<String, Value>) -> Result<Body, AdmissionIssue> {
    check_members(
        body,
        &[
            "kind",
            "properties",
            "additional_properties",
            "intrinsic_rules",
        ],
    )?;
    let properties = body
        .get("properties")
        .and_then(Value::as_array)
        .ok_or(AdmissionIssue::InvalidDocument)?;
    let properties = properties
        .iter()
        .map(parse_property)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Body::Record {
        properties,
        additional_properties: parse_additional_properties(body.get("additional_properties"))?,
        intrinsic_rules: parse_rules(body.get("intrinsic_rules"))?,
    })
}

fn parse_additional_properties(
    value: Option<&Value>,
) -> Result<AdditionalProperties, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(AdditionalProperties::Open);
    };
    match value.as_str() {
        Some("open") => Ok(AdditionalProperties::Open),
        Some("closed") => Ok(AdditionalProperties::Closed),
        Some(_) => Err(AdmissionIssue::InvalidDocument),
        None => {
            let policy = object(value)?;
            check_members(policy, &["typed"])?;
            let typed = object(policy.get("typed").ok_or(AdmissionIssue::InvalidDocument)?)?;
            parse_use(typed, &["target"])
                .map(Box::new)
                .map(AdditionalProperties::Typed)
        },
    }
}

fn parse_property(value: &Value) -> Result<PropertyUse, AdmissionIssue> {
    let property = object(value)?;
    check_members(
        property,
        &[
            "key",
            "target",
            "presence",
            "null",
            "empty_string",
            "empty_collection",
            "expression",
            "rules",
            "transformers",
            "aliases",
            "input_default",
            "protection",
            "accepted_domain",
        ],
    )?;
    let key = parse_field_key(property.get("key").ok_or(AdmissionIssue::InvalidDocument)?)?;
    let presence = parse_presence(property.get("presence"))?;
    let aliases = parse_aliases(property.get("aliases"))?;
    Ok(PropertyUse {
        key,
        presence,
        aliases,
        input_default: property.get("input_default").map(canonical_literal),
        core: parse_use(property, &["key", "presence", "aliases", "input_default"])?,
    })
}

fn parse_aliases(value: Option<&Value>) -> Result<DirectionalAliases, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(DirectionalAliases::default());
    };
    let aliases = object(value)?;
    check_members(aliases, &["read", "write"])?;
    let read = aliases.get("read").map_or(Ok(Vec::new()), |value| {
        value
            .as_array()
            .ok_or(AdmissionIssue::InvalidIdentifier)?
            .iter()
            .map(parse_field_key)
            .collect()
    })?;
    let write = aliases
        .get("write")
        .filter(|value| !value.is_null())
        .map(parse_field_key)
        .transpose()?;
    Ok(DirectionalAliases { read, write })
}

fn parse_array(body: &Map<String, Value>) -> Result<Body, AdmissionIssue> {
    check_members(
        body,
        &[
            "kind",
            "element",
            "min_items",
            "max_items",
            "unique",
            "intrinsic_rules",
        ],
    )?;
    let element = object(body.get("element").ok_or(AdmissionIssue::InvalidDocument)?)?;
    let min_items = parse_u32(body.get("min_items"), 0)?;
    let max_items = body
        .get("max_items")
        .filter(|value| !value.is_null())
        .map(|value| parse_u32(Some(value), 0))
        .transpose()?;
    Ok(Body::Array(ArrayBody {
        element: ElementUse(parse_use(element, &["target"])?),
        min_items,
        max_items,
        unique: body.get("unique").map_or(Ok(false), |value| {
            value.as_bool().ok_or(AdmissionIssue::InvalidDocument)
        })?,
        intrinsic_rules: parse_rules(body.get("intrinsic_rules"))?,
    }))
}

fn parse_u32(value: Option<&Value>, default: u32) -> Result<u32, AdmissionIssue> {
    value.map_or(Ok(default), |value| {
        value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(AdmissionIssue::InvalidBounds)
    })
}

fn parse_union(body: &Map<String, Value>) -> Result<Body, AdmissionIssue> {
    check_members(
        body,
        &["kind", "variants", "tagging", "selector_normalization"],
    )?;
    let variants = body
        .get("variants")
        .and_then(Value::as_array)
        .ok_or(AdmissionIssue::InvalidDocument)?;
    let variants = variants
        .iter()
        .map(parse_variant)
        .collect::<Result<Vec<_>, _>>()?;
    let tagging = body
        .get("tagging")
        .map_or(Ok(SerdeTagging::External), |value| {
            serde_json::from_value(value.clone()).map_err(|_| AdmissionIssue::InvalidDocument)
        })?;
    if let SerdeTagging::Adjacent { tag, content } = &tagging
        && tag == content
    {
        return Err(AdmissionIssue::InvalidDocument);
    }
    let selector = parse_selector(body.get("selector_normalization"))?;
    Ok(Body::Union(UnionBody {
        variants,
        tagging,
        selector,
    }))
}

fn parse_variant(value: &Value) -> Result<VariantUse, AdmissionIssue> {
    let variant = object(value)?;
    check_members(variant, &["key", "payload"])?;
    let key = parse_field_key(variant.get("key").ok_or(AdmissionIssue::InvalidDocument)?)?;
    let payload = variant
        .get("payload")
        .filter(|value| !value.is_null())
        .map(|value| {
            object(value).and_then(|payload| parse_use(payload, &["target"]).map(PayloadUse))
        })
        .transpose()?;
    Ok(VariantUse { key, payload })
}

fn parse_selector(value: Option<&Value>) -> Result<SelectorNormalization, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(SelectorNormalization::default());
    };
    let selector = object(value)?;
    check_members(selector, &["default_variant", "aliases"])?;
    let default_variant = selector
        .get("default_variant")
        .filter(|value| !value.is_null())
        .map(parse_field_key)
        .transpose()?;
    let aliases = selector.get("aliases").map_or(Ok(Vec::new()), |value| {
        let aliases = object(value)?;
        aliases
            .iter()
            .map(|(alias, target)| {
                let alias = FieldKey::new(alias).map_err(|_| AdmissionIssue::InvalidIdentifier)?;
                Ok((alias, parse_field_key(target)?))
            })
            .collect()
    })?;
    Ok(SelectorNormalization {
        default_variant,
        aliases,
    })
}

fn parse_use(
    object: &Map<String, Value>,
    role_fields: &[&str],
) -> Result<UseSiteCore, AdmissionIssue> {
    let mut known = vec![
        "target",
        "null",
        "empty_string",
        "empty_collection",
        "expression",
        "rules",
        "transformers",
        "protection",
        "accepted_domain",
    ];
    known.extend_from_slice(role_fields);
    check_members(object, &known)?;
    let target = DefinitionKey::parse(
        object
            .get("target")
            .ok_or(AdmissionIssue::InvalidDocument)?,
    )?;
    let null = parse_null(object.get("null"))?;
    let empty_string = parse_empty(object.get("empty_string"))?;
    let empty_collection = parse_empty(object.get("empty_collection"))?;
    let expression = object
        .get("expression")
        .map_or(Ok(ExpressionMode::Forbidden), |value| {
            serde_json::from_value(value.clone()).map_err(|_| AdmissionIssue::InvalidDocument)
        })?;
    let protection = parse_protection(object.get("protection"))?;
    let accepted_domain = parse_accepted_domain(object.get("accepted_domain"))?;
    Ok(UseSiteCore {
        target,
        null,
        empty_string,
        empty_collection,
        expression,
        protection,
        accepted_domain,
        rules: parse_rules(object.get("rules"))?,
        transformers: parse_transformers(object.get("transformers"))?,
    })
}

fn parse_protection(value: Option<&Value>) -> Result<ValueProtection, AdmissionIssue> {
    match value.and_then(Value::as_str) {
        None if value.is_none() => Ok(ValueProtection::Public),
        Some("public") => Ok(ValueProtection::Public),
        Some("secret_utf8") => Ok(ValueProtection::SecretUtf8),
        Some("secret_bytes") => Ok(ValueProtection::SecretBytes),
        _ => Err(AdmissionIssue::InvalidDocument),
    }
}

fn parse_accepted_domain(value: Option<&Value>) -> Result<AcceptedDomain, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(AcceptedDomain::Open);
    };
    if value.as_str() == Some("open") {
        return Ok(AcceptedDomain::Open);
    }
    let domain = object(value)?;
    check_members(domain, &["closed"])?;
    let values = domain
        .get("closed")
        .and_then(Value::as_array)
        .ok_or(AdmissionIssue::InvalidDocument)?;
    if values.is_empty() {
        return Err(AdmissionIssue::InvalidBounds);
    }
    let mut values = values
        .iter()
        .map(|value| {
            let value = canonical_literal(value);
            exact_json_bytes(&value).map(|encoded| (encoded, value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    if values.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(AdmissionIssue::InvalidBounds);
    }
    Ok(AcceptedDomain::Closed(
        values.into_iter().map(|(_, value)| value).collect(),
    ))
}

fn canonical_literal(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_literal).collect()),
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_literal(value)))
                    .collect(),
            )
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn parse_null(value: Option<&Value>) -> Result<NullPolicy, AdmissionIssue> {
    parse_rejection_rule(value).map(|policy| match policy {
        RejectionRule::Allow => NullPolicy::Allow,
        RejectionRule::Reject => NullPolicy::Reject,
        RejectionRule::RejectWhen(rule) => NullPolicy::RejectWhen(rule),
    })
}

fn parse_empty(value: Option<&Value>) -> Result<EmptyPolicy, AdmissionIssue> {
    parse_rejection_rule(value).map(|policy| match policy {
        RejectionRule::Allow => EmptyPolicy::Allow,
        RejectionRule::Reject => EmptyPolicy::Reject,
        RejectionRule::RejectWhen(rule) => EmptyPolicy::RejectWhen(rule),
    })
}

fn parse_presence(value: Option<&Value>) -> Result<PresencePolicy, AdmissionIssue> {
    let Some(value) = value else {
        return Ok(PresencePolicy::Optional);
    };
    match value.as_str() {
        Some("optional") => Ok(PresencePolicy::Optional),
        Some("required") => Ok(PresencePolicy::Required),
        Some(_) => Err(AdmissionIssue::InvalidDocument),
        None => {
            let object = object(value)?;
            check_members(object, &["required_when"])?;
            let rule = object
                .get("required_when")
                .ok_or(AdmissionIssue::InvalidDocument)?;
            parse_rule(rule).map(PresencePolicy::RequiredWhen)
        },
    }
}

enum RejectionRule {
    Allow,
    Reject,
    RejectWhen(Rule),
}

fn parse_rejection_rule(value: Option<&Value>) -> Result<RejectionRule, AdmissionIssue> {
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

fn parse_rule(value: &Value) -> Result<Rule, AdmissionIssue> {
    let rule: Rule =
        serde_json::from_value(value.clone()).map_err(|_| AdmissionIssue::InvalidRule)?;
    rule.check_limits()
        .map_err(|_| AdmissionIssue::InvalidRule)?;
    Ok(rule)
}

fn check_duplicate_definitions(definitions: &[Definition]) -> Result<(), AdmissionFailure> {
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

fn normalize_and_check_local(definitions: &mut [Definition]) -> Result<(), AdmissionIssue> {
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

fn check_budgets_and_build_lookup(
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

fn check_dangling(graph: &DraftGraph, lookup: &DefinitionLookup) -> Result<(), AdmissionFailure> {
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

fn check_reachability(graph: &DraftGraph, lookup: &DefinitionLookup) -> Result<(), AdmissionIssue> {
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

fn check_productivity(graph: &DraftGraph, lookup: &DefinitionLookup) -> Result<(), AdmissionIssue> {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StructuralShape {
    can_be_null: bool,
    can_be_empty_collection: bool,
    can_be_other: bool,
}

impl StructuralShape {
    const fn union(self, other: Self) -> Self {
        Self {
            can_be_null: self.can_be_null || other.can_be_null,
            can_be_empty_collection: self.can_be_empty_collection || other.can_be_empty_collection,
            can_be_other: self.can_be_other || other.can_be_other,
        }
    }

    const fn is_productive(self) -> bool {
        self.can_be_null || self.can_be_empty_collection || self.can_be_other
    }
}

fn body_shape(
    body: &Body,
    lookup: &DefinitionLookup,
    shapes: &[StructuralShape],
) -> Result<StructuralShape, AdmissionIssue> {
    match body {
        Body::Null => Ok(StructuralShape {
            can_be_null: true,
            ..StructuralShape::default()
        }),
        Body::Any => Ok(StructuralShape {
            can_be_null: true,
            can_be_empty_collection: true,
            can_be_other: true,
        }),
        Body::Boolean { .. }
        | Body::Integer(_)
        | Body::Number(_)
        | Body::String { .. }
        | Body::Bytes => Ok(StructuralShape {
            can_be_other: true,
            ..StructuralShape::default()
        }),
        Body::Record {
            properties,
            additional_properties,
            ..
        } => {
            let mut required_paths_productive = true;
            let mut has_required_property = false;
            let mut has_productive_property = false;
            for property in properties {
                let property_shape = occurrence_shape(&property.core, lookup, shapes)?;
                has_productive_property |= property_shape.is_productive();
                if presence_is_always_required(&property.presence)?
                    || property.input_default.is_some()
                {
                    has_required_property = true;
                    required_paths_productive &= property_shape.is_productive();
                }
            }
            let dynamic_property_productive = match additional_properties {
                AdditionalProperties::Open => true,
                AdditionalProperties::Closed => false,
                AdditionalProperties::Typed(core) => {
                    occurrence_shape(core, lookup, shapes)?.is_productive()
                },
            };
            Ok(StructuralShape {
                can_be_null: false,
                can_be_empty_collection: required_paths_productive && !has_required_property,
                can_be_other: required_paths_productive
                    && (has_required_property
                        || has_productive_property
                        || dynamic_property_productive),
            })
        },
        Body::Array(array) => Ok(StructuralShape {
            can_be_null: false,
            can_be_empty_collection: array.min_items == 0,
            can_be_other: array.max_items != Some(0)
                && occurrence_shape(&array.element.0, lookup, shapes)?.is_productive(),
        }),
        Body::Union(union) => {
            for variant in &union.variants {
                match &variant.payload {
                    None => {
                        return Ok(StructuralShape {
                            can_be_other: true,
                            ..StructuralShape::default()
                        });
                    },
                    Some(payload)
                        if occurrence_shape(&payload.0, lookup, shapes)?.is_productive() =>
                    {
                        return Ok(StructuralShape {
                            can_be_other: true,
                            ..StructuralShape::default()
                        });
                    },
                    Some(_) => {},
                }
            }
            Ok(StructuralShape::default())
        },
        Body::Alias(alias) => occurrence_shape(&alias.0, lookup, shapes),
    }
}

fn occurrence_shape(
    use_site: &UseSiteCore,
    lookup: &DefinitionLookup,
    shapes: &[StructuralShape],
) -> Result<StructuralShape, AdmissionIssue> {
    let target = lookup
        .get(&use_site.target)
        .map(|index| shapes[index.0])
        .ok_or(AdmissionIssue::DanglingReference)?;
    Ok(StructuralShape {
        can_be_null: null_is_possible(&use_site.null)?,
        can_be_empty_collection: target.can_be_empty_collection
            && empty_collection_is_possible(&use_site.empty_collection)?,
        can_be_other: target.can_be_other,
    })
}

fn presence_is_always_required(policy: &PresencePolicy) -> Result<bool, AdmissionIssue> {
    match policy {
        PresencePolicy::Required => Ok(true),
        PresencePolicy::Optional => Ok(false),
        PresencePolicy::RequiredWhen(rule) => {
            Ok(condition_truth(rule.root())? == ConditionTruth::AlwaysTrue)
        },
    }
}

fn null_is_possible(policy: &NullPolicy) -> Result<bool, AdmissionIssue> {
    match policy {
        NullPolicy::Allow => Ok(true),
        NullPolicy::Reject => Ok(false),
        NullPolicy::RejectWhen(rule) => {
            Ok(condition_truth(rule.root())? != ConditionTruth::AlwaysTrue)
        },
    }
}

fn empty_collection_is_possible(policy: &EmptyPolicy) -> Result<bool, AdmissionIssue> {
    match policy {
        EmptyPolicy::Allow => Ok(true),
        EmptyPolicy::Reject => Ok(false),
        EmptyPolicy::RejectWhen(rule) => {
            Ok(condition_truth(rule.root())? != ConditionTruth::AlwaysTrue)
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionTruth {
    AlwaysTrue,
    AlwaysFalse,
    Unknown,
}

fn condition_truth(rule: RuleRef<'_>) -> Result<ConditionTruth, AdmissionIssue> {
    match rule.view() {
        RuleView::Predicate(_) => Ok(ConditionTruth::Unknown),
        RuleView::All(children) => {
            let mut result = ConditionTruth::AlwaysTrue;
            for child in children {
                result = combine_all(result, condition_truth(child)?);
            }
            Ok(result)
        },
        RuleView::Any(children) => {
            let mut result = ConditionTruth::AlwaysFalse;
            for child in children {
                result = combine_any(result, condition_truth(child)?);
            }
            Ok(result)
        },
        RuleView::Not(inner) => Ok(match condition_truth(inner)? {
            ConditionTruth::AlwaysTrue => ConditionTruth::AlwaysFalse,
            ConditionTruth::AlwaysFalse => ConditionTruth::AlwaysTrue,
            ConditionTruth::Unknown => ConditionTruth::Unknown,
        }),
        RuleView::Described { inner, .. } => condition_truth(inner),
        RuleView::Value(_) | RuleView::Deferred(_) => Err(AdmissionIssue::InapplicableFacet),
        _ => Err(AdmissionIssue::InvalidRule),
    }
}

const fn combine_all(left: ConditionTruth, right: ConditionTruth) -> ConditionTruth {
    match (left, right) {
        (ConditionTruth::AlwaysFalse, _) | (_, ConditionTruth::AlwaysFalse) => {
            ConditionTruth::AlwaysFalse
        },
        (ConditionTruth::AlwaysTrue, ConditionTruth::AlwaysTrue) => ConditionTruth::AlwaysTrue,
        _ => ConditionTruth::Unknown,
    }
}

const fn combine_any(left: ConditionTruth, right: ConditionTruth) -> ConditionTruth {
    match (left, right) {
        (ConditionTruth::AlwaysTrue, _) | (_, ConditionTruth::AlwaysTrue) => {
            ConditionTruth::AlwaysTrue
        },
        (ConditionTruth::AlwaysFalse, ConditionTruth::AlwaysFalse) => ConditionTruth::AlwaysFalse,
        _ => ConditionTruth::Unknown,
    }
}

fn check_facet_applicability(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<(), AdmissionIssue> {
    check_use_facets(graph, lookup, &graph.root.0)?;
    for definition in &graph.definitions {
        check_intrinsic_rules(&definition.body)?;
        if let Body::Record { properties, .. } = &definition.body {
            for property in properties {
                if let PresencePolicy::RequiredWhen(rule) = &property.presence {
                    check_condition(rule)?;
                }
                if let Some(default) = &property.input_default {
                    check_input_default(graph, lookup, property, default)?;
                }
            }
        }
        match &definition.body {
            Body::Integer(number) => {
                check_numeric(number, true)?;
            },
            Body::Number(number) => {
                check_numeric(number, false)?;
            },
            Body::Array(array)
                if array
                    .max_items
                    .is_some_and(|maximum| array.min_items > maximum) =>
            {
                return Err(AdmissionIssue::InvalidBounds);
            },
            Body::Union(union) => check_selector(union)?,
            _ => {},
        }
        for edge in definition.edges()? {
            let core = definition
                .use_for_edge(&edge)
                .ok_or(AdmissionIssue::InvalidDocument)?;
            check_use_facets(graph, lookup, core)?;
        }
    }
    Ok(())
}

fn resolved_body<'a>(
    graph: &'a DraftGraph,
    lookup: &DefinitionLookup,
    target: &DefinitionKey,
) -> Result<&'a Body, AdmissionIssue> {
    let mut current = *lookup
        .get(target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    for _ in 0..graph.definitions.len() {
        let body = &graph.definitions[current.0].body;
        match body {
            Body::Alias(alias) => {
                current = *lookup
                    .get(&alias.0.target)
                    .ok_or(AdmissionIssue::DanglingReference)?;
            },
            _ => return Ok(body),
        }
    }
    Err(AdmissionIssue::NonproductiveDefinition)
}

fn check_use_facets(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    core: &UseSiteCore,
) -> Result<(), AdmissionIssue> {
    if let NullPolicy::RejectWhen(rule) = &core.null {
        check_condition(rule)?;
    }
    if let EmptyPolicy::RejectWhen(rule) = &core.empty_string {
        check_condition(rule)?;
    }
    if let EmptyPolicy::RejectWhen(rule) = &core.empty_collection {
        check_condition(rule)?;
    }
    let needs_target = !matches!(core.empty_string, EmptyPolicy::Allow)
        || !matches!(core.empty_collection, EmptyPolicy::Allow)
        || !core.transformers.is_empty()
        || !core.rules.is_empty()
        || core.protection != ValueProtection::Public
        || matches!(core.accepted_domain, AcceptedDomain::Closed(_));
    if !needs_target {
        return Ok(());
    }
    let target = resolved_body(graph, lookup, &core.target)?;
    match core.protection {
        ValueProtection::Public => {},
        ValueProtection::SecretUtf8 if matches!(target, Body::String { .. }) => {},
        ValueProtection::SecretBytes if matches!(target, Body::Bytes) => {},
        ValueProtection::SecretUtf8 | ValueProtection::SecretBytes => {
            return Err(AdmissionIssue::InapplicableFacet);
        },
    }
    if let AcceptedDomain::Closed(values) = &core.accepted_domain {
        if occurrence_contains_protected(graph, lookup, core)? || matches!(target, Body::Bytes) {
            return Err(AdmissionIssue::InapplicableFacet);
        }
        for value in values {
            if !literal_matches_use(graph, lookup, core, value, LiteralPurpose::AcceptedDomain)? {
                return Err(AdmissionIssue::InapplicableFacet);
            }
        }
    }
    if matches!(
        core.empty_string,
        EmptyPolicy::Reject | EmptyPolicy::RejectWhen(_)
    ) && !matches!(target, Body::String { .. } | Body::Bytes)
    {
        return Err(AdmissionIssue::InapplicableFacet);
    }
    if matches!(
        core.empty_collection,
        EmptyPolicy::Reject | EmptyPolicy::RejectWhen(_)
    ) && !matches!(target, Body::Record { .. } | Body::Array(_))
    {
        return Err(AdmissionIssue::InapplicableFacet);
    }
    if !core.transformers.is_empty() && !matches!(target, Body::String { .. }) {
        return Err(AdmissionIssue::InapplicableFacet);
    }
    check_rules(&core.rules, target, false)?;
    Ok(())
}

fn check_input_default(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    property: &PropertyUse,
    default: &Value,
) -> Result<(), AdmissionIssue> {
    if property.core.expression == ExpressionMode::Required
        || occurrence_contains_protected(graph, lookup, &property.core)?
        || !literal_matches_use(
            graph,
            lookup,
            &property.core,
            default,
            LiteralPurpose::InputDefault,
        )?
    {
        return Err(AdmissionIssue::InvalidDefault);
    }
    Ok(())
}

fn occurrence_contains_protected(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    core: &UseSiteCore,
) -> Result<bool, AdmissionIssue> {
    if core.protection != ValueProtection::Public {
        return Ok(true);
    }
    let root = *lookup
        .get(&core.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut visited = vec![false; graph.definitions.len()];
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if visited[index.0] {
            continue;
        }
        visited[index.0] = true;
        for edge in graph.definitions[index.0].edges()? {
            let edge_core = graph.definitions[index.0]
                .use_for_edge(&edge)
                .ok_or(AdmissionIssue::InvalidDocument)?;
            if edge_core.protection != ValueProtection::Public {
                return Ok(true);
            }
            pending.push(
                *lookup
                    .get(&edge.target)
                    .ok_or(AdmissionIssue::DanglingReference)?,
            );
        }
    }
    Ok(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiteralPurpose {
    AcceptedDomain,
    InputDefault,
}

fn literal_matches_use(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    core: &UseSiteCore,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let mut current_value = literal_with_use_transformers(core, value, purpose);
    if !literal_matches_use_facets(core, &current_value, purpose)? {
        return Ok(false);
    }
    let mut current = *lookup
        .get(&core.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut visited = vec![false; graph.definitions.len()];
    loop {
        if visited[current.0] {
            return Err(AdmissionIssue::NonproductiveDefinition);
        }
        visited[current.0] = true;
        let body = &graph.definitions[current.0].body;
        if let Body::Alias(alias) = body {
            current_value = literal_with_use_transformers(&alias.0, &current_value, purpose);
            if !literal_matches_use_facets(&alias.0, &current_value, purpose)? {
                return Ok(false);
            }
            current = *lookup
                .get(&alias.0.target)
                .ok_or(AdmissionIssue::DanglingReference)?;
            continue;
        }
        return literal_matches_body(graph, lookup, body, &current_value, purpose);
    }
}

fn literal_with_use_transformers(
    core: &UseSiteCore,
    value: &Value,
    purpose: LiteralPurpose,
) -> Value {
    if purpose != LiteralPurpose::InputDefault {
        return value.clone();
    }
    core.transformers
        .iter()
        .fold(value.clone(), |current, transformer| {
            transformer.apply(&current)
        })
}

fn literal_matches_use_facets(
    core: &UseSiteCore,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    Ok(
        !(purpose == LiteralPurpose::InputDefault && core.expression == ExpressionMode::Required)
            && literal_matches_occurrence_policies(core, value)?
            && !matches!(&core.accepted_domain, AcceptedDomain::Closed(values) if !values.contains(value))
            && rules_accept(&core.rules, value)?,
    )
}

fn literal_matches_occurrence_policies(
    core: &UseSiteCore,
    value: &Value,
) -> Result<bool, AdmissionIssue> {
    if value.is_null() {
        return Ok(matches!(core.null, NullPolicy::Allow));
    }
    if value.as_str() == Some("") && !matches!(core.empty_string, EmptyPolicy::Allow) {
        return Ok(false);
    }
    if matches!(value, Value::Array(values) if values.is_empty())
        || matches!(value, Value::Object(values) if values.is_empty())
    {
        return Ok(matches!(core.empty_collection, EmptyPolicy::Allow));
    }
    Ok(true)
}

fn literal_matches_body(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    body: &Body,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let shape_matches = match body {
        Body::Any => true,
        Body::Null => value.is_null(),
        Body::Boolean { .. } => value.is_boolean(),
        Body::Integer(number) => {
            value
                .as_number()
                .is_some_and(|value| value.is_i64() || value.is_u64())
                && numeric_value_in_bounds(value, number)?
        },
        Body::Number(number) => value.is_number() && numeric_value_in_bounds(value, number)?,
        Body::String { .. } => value.is_string(),
        Body::Bytes => value.as_str().is_some_and(is_canonical_base64),
        Body::Record {
            properties,
            additional_properties,
            ..
        } => record_literal_matches(
            graph,
            lookup,
            properties,
            additional_properties,
            value,
            purpose,
        )?,
        Body::Array(array) => array_literal_matches(graph, lookup, array, value, purpose)?,
        Body::Union(union) => union_literal_matches(graph, lookup, union, value, purpose)?,
        Body::Alias(_) => return Err(AdmissionIssue::InvalidDocument),
    };
    Ok(shape_matches && rules_accept(intrinsic_rules(body), value)?)
}

fn intrinsic_rules(body: &Body) -> &[Rule] {
    match body {
        Body::Boolean { intrinsic_rules } | Body::String { intrinsic_rules } => intrinsic_rules,
        Body::Integer(number) | Body::Number(number) => &number.intrinsic_rules,
        Body::Record {
            intrinsic_rules, ..
        } => intrinsic_rules,
        Body::Array(array) => &array.intrinsic_rules,
        Body::Any | Body::Null | Body::Bytes | Body::Union(_) | Body::Alias(_) => &[],
    }
}

fn numeric_value_in_bounds(value: &Value, body: &NumericBody) -> Result<bool, AdmissionIssue> {
    let Some(value) = value.as_number() else {
        return Ok(false);
    };
    if let Some(minimum) = &body.minimum
        && compare_numbers(value, minimum)? == Ordering::Less
    {
        return Ok(false);
    }
    if let Some(maximum) = &body.maximum
        && compare_numbers(value, maximum)? == Ordering::Greater
    {
        return Ok(false);
    }
    Ok(true)
}

fn record_literal_matches(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    properties: &[PropertyUse],
    additional_properties: &AdditionalProperties,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let Some(object) = value.as_object() else {
        return Ok(false);
    };
    for property in properties {
        match object.get(property.key.as_str()) {
            Some(value) if !literal_matches_use(graph, lookup, &property.core, value, purpose)? => {
                return Ok(false);
            },
            None if presence_is_always_required(&property.presence)?
                && property.input_default.is_none() =>
            {
                return Ok(false);
            },
            Some(_) | None => {},
        }
    }
    for (key, value) in object {
        if properties
            .iter()
            .any(|property| property.key.as_str() == key)
        {
            continue;
        }
        match additional_properties {
            AdditionalProperties::Open => {},
            AdditionalProperties::Closed => return Ok(false),
            AdditionalProperties::Typed(core)
                if literal_matches_use(graph, lookup, core, value, purpose)? => {},
            AdditionalProperties::Typed(_) => return Ok(false),
        }
    }
    Ok(true)
}

fn array_literal_matches(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    array: &ArrayBody,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    let Some(values) = value.as_array() else {
        return Ok(false);
    };
    let len = u64::try_from(values.len()).map_err(|_| AdmissionIssue::IndexOverflow)?;
    if len < u64::from(array.min_items)
        || array
            .max_items
            .is_some_and(|maximum| len > u64::from(maximum))
        || array.unique
            && values
                .iter()
                .enumerate()
                .any(|(index, value)| values[..index].contains(value))
    {
        return Ok(false);
    }
    for value in values {
        if !literal_matches_use(graph, lookup, &array.element.0, value, purpose)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn union_literal_matches(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
    union: &UnionBody,
    value: &Value,
    purpose: LiteralPurpose,
) -> Result<bool, AdmissionIssue> {
    match &union.tagging {
        SerdeTagging::External => match value {
            Value::String(selector) => Ok(union
                .variants
                .iter()
                .any(|variant| variant.key.as_str() == selector && variant.payload.is_none())),
            Value::Object(object) if object.len() == 1 => {
                let Some((selector, payload_value)) = object.iter().next() else {
                    return Ok(false);
                };
                let Some(payload) = union
                    .variants
                    .iter()
                    .find(|variant| variant.key.as_str() == selector)
                    .and_then(|variant| variant.payload.as_ref())
                else {
                    return Ok(false);
                };
                literal_matches_use(graph, lookup, &payload.0, payload_value, purpose)
            },
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => Ok(false),
        },
        SerdeTagging::Adjacent { tag, content } => {
            let Some(object) = value.as_object() else {
                return Ok(false);
            };
            let Some(selector) = object.get(tag).and_then(Value::as_str) else {
                return Ok(false);
            };
            let Some(variant) = union
                .variants
                .iter()
                .find(|variant| variant.key.as_str() == selector)
            else {
                return Ok(false);
            };
            match &variant.payload {
                Some(payload) => {
                    if object.len() != 2 {
                        return Ok(false);
                    }
                    let Some(value) = object.get(content) else {
                        return Ok(false);
                    };
                    literal_matches_use(graph, lookup, &payload.0, value, purpose)
                },
                None => Ok(object.len() == 1 && !object.contains_key(content)),
            }
        },
    }
}

fn rules_accept(rules: &[Rule], value: &Value) -> Result<bool, AdmissionIssue> {
    for rule in rules {
        let mut pending = vec![rule.root()];
        while let Some(current) = pending.pop() {
            match current.view() {
                RuleView::Value(_) => {},
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Predicate(_) | RuleView::Deferred(_) => return Ok(false),
                _ => return Err(AdmissionIssue::InvalidRule),
            }
        }
        if rule
            .validate(
                value,
                None,
                ExecutionMode::StaticOnly,
                DiagnosticDisclosure::OmitValue,
            )
            .and_then(nebula_validator::EvaluationOutcome::require_satisfied)
            .is_err()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn is_canonical_base64(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    if bytes.is_empty() {
        return true;
    }
    let padding = usize::from(bytes.ends_with(b"=")) + usize::from(bytes.ends_with(b"=="));
    let payload_len = bytes.len() - padding;
    if bytes[..payload_len]
        .iter()
        .any(|byte| base64_value(*byte).is_none())
        || bytes[payload_len..].iter().any(|byte| *byte != b'=')
    {
        return false;
    }
    match padding {
        0 => true,
        1 => base64_value(bytes[payload_len - 1]).is_some_and(|value| value.trailing_zeros() >= 2),
        2 => base64_value(bytes[payload_len - 1]).is_some_and(|value| value.trailing_zeros() >= 4),
        _ => false,
    }
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn check_condition(rule: &Rule) -> Result<(), AdmissionIssue> {
    let mut pending = vec![rule.root()];
    while let Some(current) = pending.pop() {
        match current.view() {
            RuleView::Predicate(_) => {},
            RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
            RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
            RuleView::Value(_) | RuleView::Deferred(_) => {
                return Err(AdmissionIssue::InapplicableFacet);
            },
            _ => return Err(AdmissionIssue::InapplicableFacet),
        }
    }
    Ok(())
}

fn check_intrinsic_rules(body: &Body) -> Result<(), AdmissionIssue> {
    let rules = match body {
        Body::Boolean { intrinsic_rules } | Body::String { intrinsic_rules } => intrinsic_rules,
        Body::Integer(number) | Body::Number(number) => &number.intrinsic_rules,
        Body::Record {
            intrinsic_rules, ..
        } => intrinsic_rules,
        Body::Array(array) => &array.intrinsic_rules,
        Body::Any | Body::Null | Body::Bytes | Body::Union(_) | Body::Alias(_) => return Ok(()),
    };
    check_rules(rules, body, true)
}

fn check_rules(rules: &[Rule], target: &Body, context_free: bool) -> Result<(), AdmissionIssue> {
    for rule in rules {
        let mut pending = vec![rule.root()];
        while let Some(current) = pending.pop() {
            match current.view() {
                RuleView::Value(value) if value_rule_applies(value, target) => {},
                RuleView::Value(_) => return Err(AdmissionIssue::InapplicableFacet),
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Predicate(_) | RuleView::Deferred(_) if context_free => {
                    return Err(AdmissionIssue::InapplicableFacet);
                },
                RuleView::Predicate(_) | RuleView::Deferred(_) => {},
                _ => return Err(AdmissionIssue::InapplicableFacet),
            }
        }
    }
    Ok(())
}

fn value_rule_applies(rule: &ValueRule, target: &Body) -> bool {
    match (target, rule) {
        (
            Body::String { .. },
            ValueRule::MinLength(_)
            | ValueRule::MaxLength(_)
            | ValueRule::Pattern(_)
            | ValueRule::Email
            | ValueRule::Url,
        ) => true,
        (
            Body::Integer(_) | Body::Number(_),
            ValueRule::Min(_)
            | ValueRule::Max(_)
            | ValueRule::GreaterThan(_)
            | ValueRule::LessThan(_),
        ) => true,
        (Body::Array(_), ValueRule::MinItems(_) | ValueRule::MaxItems(_)) => true,
        (body, ValueRule::OneOf(values)) => {
            values.iter().all(|value| value_matches_body(value, body))
        },
        _ => false,
    }
}

fn value_matches_body(value: &Value, body: &Body) -> bool {
    match body {
        Body::Any => true,
        Body::Null => value.is_null(),
        Body::Boolean { .. } => value.is_boolean(),
        Body::Integer(_) => value
            .as_number()
            .is_some_and(|number| number.is_i64() || number.is_u64()),
        Body::Number(_) => value.is_number(),
        Body::String { .. } => value.is_string(),
        Body::Bytes => value.as_str().is_some_and(is_canonical_base64),
        Body::Record { .. } => value.is_object(),
        Body::Array(_) => value.is_array(),
        Body::Union(_) | Body::Alias(_) => true,
    }
}

fn check_numeric(number: &NumericBody, integer: bool) -> Result<(), AdmissionIssue> {
    if integer
        && number
            .minimum
            .iter()
            .chain(number.maximum.iter())
            .any(|value| !value.is_i64() && !value.is_u64())
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    if let (Some(minimum), Some(maximum)) = (&number.minimum, &number.maximum)
        && compare_numbers(minimum, maximum)?.is_gt()
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    Ok(())
}

fn check_selector(union: &UnionBody) -> Result<(), AdmissionIssue> {
    let has_variant = |key: &FieldKey| union.variants.iter().any(|variant| &variant.key == key);
    if union
        .selector
        .default_variant
        .as_ref()
        .is_some_and(|key| !has_variant(key))
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    if union
        .selector
        .aliases
        .iter()
        .any(|(_, target)| !has_variant(target))
    {
        return Err(AdmissionIssue::InvalidBounds);
    }
    Ok(())
}

fn canonical_numbering(
    graph: &DraftGraph,
    lookup: &DefinitionLookup,
) -> Result<Vec<u32>, AdmissionIssue> {
    let root = *lookup
        .get(&graph.root.0.target)
        .ok_or(AdmissionIssue::DanglingReference)?;
    let mut numbers = vec![u32::MAX; graph.definitions.len()];
    numbers[root.0] = 0;
    let mut next = 1u32;
    let mut pending = VecDeque::from([root]);
    while let Some(index) = pending.pop_front() {
        let mut edges = graph.definitions[index.0].edges()?;
        edges.sort_unstable_by(Edge::compare);
        for edge in edges {
            let target = *lookup
                .get(&edge.target)
                .ok_or(AdmissionIssue::DanglingReference)?;
            if numbers[target.0] == u32::MAX {
                numbers[target.0] = next;
                next = next.checked_add(1).ok_or(AdmissionIssue::IndexOverflow)?;
                pending.push_back(target);
            }
        }
    }
    Ok(numbers)
}

fn address_exists(
    graph: &DraftGraph,
    definition: DefinitionIndex,
    use_site: &DeclarationUse,
) -> bool {
    match use_site {
        DeclarationUse::Root => graph.root.0.target == graph.definitions[definition.0].key,
        DeclarationUse::Alias => matches!(graph.definitions[definition.0].body, Body::Alias(_)),
        DeclarationUse::Property(key) => {
            matches!(&graph.definitions[definition.0].body, Body::Record { properties, .. } if properties.iter().any(|property| property.key.as_str() == key.as_str()))
        },
        DeclarationUse::AdditionalProperty => {
            matches!(
                &graph.definitions[definition.0].body,
                Body::Record {
                    additional_properties: AdditionalProperties::Typed(_),
                    ..
                }
            )
        },
        DeclarationUse::Element => matches!(graph.definitions[definition.0].body, Body::Array(_)),
        DeclarationUse::Variant(key) => {
            matches!(&graph.definitions[definition.0].body, Body::Union(union) if union.variants.iter().any(|variant| variant.key.as_str() == key.as_str()))
        },
        DeclarationUse::VariantPayload(key) => {
            matches!(&graph.definitions[definition.0].body, Body::Union(union) if union.variants.iter().any(|variant| variant.key.as_str() == key.as_str() && variant.payload.is_some()))
        },
    }
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
