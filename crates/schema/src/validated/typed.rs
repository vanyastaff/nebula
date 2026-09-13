//! Borrowing typed extraction for validated trees, including protected leaves.

use std::{borrow::Cow, collections::HashSet, fmt};

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    VariantAccess, Visitor, value::BorrowedStrDeserializer,
};
use serde_json::{Number, Value};
use zeroize::Zeroizing;

use super::{MODE_PAYLOAD_KEY, MODE_SELECTOR_KEY, SchemaKind, SerdeTagging, ValidSchema};
use crate::{
    Field, ResolvedValue, SecretValue, ValidationError, ValuePath, ValueTree, field::ModeField,
};

#[derive(Clone, Copy)]
pub(super) enum SecretDisclosure {
    Refuse,
    Expose,
}

enum SensitiveValue<'a> {
    Null,
    Bool(bool),
    Number(&'a Number),
    Text(&'a str),
    ProtectedText(&'a str),
    ProtectedOwned(Zeroizing<String>),
    List(Vec<Self>),
    Object(Vec<(Cow<'a, str>, Self)>),
}

/// Deserialize without retaining serde's potentially secret-bearing diagnostics.
#[derive(Debug)]
struct SensitiveDecodeError;

impl fmt::Display for SensitiveDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("typed value decoding failed")
    }
}

impl std::error::Error for SensitiveDecodeError {}

impl de::Error for SensitiveDecodeError {
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Self
    }
}

pub(super) fn decode<T: DeserializeOwned>(
    schema: &ValidSchema,
    values: &ResolvedValue,
    disclosure: SecretDisclosure,
) -> Result<T, ValidationError> {
    let path = ValuePath::root();
    let tree = match values {
        ValueTree::Object(properties) => {
            project_sensitive_level(schema.fields(), properties, disclosure, &path)?
        },
        _ => build_sensitive_tree(values, disclosure, &path)?,
    };
    let wire = project_root_union_wire(schema, tree);
    T::deserialize(&wire).map_err(|error| {
        ValidationError::builder("type_mismatch")
            .message("validated data cannot be decoded as the requested type")
            .private_source(error)
            .build()
    })
}

fn project_sensitive_level<'a>(
    fields: &'a [Field],
    properties: &'a indexmap::IndexMap<String, ResolvedValue>,
    disclosure: SecretDisclosure,
    path: &ValuePath,
) -> Result<SensitiveValue<'a>, ValidationError> {
    let reserved: HashSet<&str> = fields
        .iter()
        .flat_map(|field| {
            std::iter::once(field.key().as_str())
                .chain(field.read_aliases().iter().map(crate::FieldKey::as_str))
                .chain(field.emit_as().map(crate::FieldKey::as_str))
        })
        .collect();
    let mut projected = Vec::with_capacity(properties.len());
    for field in fields {
        let Some(value) = property_for_field(field, properties) else {
            continue;
        };
        let output_key = field.emit_as().unwrap_or_else(|| field.key()).as_str();
        projected.push((
            Cow::Borrowed(output_key),
            project_sensitive_field(field, value, disclosure, &path.push(field.key().as_str()))?,
        ));
    }
    for (key, value) in properties {
        if !reserved.contains(key.as_str()) {
            projected.push((
                Cow::Borrowed(key.as_str()),
                build_sensitive_tree(value, disclosure, &path.push(key))?,
            ));
        }
    }
    Ok(SensitiveValue::Object(projected))
}

fn property_for_field<'a>(
    field: &Field,
    properties: &'a indexmap::IndexMap<String, ResolvedValue>,
) -> Option<&'a ResolvedValue> {
    properties.get(field.key().as_str()).or_else(|| {
        field
            .read_aliases()
            .iter()
            .find_map(|alias| properties.get(alias.as_str()))
    })
}

fn project_sensitive_field<'a>(
    field: &'a Field,
    value: &'a ResolvedValue,
    disclosure: SecretDisclosure,
    path: &ValuePath,
) -> Result<SensitiveValue<'a>, ValidationError> {
    match (field, value) {
        (Field::Object(object), ValueTree::Object(properties)) => {
            project_sensitive_level(&object.fields, properties, disclosure, path)
        },
        (Field::List(list), ValueTree::List(items)) if let Some(item) = list.item.as_deref() => {
            items
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    project_sensitive_field(item, value, disclosure, &path.push(index.to_string()))
                })
                .collect::<Result<_, _>>()
                .map(SensitiveValue::List)
        },
        (Field::Mode(mode), ValueTree::Object(properties)) => {
            project_sensitive_mode(mode, properties, disclosure, path)
        },
        _ => build_sensitive_tree(value, disclosure, path),
    }
}

fn project_sensitive_mode<'a>(
    mode: &'a ModeField,
    properties: &'a indexmap::IndexMap<String, ResolvedValue>,
    disclosure: SecretDisclosure,
    path: &ValuePath,
) -> Result<SensitiveValue<'a>, ValidationError> {
    let mut projected = Vec::with_capacity(2);
    if let Some(selector) = properties.get(MODE_SELECTOR_KEY) {
        projected.push((
            Cow::Borrowed(MODE_SELECTOR_KEY),
            build_sensitive_tree(selector, disclosure, &path.push(MODE_SELECTOR_KEY))?,
        ));
    }
    if let Some(payload) = properties.get(MODE_PAYLOAD_KEY)
        && let Some(variant) = super::active_mode_variant_for_object(mode, properties)
    {
        projected.push((
            Cow::Borrowed(MODE_PAYLOAD_KEY),
            project_sensitive_field(
                &variant.field,
                payload,
                disclosure,
                &path.push(MODE_PAYLOAD_KEY),
            )?,
        ));
    }
    Ok(SensitiveValue::Object(projected))
}

fn build_sensitive_tree<'a>(
    value: &'a ResolvedValue,
    disclosure: SecretDisclosure,
    path: &ValuePath,
) -> Result<SensitiveValue<'a>, ValidationError> {
    match value {
        ValueTree::Literal(value) => Ok(borrow_json(value.as_json())),
        ValueTree::Object(values) => values
            .iter()
            .map(|(key, value)| {
                build_sensitive_tree(value, disclosure, &path.push(key))
                    .map(|value| (Cow::Borrowed(key.as_str()), value))
            })
            .collect::<Result<_, _>>()
            .map(SensitiveValue::Object),
        ValueTree::List(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                build_sensitive_tree(value, disclosure, &path.push(index.to_string()))
            })
            .collect::<Result<_, _>>()
            .map(SensitiveValue::List),
        ValueTree::Expression(_) => Err(ValidationError::builder("type_mismatch")
            .at(path.clone())
            .message("resolved data cannot contain an expression")
            .build()),
        ValueTree::Secret(secret) => match disclosure {
            SecretDisclosure::Refuse => Err(ValidationError::builder("type_mismatch")
                .at(path.clone())
                .message("secret-bearing data requires explicit secret access")
                .build()),
            SecretDisclosure::Expose => Ok(expose_secret(secret)),
        },
    }
}

fn borrow_json(value: &Value) -> SensitiveValue<'_> {
    match value {
        Value::Null => SensitiveValue::Null,
        Value::Bool(value) => SensitiveValue::Bool(*value),
        Value::Number(value) => SensitiveValue::Number(value),
        Value::String(value) => SensitiveValue::Text(value),
        Value::Array(values) => SensitiveValue::List(values.iter().map(borrow_json).collect()),
        Value::Object(properties) => SensitiveValue::Object(
            properties
                .iter()
                .map(|(key, value)| (Cow::Borrowed(key.as_str()), borrow_json(value)))
                .collect(),
        ),
    }
}

fn expose_secret(secret: &SecretValue) -> SensitiveValue<'_> {
    match secret {
        SecretValue::String(value) => SensitiveValue::ProtectedText(value.expose()),
        SecretValue::Bytes(value) => {
            let bytes = value.expose();
            let mut encoded = Zeroizing::new(String::with_capacity(bytes.len().saturating_mul(2)));
            for byte in bytes {
                encoded.push(hex_digit(byte >> 4));
                encoded.push(hex_digit(byte & 0x0f));
            }
            SensitiveValue::ProtectedOwned(encoded)
        },
    }
}

const fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'a' + nibble - 10) as char,
    }
}

fn project_root_union_wire<'a>(
    schema: &'a ValidSchema,
    mut tree: SensitiveValue<'a>,
) -> SensitiveValue<'a> {
    if schema.kind() != SchemaKind::Union {
        return tree;
    }
    let (Some(Field::Mode(mode)), Some(tagging)) =
        (schema.fields().first(), schema.serde_tagging())
    else {
        return tree;
    };
    let SensitiveValue::Object(properties) = &mut tree else {
        return tree;
    };
    let Some(root_index) = properties
        .iter()
        .position(|(key, _)| key.as_ref() == mode.key.as_str())
    else {
        return tree;
    };
    let SensitiveValue::Object(envelope) = &properties[root_index].1 else {
        return tree;
    };
    let Some(SensitiveValue::Text(variant_key)) = envelope
        .iter()
        .find(|(key, _)| key.as_ref() == MODE_SELECTOR_KEY)
        .map(|(_, value)| value)
    else {
        return tree;
    };
    let Some(variant) = mode
        .variants
        .iter()
        .find(|variant| variant.key == *variant_key)
    else {
        return tree;
    };
    let is_unit = variant.field.key().as_str() == ModeField::EMPTY_PLACEHOLDER_KEY;
    let has_payload = envelope
        .iter()
        .any(|(key, _)| key.as_ref() == MODE_PAYLOAD_KEY);
    if envelope
        .iter()
        .any(|(key, _)| key.as_ref() != MODE_SELECTOR_KEY && key.as_ref() != MODE_PAYLOAD_KEY)
        || is_unit == has_payload
    {
        return tree;
    }

    let (_, SensitiveValue::Object(mut envelope)) = properties.remove(root_index) else {
        return tree;
    };
    let payload = envelope
        .iter()
        .position(|(key, _)| key.as_ref() == MODE_PAYLOAD_KEY)
        .map(|index| envelope.remove(index).1);
    let canonical_variant = variant.key.as_str();
    match tagging {
        SerdeTagging::External if is_unit => SensitiveValue::Text(canonical_variant),
        SerdeTagging::External => SensitiveValue::Object(vec![(
            Cow::Borrowed(canonical_variant),
            payload.unwrap_or(SensitiveValue::Null),
        )]),
        SerdeTagging::Adjacent { tag, content } => {
            let mut wire = Vec::with_capacity(2);
            wire.push((
                Cow::Borrowed(tag.as_str()),
                SensitiveValue::Text(canonical_variant),
            ));
            if let Some(payload) = payload {
                wire.push((Cow::Borrowed(content.as_str()), payload));
            }
            SensitiveValue::Object(wire)
        },
    }
}

impl<'de, 'a: 'de> de::Deserializer<'de> for &'de SensitiveValue<'a> {
    type Error = SensitiveDecodeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self {
            SensitiveValue::Null => visitor.visit_unit(),
            SensitiveValue::Bool(value) => visitor.visit_bool(*value),
            SensitiveValue::Number(value) => deserialize_number(value, visitor),
            SensitiveValue::Text(value) | SensitiveValue::ProtectedText(value) => {
                visitor.visit_borrowed_str(value)
            },
            SensitiveValue::ProtectedOwned(value) => visitor.visit_borrowed_str(value.as_str()),
            SensitiveValue::List(values) => {
                let mut access = SensitiveSeqAccess {
                    values: values.iter(),
                };
                let decoded = visitor.visit_seq(&mut access)?;
                if access.values.len() == 0 {
                    Ok(decoded)
                } else {
                    Err(SensitiveDecodeError)
                }
            },
            SensitiveValue::Object(properties) => {
                let mut access = SensitiveMapAccess {
                    properties: properties.iter(),
                    pending_value: None,
                };
                let decoded = visitor.visit_map(&mut access)?;
                if access.properties.len() == 0 && access.pending_value.is_none() {
                    Ok(decoded)
                } else {
                    Err(SensitiveDecodeError)
                }
            },
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self {
            SensitiveValue::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self {
            SensitiveValue::Text(variant) => visitor.visit_enum(variant.into_deserializer()),
            SensitiveValue::Object(properties) if properties.len() == 1 => {
                let (variant, value) = &properties[0];
                visitor.visit_enum(SensitiveEnumAccess {
                    variant: variant.as_ref(),
                    value,
                })
            },
            _ => Err(SensitiveDecodeError),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
        identifier
    }
}

fn deserialize_number<'de, V: Visitor<'de>>(
    number: &Number,
    visitor: V,
) -> Result<V::Value, SensitiveDecodeError> {
    if let Some(value) = number.as_i64() {
        visitor.visit_i64(value)
    } else if let Some(value) = number.as_u64() {
        visitor.visit_u64(value)
    } else if let Some(value) = number.as_f64() {
        visitor.visit_f64(value)
    } else {
        Err(SensitiveDecodeError)
    }
}

struct SensitiveSeqAccess<'de, 'a> {
    values: std::slice::Iter<'de, SensitiveValue<'a>>,
}

impl<'de, 'a: 'de> SeqAccess<'de> for SensitiveSeqAccess<'de, 'a> {
    type Error = SensitiveDecodeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.values
            .next()
            .map(|value| seed.deserialize(value))
            .transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.values.len())
    }
}

struct SensitiveMapAccess<'de, 'a> {
    properties: std::slice::Iter<'de, (Cow<'a, str>, SensitiveValue<'a>)>,
    pending_value: Option<&'de SensitiveValue<'a>>,
}

impl<'de, 'a: 'de> MapAccess<'de> for SensitiveMapAccess<'de, 'a> {
    type Error = SensitiveDecodeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, value)) = self.properties.next() else {
            return Ok(None);
        };
        self.pending_value = Some(value);
        seed.deserialize(BorrowedStrDeserializer::new(key.as_ref()))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let Some(value) = self.pending_value.take() else {
            return Err(SensitiveDecodeError);
        };
        seed.deserialize(value)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.properties.len())
    }
}

struct SensitiveEnumAccess<'de, 'a> {
    variant: &'de str,
    value: &'de SensitiveValue<'a>,
}

impl<'de, 'a: 'de> EnumAccess<'de> for SensitiveEnumAccess<'de, 'a> {
    type Error = SensitiveDecodeError;
    type Variant = SensitiveVariantAccess<'de, 'a>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant = seed.deserialize(BorrowedStrDeserializer::new(self.variant))?;
        Ok((variant, SensitiveVariantAccess { value: self.value }))
    }
}

struct SensitiveVariantAccess<'de, 'a> {
    value: &'de SensitiveValue<'a>,
}

impl<'de, 'a: 'de> VariantAccess<'de> for SensitiveVariantAccess<'de, 'a> {
    type Error = SensitiveDecodeError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        serde::Deserialize::deserialize(self.value)
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        seed.deserialize(self.value)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        de::Deserializer::deserialize_seq(self.value, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        de::Deserializer::deserialize_map(self.value, visitor)
    }
}
