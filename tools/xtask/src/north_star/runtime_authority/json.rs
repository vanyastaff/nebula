use std::fmt;

use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use super::VerificationError;

const MAX_DEPTH: usize = 16;
const MAX_COLLECTION: usize = 256;
const MAX_STRING_BYTES: usize = 4096;
const MAX_VALUES: usize = 16_384;

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, VerificationError> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let mut remaining = MAX_VALUES;
    let value = BoundedValue {
        depth: 0,
        remaining: &mut remaining,
    }
    .deserialize(&mut decoder)
    .map_err(|_| VerificationError::InvalidJson)?;
    decoder.end().map_err(|_| VerificationError::InvalidJson)?;
    serde_json::from_value(value).map_err(|_| VerificationError::InvalidJson)
}

struct BoundedValue<'a> {
    depth: usize,
    remaining: &'a mut usize,
}

impl<'de> DeserializeSeed<'de> for BoundedValue<'_> {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.depth > MAX_DEPTH || *self.remaining == 0 {
            return Err(de::Error::custom("JSON structural limit"));
        }
        *self.remaining -= 1;
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for BoundedValue<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded JSON with unique object keys")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| de::Error::custom("nonfinite JSON number"))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        if value.len() > MAX_STRING_BYTES {
            return Err(de::Error::custom("JSON string limit"));
        }
        Ok(Value::String(value.into()))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(BoundedValue {
            depth: self.depth + 1,
            remaining: self.remaining,
        })? {
            if values.len() == MAX_COLLECTION {
                return Err(de::Error::custom("JSON collection limit"));
            }
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
        let mut fields = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if key.len() > MAX_STRING_BYTES
                || fields.len() == MAX_COLLECTION
                || fields.contains_key(&key)
            {
                return Err(de::Error::custom("JSON object limit or duplicate key"));
            }
            let value = object.next_value_seed(BoundedValue {
                depth: self.depth + 1,
                remaining: self.remaining,
            })?;
            fields.insert(key, value);
        }
        Ok(Value::Object(fields))
    }
}
