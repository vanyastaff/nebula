//! Wire-document parsing for schema graph admission.

use serde_json::{Map, Value};

use crate::{ExpressionMode, FieldKey, SerdeTagging};

use super::super::{
    MAX_GRAPH_DEFINITIONS, SCHEMA_GRAPH_WIRE_VERSION,
    canonical::exact_json_bytes,
    model::{
        AcceptedDomain, AdditionalProperties, AdmissionIssue, AliasUse, ArrayBody, Body,
        Definition, DefinitionKey, DirectionalAliases, DraftGraph, ElementUse, EmptyPolicy,
        NullPolicy, NumericBody, PayloadUse, PresencePolicy, PropertyUse, RootUse,
        SelectorNormalization, UnionBody, UseSiteCore, ValueProtection, VariantUse, check_members,
        object, parse_field_key, parse_rules, parse_transformers,
    },
};

use super::{RejectionRule, parse_rejection_rule, parse_rule};

pub(super) fn parse_document(raw: &Value) -> Result<DraftGraph, AdmissionIssue> {
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
