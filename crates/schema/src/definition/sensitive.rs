//! Private disclosure boundaries for native schema-graph input.

use std::fmt;

use nebula_validator::PredicateContext;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use zeroize::Zeroize;

use super::{
    AdmittedSchemaGraph,
    model::{AdditionalProperties, Body, UnionBody, UseSiteCore, ValueProtection},
};
use crate::{ResolvedValue, SerdeTagging, ValidationError, ValuePath, ValueTree};

pub(super) fn decode_wire<T: DeserializeOwned>(
    values: &ResolvedValue,
    expose: bool,
) -> Result<T, ValidationError> {
    crate::validated::decode_graph_wire(values, expose)
}

/// Scoped plaintext used only while the owning validator checks literal data.
/// All allocated text is erased before releasing the temporary JSON tree.
pub(super) struct SensitiveJson(Value);

impl SensitiveJson {
    pub(super) fn new(values: &ResolvedValue) -> Result<Self, ValidationError> {
        decode_wire(values, true).map(Self)
    }

    pub(super) fn value(&self) -> &Value {
        &self.0
    }
}

impl fmt::Debug for SensitiveJson {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveJson(<redacted>)")
    }
}

impl Drop for SensitiveJson {
    fn drop(&mut self) {
        erase_text(&mut self.0);
    }
}

fn erase_text(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(erase_text),
        Value::Object(values) => {
            for (mut key, mut value) in std::mem::take(values) {
                key.zeroize();
                erase_text(&mut value);
            }
        },
        _ => {},
    }
}

/// Project only public predicate data; sources and pending data never enter it.
#[tracing::instrument(level = "debug", skip_all)]
pub(super) fn predicate_context<E>(
    graph: &AdmittedSchemaGraph,
    values: &ValueTree<E>,
    pending: &[ValuePath],
) -> Result<PredicateContext, ValidationError> {
    values.check_depth(&ValuePath::root(), 0)?;
    let projection = Projection {
        graph,
        protected: protected_definitions(graph),
        pending,
    };
    let root = projection
        .project(&graph.0.graph.root.0, values, &ValuePath::root())
        .unwrap_or(Value::Null);
    Ok(PredicateContext::from_json(root).with_pending_paths(pending.iter().cloned()))
}

/// Fixed-point reachability also handles recursive declarations without recursion.
fn protected_definitions(graph: &AdmittedSchemaGraph) -> Vec<bool> {
    let definitions = &graph.0.graph.definitions;
    let mut protected = vec![false; definitions.len()];
    loop {
        let mut changed = false;
        for (index, definition) in definitions.iter().enumerate() {
            if protected[index] {
                continue;
            }
            let contains = |core: &UseSiteCore| {
                core.protection != ValueProtection::Public
                    || graph
                        .0
                        .lookup
                        .get(&core.target)
                        .is_some_and(|target| protected[target.0])
            };
            let secret = match &definition.body {
                Body::Alias(alias) => contains(&alias.0),
                Body::Array(array) => contains(&array.element.0),
                Body::Record {
                    properties,
                    additional_properties,
                    ..
                } => {
                    properties.iter().any(|property| contains(&property.core))
                        || matches!(additional_properties, AdditionalProperties::Typed(core) if contains(core))
                },
                Body::Union(union) => union.variants.iter().any(|variant| {
                    variant
                        .payload
                        .as_ref()
                        .is_some_and(|payload| contains(&payload.0))
                }),
                _ => false,
            };
            if secret {
                protected[index] = true;
                changed = true;
            }
        }
        if !changed {
            return protected;
        }
    }
}

struct Projection<'a> {
    graph: &'a AdmittedSchemaGraph,
    protected: Vec<bool>,
    pending: &'a [ValuePath],
}

/// Determine whether literal authoring data supplies material at a protected use.
/// Missing and null optional branches do not populate a secret. Malformed
/// containers with protected descendants fail closed before typed extraction.
pub(super) fn data_populates_protected<'a>(
    graph: &'a AdmittedSchemaGraph,
    core: &'a UseSiteCore,
    value: &Value,
) -> bool {
    let protected = protected_definitions(graph);
    let mut pending = vec![(core, value)];
    while let Some((mut core, value)) = pending.pop() {
        if value.is_null() {
            continue;
        }
        let (body, secret_bearing) = loop {
            if core.protection != ValueProtection::Public {
                return true;
            }
            let Some(index) = graph.0.lookup.get(&core.target) else {
                return true;
            };
            let body = &graph.0.graph.definitions[index.0].body;
            if let Body::Alias(alias) = body {
                core = &alias.0;
            } else {
                break (body, protected[index.0]);
            }
        };
        match (body, value) {
            (
                Body::Record {
                    properties,
                    additional_properties,
                    ..
                },
                Value::Object(values),
            ) => {
                for property in properties {
                    if let Some(value) = values.get(property.key.as_str()) {
                        pending.push((&property.core, value));
                    }
                    // Authoring admission must reject secret constants even in
                    // losing spellings; preparation precedence is not permission.
                    for alias in &property.aliases.read {
                        if let Some(value) = values.get(alias.as_str()) {
                            pending.push((&property.core, value));
                        }
                    }
                }
                if let AdditionalProperties::Typed(core) = additional_properties {
                    for (key, value) in values {
                        if !properties.iter().any(|property| {
                            property.key.as_str() == key
                                || property
                                    .aliases
                                    .read
                                    .iter()
                                    .any(|alias| alias.as_str() == key)
                        }) {
                            pending.push((core, value));
                        }
                    }
                }
            },
            (Body::Array(array), Value::Array(values)) => {
                pending.extend(values.iter().map(|value| (&array.element.0, value)));
            },
            (Body::Union(union), _) => {
                let selected = match (&union.tagging, value) {
                    (SerdeTagging::External, Value::String(key)) => union
                        .variants
                        .iter()
                        .find(|variant| variant.key.as_str() == key && variant.payload.is_none())
                        .map(|variant| (variant, None)),
                    (SerdeTagging::External, Value::Object(values)) if values.len() == 1 => {
                        values.iter().next().and_then(|(key, value)| {
                            union
                                .variants
                                .iter()
                                .find(|variant| variant.key.as_str() == key)
                                .map(|variant| (variant, Some(value)))
                        })
                    },
                    (SerdeTagging::Adjacent { tag, content }, Value::Object(values)) => values
                        .get(tag)
                        .and_then(Value::as_str)
                        .and_then(|key| {
                            union
                                .variants
                                .iter()
                                .find(|variant| variant.key.as_str() == key)
                        })
                        .map(|variant| (variant, values.get(content))),
                    _ => None,
                };
                if let Some((variant, value)) = selected {
                    if let Some(payload) = &variant.payload
                        && let Some(value) = value
                    {
                        pending.push((&payload.0, value));
                    }
                } else if secret_bearing {
                    return true;
                }
            },
            (Body::Record { .. } | Body::Array(_), _) if secret_bearing => return true,
            _ => {},
        }
    }
    false
}

impl Projection<'_> {
    fn project<'a, E>(
        &'a self,
        mut core: &'a UseSiteCore,
        value: &ValueTree<E>,
        path: &ValuePath,
    ) -> Option<Value> {
        if self.pending.iter().any(|pending| path.starts_with(pending))
            || matches!(value, ValueTree::Expression(_) | ValueTree::Secret(_))
        {
            return None;
        }
        // Admission forbids nonproductive alias cycles. Resolve aliases without
        // consuming call-stack depth before descending the bounded value tree.
        let (body, secret_bearing) = loop {
            if core.protection != ValueProtection::Public {
                return None;
            }
            let index = self.graph.0.lookup.get(&core.target)?;
            let body = &self.graph.0.graph.definitions[index.0].body;
            if let Body::Alias(alias) = body {
                core = &alias.0;
            } else {
                break (body, self.protected[index.0]);
            }
        };
        match (body, value) {
            (
                Body::Record {
                    properties,
                    additional_properties,
                    ..
                },
                ValueTree::Object(values),
            ) => {
                let mut output = Map::new();
                for property in properties {
                    let selected = values.get(property.key.as_str()).or_else(|| {
                        property
                            .aliases
                            .read
                            .iter()
                            .find_map(|alias| values.get(alias.as_str()))
                    });
                    if let Some(value) = selected.and_then(|value| {
                        self.project(&property.core, value, &path.push(property.key.as_str()))
                    }) {
                        output.insert(property.key.as_str().to_owned(), value);
                    }
                }
                for (key, value) in values {
                    if properties.iter().any(|property| {
                        property.key.as_str() == key
                            || property
                                .aliases
                                .read
                                .iter()
                                .any(|alias| alias.as_str() == key)
                            || property
                                .aliases
                                .write
                                .as_ref()
                                .is_some_and(|alias| alias.as_str() == key)
                    }) {
                        continue;
                    }
                    let projected = match additional_properties {
                        AdditionalProperties::Typed(core) => {
                            self.project(core, value, &path.push(key))
                        },
                        AdditionalProperties::Open if !secret_bearing => {
                            self.public_value(value, &path.push(key))
                        },
                        _ => None,
                    };
                    if let Some(value) = projected {
                        output.insert(key.clone(), value);
                    }
                }
                Some(Value::Object(output))
            },
            (Body::Array(array), ValueTree::List(values)) => Some(Value::Array(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.project(&array.element.0, value, &path.push(index.to_string()))
                            .unwrap_or(Value::Null)
                    })
                    .collect(),
            )),
            (Body::Union(union), _) => self.union(union, value, path),
            (Body::Record { .. } | Body::Array(_), _) if secret_bearing => None,
            _ => self.public_value(value, path),
        }
    }

    fn public_value<E>(&self, value: &ValueTree<E>, path: &ValuePath) -> Option<Value> {
        if self.pending.iter().any(|pending| path.starts_with(pending)) {
            return None;
        }
        match value {
            ValueTree::Literal(value) => Some(value.as_json().clone()),
            ValueTree::Object(values) => Some(Value::Object(
                values
                    .iter()
                    .filter_map(|(key, value)| {
                        self.public_value(value, &path.push(key))
                            .map(|value| (key.clone(), value))
                    })
                    .collect(),
            )),
            ValueTree::List(values) => Some(Value::Array(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.public_value(value, &path.push(index.to_string()))
                            .unwrap_or(Value::Null)
                    })
                    .collect(),
            )),
            ValueTree::Secret(_) | ValueTree::Expression(_) => None,
        }
    }

    fn union<E>(&self, union: &UnionBody, value: &ValueTree<E>, path: &ValuePath) -> Option<Value> {
        match (&union.tagging, value) {
            (SerdeTagging::External, ValueTree::Literal(value)) => {
                let selected = value.as_json().as_str()?;
                union.variants.iter().find(|variant| {
                    variant.key.as_str() == selected && variant.payload.is_none()
                })?;
                Some(Value::String(selected.to_owned()))
            },
            (SerdeTagging::External, ValueTree::Object(values)) if values.len() == 1 => {
                let (key, value) = values.first()?;
                let variant = union
                    .variants
                    .iter()
                    .find(|variant| variant.key.as_str() == key)?;
                let payload = variant.payload.as_ref()?;
                let value = self.project(&payload.0, value, &path.push(key))?;
                Some(Value::Object(Map::from_iter([(key.clone(), value)])))
            },
            (SerdeTagging::Adjacent { tag, content }, ValueTree::Object(values)) => {
                let selector = values.get(tag)?;
                let selected = selector.as_str()?;
                let variant = union
                    .variants
                    .iter()
                    .find(|variant| variant.key.as_str() == selected)?;
                let mut output = Map::new();
                if let Some(value) = self.public_value(selector, &path.push(tag)) {
                    output.insert(tag.clone(), value);
                }
                if let Some(payload) = &variant.payload
                    && let Some(value) = values.get(content)
                    && let Some(value) = self.project(&payload.0, value, &path.push(content))
                {
                    output.insert(content.clone(), value);
                }
                Some(Value::Object(output))
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Deserializer};
    use serde_json::json;

    use super::*;
    use crate::{AuthoredValue, SchemaGraphDocument, SecretValue};

    const CANARY: &str = "sensitive-input-canary-9f3";

    fn graph() -> AdmittedSchemaGraph {
        let document: SchemaGraphDocument = serde_json::from_value(json!({
            "version": 3,
            "root": {"target":"root"},
            "definitions": [
                {"key":"root", "body":{"kind":"record", "properties":[
                    {"key":"settings", "target":"settings"},
                    {"key":"items", "target":"items"}
                ]}},
                {"key":"settings", "body":{"kind":"record", "properties":[
                    {"key":"password", "target":"text", "protection":"secret_utf8", "aliases":{"read":["old_password","older_password"]}},
                    {"key":"region", "target":"text"}
                ]}},
                {"key":"items", "body":{"kind":"array", "element":{"target":"text"}}},
                {"key":"text", "body":{"kind":"string"}}
            ]
        })).unwrap();
        document.admit().unwrap()
    }

    #[test]
    fn declaration_scrub_removes_aliases_wrong_shapes_and_pending_values() {
        let graph = graph();
        for settings in [
            json!({"password":CANARY,"old_password":{"nested":CANARY},"region":"eu","extra":CANARY}),
            json!({"old_password":[CANARY],"region":"eu"}),
            json!(CANARY),
            json!([CANARY]),
        ] {
            let values =
                AuthoredValue::from_data(json!({"settings":settings,"items":["visible",CANARY]}))
                    .unwrap();
            let pending = ValuePath::parse("/items/1").unwrap();
            let context =
                predicate_context(&graph, &values, std::slice::from_ref(&pending)).unwrap();
            let root = context.get(&ValuePath::root()).unwrap();
            assert!(!root.to_string().contains(CANARY));
            assert!(context.is_pending(&pending));
            assert_eq!(
                context.get(&ValuePath::parse("/items").unwrap()),
                Some(&json!(["visible", null]))
            );
            if settings.is_object() {
                assert_eq!(
                    context.get(&ValuePath::parse("/settings").unwrap()),
                    Some(&json!({"region":"eu"}))
                );
            }
        }
    }

    #[test]
    fn literal_protection_scan_distinguishes_absent_null_and_populated_data() {
        let graph = graph();
        let root = &graph.0.graph.root.0;
        for value in [
            json!({}),
            json!({"settings":null}),
            json!({"settings":{"region":"eu"}}),
            json!({"settings":{"password":null}}),
        ] {
            assert!(!data_populates_protected(&graph, root, &value));
        }
        for value in [
            json!({"settings":{"password":CANARY}}),
            json!({"settings":{"old_password":CANARY}}),
            json!({"settings":{"password":null,"old_password":CANARY}}),
            json!({"settings":{"old_password":null,"older_password":CANARY}}),
            json!({"settings":[CANARY]}),
            json!({"settings":CANARY}),
        ] {
            assert!(data_populates_protected(&graph, root, &value));
        }
    }

    #[derive(Debug)]
    struct RejectedSecret;

    impl<'de> Deserialize<'de> for RejectedSecret {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let value = String::deserialize(deserializer)?;
            Err(serde::de::Error::custom(value))
        }
    }

    #[test]
    fn protected_decoder_erases_custom_deserializer_diagnostics() {
        let values = ResolvedValue::Secret(SecretValue::string(CANARY.to_owned()));
        let error = decode_wire::<RejectedSecret>(&values, true).unwrap_err();
        assert!(!format!("{error:?} {error}").contains(CANARY));
        let mut source = std::error::Error::source(&error);
        while let Some(error) = source {
            assert!(!format!("{error:?} {error}").contains(CANARY));
            source = error.source();
        }
        assert!(decode_wire::<String>(&values, false).is_err());
        assert_eq!(decode_wire::<String>(&values, true).unwrap(), CANARY);
        let temporary = SensitiveJson::new(&values).unwrap();
        assert!(!format!("{temporary:?}").contains(CANARY));
        assert_eq!(temporary.value(), &json!(CANARY));
    }

    #[test]
    fn expression_sources_are_absent_from_predicate_containers() {
        let mut values =
            AuthoredValue::from_data(json!({"settings":{"region":"eu"},"items":[]})).unwrap();
        if let ValueTree::Object(root) = &mut values
            && let Some(ValueTree::Object(settings)) = root.get_mut("settings")
        {
            settings.insert(
                "region".to_owned(),
                ValueTree::Expression(crate::Expression::new(CANARY)),
            );
        }
        let context = predicate_context(&graph(), &values, &[]).unwrap();
        assert_eq!(
            context.get(&ValuePath::parse("/settings").unwrap()),
            Some(&json!({}))
        );
        assert!(
            !context
                .get(&ValuePath::root())
                .unwrap()
                .to_string()
                .contains(CANARY)
        );
    }

    #[test]
    fn unknown_native_union_payload_is_unavailable() {
        let document: SchemaGraphDocument = serde_json::from_value(json!({
            "version":3,"root":{"target":"choice"},"definitions":[
                {"key":"choice","body":{"kind":"union","tagging":{"adjacent":{"tag":"kind","content":"payload"}},"variants":[
                    {"key":"private","payload":{"target":"text","protection":"secret_utf8"}}
                ]}},
                {"key":"text","body":{"kind":"string"}}
            ]
        })).unwrap();
        let graph = document.admit().unwrap();
        let values = AuthoredValue::from_data(
            json!({"kind":"unknown","payload":{"nested":CANARY},"extra":CANARY}),
        )
        .unwrap();
        let context = predicate_context(&graph, &values, &[]).unwrap();
        assert_eq!(context.get(&ValuePath::root()), Some(&Value::Null));
    }

    #[test]
    fn erased_temporary_removes_strings_and_keys() {
        let mut value = json!({(CANARY):[CANARY,{"nested":CANARY}]});
        erase_text(&mut value);
        assert_eq!(value, json!({}));
    }

    #[test]
    fn native_protected_bytes_are_guarded_and_decoder_failures_are_redacted() {
        let values = ResolvedValue::Secret(SecretValue::bytes(CANARY.as_bytes().to_vec()));
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            CANARY.as_bytes(),
        );
        let temporary = SensitiveJson::new(&values).unwrap();
        assert_eq!(temporary.value(), &Value::String(encoded.clone()));
        assert!(!format!("{temporary:?}").contains(&encoded));
        assert!(!format!("{temporary:?}").contains(CANARY));
        let error = decode_wire::<RejectedSecret>(&values, true).unwrap_err();
        assert!(!format!("{error:?} {error}").contains(&encoded));
        assert!(!format!("{error:?} {error}").contains(CANARY));
        assert!(decode_wire::<Value>(&values, false).is_err());
    }
}
