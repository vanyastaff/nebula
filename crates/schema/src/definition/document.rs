use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};

use super::{
    MAX_GRAPH_DOCUMENT_BYTES, SCHEMA_GRAPH_WIRE_VERSION, admission, error::SchemaAdmissionError,
};

/// A bounded, lossless current-version semantic graph document.
///
/// Decoding preserves unknown members and body values. Admission separately
/// rejects unknown mandatory semantics while permitting optional `x-*`
/// extensions. Self-describing JSON-data-model deserializers are supported and
/// the retained representation is bounded; transports must independently cap
/// input bytes and nesting depth when they require protection before
/// deserialization allocates.
#[derive(Clone, PartialEq, Eq)]
pub struct SchemaGraphDocument {
    raw: Value,
}

impl SchemaGraphDocument {
    /// Consumes this document and admits its complete executable semantics.
    ///
    /// # Errors
    ///
    /// Returns a redacted [`SchemaAdmissionError`] containing both the
    /// validation report and the original lossless document.
    ///
    /// Admission proves conservative structural inhabitation. General
    /// predicate satisfiability, intrinsic and value-rule satisfiability, and
    /// interactions among independent validation bounds remain runtime
    /// validation concerns.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_schema::SchemaGraphDocument;
    /// use serde_json::json;
    ///
    /// let document: SchemaGraphDocument = serde_json::from_value(json!({
    ///     "version": 3,
    ///     "root": { "target": "root" },
    ///     "definitions": [{ "key": "root", "body": { "kind": "string" } }]
    /// }))?;
    /// let admitted = document.admit()?;
    /// assert_eq!(admitted.definition_count(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn admit(self) -> Result<super::AdmittedSchemaGraph, SchemaAdmissionError> {
        admission::admit(self)
    }

    pub(super) const fn raw(&self) -> &Value {
        &self.raw
    }
}

impl fmt::Debug for SchemaGraphDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SchemaGraphDocument")
            .field("version", &SCHEMA_GRAPH_WIRE_VERSION)
            .field("content", &"<redacted>")
            .finish()
    }
}

impl Serialize for SchemaGraphDocument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchemaGraphDocument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = UniqueValueSeed.deserialize(deserializer)?;
        let encoded_len = serde_json::to_vec(&raw).map_err(de::Error::custom)?.len();
        if encoded_len > MAX_GRAPH_DOCUMENT_BYTES {
            return Err(de::Error::custom(
                "schema graph document exceeds byte limit",
            ));
        }
        Ok(Self { raw })
    }
}

#[derive(Clone, Copy)]
struct UniqueValueSeed;

impl<'de> DeserializeSeed<'de> for UniqueValueSeed {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object members")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_i128<E: de::Error>(self, value: i128) -> Result<Value, E> {
        if let Ok(value) = i64::try_from(value) {
            return Ok(Value::Number(Number::from(value)));
        }
        if let Ok(value) = u64::try_from(value) {
            return Ok(Value::Number(Number::from(value)));
        }
        Err(E::custom(
            "schema graph integer is not representable by serde_json::Number",
        ))
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<Value, E> {
        u64::try_from(value)
            .map(Number::from)
            .map(Value::Number)
            .map_err(|_| {
                E::custom("schema graph integer is not representable by serde_json::Number")
            })
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("schema graph contains a non-finite number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        UniqueValueSeed.deserialize(deserializer)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Value, D::Error> {
        UniqueValueSeed.deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(UniqueValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(
                    "schema graph has a duplicate object member",
                ));
            }
            let value = object.next_value_seed(UniqueValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
