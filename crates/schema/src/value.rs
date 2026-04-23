//! Runtime value tree and container.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    expression::Expression,
    key::FieldKey,
    path::{FieldPath, PathSegment},
    secret::SecretValue,
};

/// Reserved key for an explicit expression wrapper.
pub const EXPRESSION_KEY: &str = "$expr";

/// Runtime value — may be literal, expression, tree, or mode-dispatched.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// Plain JSON literal (number, bool, null, or non-expression string).
    Literal(Value),
    /// Expression template to be evaluated at runtime.
    Expression(Expression),
    /// Nested key-value map.
    Object(IndexMap<FieldKey, Self>),
    /// Ordered sequence of values.
    List(Vec<Self>),
    /// Discriminated mode payload. JSON object `{"mode": "...", "value": ...}` — see [`ModeField`]
    /// for how `value` is shaped (object, array, or scalar).
    Mode {
        /// Chosen mode key.
        mode: FieldKey,
        /// Optional mode payload.
        value: Option<Box<Self>>,
    },
    /// Redacted secret material (introduced at resolve time for `Field::Secret`).
    SecretLiteral(SecretValue),
}

impl FieldValue {
    /// Parse a raw JSON value into a typed tree.
    ///
    /// This function preserves object literals when keys are not valid
    /// [`FieldKey`] identifiers.
    pub fn from_json(value: Value) -> Self {
        match value {
            Value::Object(map) => {
                if map.len() == 1
                    && let Some(expr) = map.get(EXPRESSION_KEY).and_then(Value::as_str)
                {
                    return Self::Expression(Expression::new(expr));
                }
                let only_mode_keys = map.keys().all(|k| k == "mode" || k == "value");
                if only_mode_keys
                    && map.contains_key("mode")
                    && let Some(mode_str) = map.get("mode").and_then(Value::as_str)
                    && let Ok(mode_key) = FieldKey::new(mode_str)
                {
                    let v = map
                        .get("value")
                        .cloned()
                        .map(|inner| Box::new(Self::from_json(inner)));
                    return Self::Mode {
                        mode: mode_key,
                        value: v,
                    };
                }

                // Parse keys first so conversion remains panic-free.
                let Some(parsed_keys): Option<Vec<FieldKey>> = map
                    .keys()
                    .map(|key| FieldKey::new(key.as_str()).ok())
                    .collect()
                else {
                    return Self::Literal(Value::Object(map));
                };

                let mut out: IndexMap<FieldKey, Self> = IndexMap::with_capacity(map.len());
                for ((_, v), key) in map.into_iter().zip(parsed_keys) {
                    out.insert(key, Self::from_json(v));
                }
                Self::Object(out)
            },
            Value::Array(arr) => Self::List(arr.into_iter().map(Self::from_json).collect()),
            Value::String(text) if contains_expression_marker(&text) => {
                Self::Expression(Expression::new(text))
            },
            _ => Self::Literal(value),
        }
    }

    /// Encode into canonical JSON wire format.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Literal(v) => v.clone(),
            Self::SecretLiteral(s) => s.to_json(),
            Self::Expression(e) => serde_json::json!({ EXPRESSION_KEY: e.source() }),
            Self::Object(map) => {
                let mut out = Map::with_capacity(map.len());
                for (k, v) in map {
                    out.insert(k.as_str().to_owned(), v.to_json());
                }
                Value::Object(out)
            },
            Self::List(items) => Value::Array(items.iter().map(Self::to_json).collect()),
            Self::Mode { mode, value } => {
                let mut out = Map::new();
                out.insert("mode".into(), Value::String(mode.as_str().to_owned()));
                if let Some(v) = value {
                    out.insert("value".into(), v.to_json());
                }
                Value::Object(out)
            },
        }
    }

    /// Navigate to a nested value using a typed path.
    #[must_use]
    pub fn path(&self, path: &FieldPath) -> Option<&Self> {
        let mut cur = self;
        for seg in path.segments() {
            cur = match (cur, seg) {
                (Self::Object(map), PathSegment::Key(k)) => map.get(k)?,
                (Self::List(items), PathSegment::Index(i)) => items.get(*i)?,
                (
                    Self::Mode {
                        value: Some(inner), ..
                    },
                    PathSegment::Key(k),
                ) if k.as_str() == "value" => inner,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Returns true when this value is an expression variant.
    #[must_use]
    pub const fn is_expression(&self) -> bool {
        matches!(self, Self::Expression(_))
    }
}

impl Serialize for FieldValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for FieldValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_json(Value::deserialize(d)?))
    }
}

fn contains_expression_marker(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if i + 3 < bytes.len() && bytes[i + 2] == b'{' && bytes[i + 3] == b'{' {
                i += 4;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

/// Top-level runtime value store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldValues(IndexMap<FieldKey, FieldValue>);

impl FieldValues {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a JSON object into a `FieldValues` store.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key` for invalid object keys, or `type_mismatch` when
    /// `value` is not a top-level object.
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn from_json(value: Value) -> Result<Self, crate::error::ValidationError> {
        validate_json_keys(&value, &FieldPath::root())?;
        match FieldValue::from_json(value) {
            FieldValue::Object(map) => Ok(Self(map)),
            _ => Err(crate::error::ValidationError::builder("type_mismatch")
                .message("top-level values must be a JSON object")
                .build()),
        }
    }

    /// Set a typed value by key.
    pub fn set(&mut self, key: FieldKey, value: FieldValue) {
        self.0.insert(key, value);
    }

    /// Convenience: set a raw JSON value by string key.
    ///
    /// Parses `key` as a [`FieldKey`] and wraps `value` via
    /// [`FieldValue::from_json`]. Panics if `key` is invalid — use only in
    /// tests/migrations with known-good keys. For runtime input, prefer
    /// [`Self::try_set_raw`].
    ///
    /// # Panics
    ///
    /// Panics when `key` is invalid or nested object keys fail validation.
    pub fn set_raw(&mut self, key: &str, value: Value) {
        self.try_set_raw(key, value)
            .unwrap_or_else(|e| panic!("set_raw failed for key {key:?}: {e}"));
    }

    /// Fallible variant of [`Self::set_raw`] for runtime code paths.
    ///
    /// Validates nested object keys before insertion and returns `invalid_key`
    /// when any path segment violates [`FieldKey`] constraints.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::error::ValidationError`] when `key` or nested keys
    /// are invalid.
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn try_set_raw(
        &mut self,
        key: &str,
        value: Value,
    ) -> Result<(), crate::error::ValidationError> {
        let fk = FieldKey::new(key).map_err(|e| {
            crate::error::ValidationError::builder("invalid_key")
                .message(format!("set_raw: invalid key {key:?}: {e}"))
                .param("key", Value::String(key.to_owned()))
                .build()
        })?;
        let field_path = FieldPath::root().join(fk.clone());
        validate_json_keys(&value, &field_path)?;
        self.0.insert(fk, FieldValue::from_json(value));
        Ok(())
    }

    /// Remove a value by key, returning it if present.
    pub fn remove(&mut self, key: &FieldKey) -> Option<FieldValue> {
        self.0.shift_remove(key)
    }

    /// Borrow a value by key.
    #[inline]
    #[must_use]
    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> {
        self.0.get(key)
    }

    /// Mutably borrow a value by key.
    #[inline]
    pub fn get_mut(&mut self, key: &FieldKey) -> Option<&mut FieldValue> {
        self.0.get_mut(key)
    }

    /// Get the raw JSON representation of a value by string key.
    ///
    /// Uses `Borrow<str>` on `FieldKey` — no allocation for the lookup.
    /// Returns `None` for invalid keys or missing entries.
    pub fn get_raw_by_str(&self, key: &str) -> Option<Value> {
        self.0.get(key).map(FieldValue::to_json)
    }

    /// Get a `FieldValue` by string key (convenience for migration code).
    ///
    /// Uses `Borrow<str>` on `FieldKey` — no allocation for the lookup.
    #[must_use]
    pub fn get_by_str(&self, key: &str) -> Option<&FieldValue> {
        self.0.get(key)
    }

    /// Navigate to a nested value using a typed path.
    #[must_use]
    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> {
        let mut segs = path.segments().iter();
        let PathSegment::Key(first) = segs.next()? else {
            return None;
        };
        let mut cur = self.0.get(first)?;
        for seg in segs {
            cur = match (cur, seg) {
                (FieldValue::Object(map), PathSegment::Key(k)) => map.get(k)?,
                (FieldValue::List(items), PathSegment::Index(i)) => items.get(*i)?,
                (
                    FieldValue::Mode {
                        value: Some(inner), ..
                    },
                    PathSegment::Key(k),
                ) if k.as_str() == "value" => inner,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Returns true when key exists.
    #[must_use]
    pub fn contains(&self, key: &FieldKey) -> bool {
        self.0.contains_key(key)
    }

    /// Check by string key (for migration code in schema.rs).
    #[must_use]
    pub fn contains_str(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Iterate over all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&FieldKey, &FieldValue)> {
        self.0.iter()
    }

    /// Number of values currently set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true when no values are set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Consume into the underlying map.
    #[must_use]
    pub fn into_inner(self) -> IndexMap<FieldKey, FieldValue> {
        self.0
    }

    /// Encode all values to a JSON object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::with_capacity(self.0.len());
        for (k, v) in &self.0 {
            out.insert(k.as_str().to_owned(), v.to_json());
        }
        Value::Object(out)
    }

    /// Produce a `HashMap<String, Value>` for rule-evaluation context.
    ///
    /// Used by `schema.rs` validate logic which expects `HashMap<String, Value>`.
    #[must_use]
    pub fn to_context_map(&self) -> HashMap<String, Value> {
        self.0
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_json()))
            .collect()
    }

    /// Get a string literal value by key.
    #[must_use]
    pub fn get_string(&self, key: &FieldKey) -> Option<&str> {
        match self.0.get(key)? {
            FieldValue::Literal(Value::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Get string by string key (for loader context and migration code).
    #[must_use]
    pub fn get_string_by_str(&self, key: &str) -> Option<&str> {
        match self.0.get(key)? {
            FieldValue::Literal(Value::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Get a bool literal value by key.
    #[must_use]
    pub fn get_bool(&self, key: &FieldKey) -> Option<bool> {
        match self.0.get(key)? {
            FieldValue::Literal(v) => v.as_bool(),
            _ => None,
        }
    }
    /// Get an i64 literal value by key.
    #[must_use]
    pub fn get_i64(&self, key: &FieldKey) -> Option<i64> {
        match self.0.get(key)? {
            FieldValue::Literal(v) => v.as_i64(),
            _ => None,
        }
    }
    /// Get an f64 literal value by key.
    #[must_use]
    pub fn get_f64(&self, key: &FieldKey) -> Option<f64> {
        match self.0.get(key)? {
            FieldValue::Literal(v) => v.as_f64(),
            _ => None,
        }
    }
}

#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn validate_json_keys(
    value: &Value,
    path: &FieldPath,
) -> Result<(), crate::error::ValidationError> {
    match value {
        Value::Object(map) => {
            if map.len() == 1 && map.get(EXPRESSION_KEY).is_some_and(Value::is_string) {
                return Ok(());
            }

            let only_mode_keys = map.keys().all(|k| k == "mode" || k == "value");
            let valid_mode_key = map
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| FieldKey::new(mode).is_ok());
            if only_mode_keys && valid_mode_key {
                if let Some(inner) = map.get("value") {
                    let payload_key = FieldKey::new("value").expect("value is a valid field key");
                    let payload_path = path.clone().join(payload_key);
                    validate_json_keys(inner, &payload_path)?;
                }
                return Ok(());
            }

            for (raw_key, child) in map {
                let key = FieldKey::new(raw_key).map_err(|e| {
                    crate::error::ValidationError::builder("invalid_key")
                        .at(path.clone())
                        .message(format!("invalid key `{raw_key}`: {e}"))
                        .param("key", Value::String(raw_key.clone()))
                        .build()
                })?;
                let child_path = path.clone().join(key);
                validate_json_keys(child, &child_path)?;
            }
            Ok(())
        },
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let item_path = path.clone().join(idx);
                validate_json_keys(item, &item_path)?;
            }
            Ok(())
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn from_json_flat_literal() {
        let v = FieldValue::from_json(json!(42));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn from_json_object_becomes_tree() {
        let v = FieldValue::from_json(json!({"a": 1, "b": "x"}));
        let FieldValue::Object(map) = v else { panic!() };
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn detects_expression_wrapper() {
        let v = FieldValue::from_json(json!({"$expr": "{{ $x }}"}));
        assert!(matches!(v, FieldValue::Expression(_)));
    }

    #[test]
    fn detects_inline_expression_marker() {
        let v = FieldValue::from_json(json!("hello {{ $y }}"));
        assert!(matches!(v, FieldValue::Expression(_)));
    }

    #[test]
    fn escaped_double_braces_stay_literal() {
        let v = FieldValue::from_json(json!("{{{{ x }}}}"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn detects_mode_wrapper() {
        let v = FieldValue::from_json(json!({"mode": "oauth2", "value": {"scope":"r"}}));
        assert!(matches!(v, FieldValue::Mode { .. }));
    }

    #[test]
    fn mode_with_extra_keys_stays_object() {
        let v = FieldValue::from_json(json!({"mode":"x","value":null,"extra":1}));
        assert!(matches!(v, FieldValue::Object(_)));
    }

    #[test]
    fn values_set_get_path() {
        let mut vs = FieldValues::new();
        let key = FieldKey::new("user").unwrap();
        let email = FieldKey::new("email").unwrap();
        vs.set(
            key,
            FieldValue::Object(indexmap::indexmap! { email => FieldValue::Literal(json!("a@b")) }),
        );
        let p = FieldPath::parse("user.email").unwrap();
        assert!(matches!(vs.get_path(&p), Some(FieldValue::Literal(_))));
    }

    #[test]
    fn values_get_path_through_mode_value() {
        let mut vs = FieldValues::new();
        let auth = FieldKey::new("auth").unwrap();
        let mode = FieldKey::new("oauth").unwrap();
        let token = FieldKey::new("token").unwrap();
        vs.set(
            auth,
            FieldValue::Mode {
                mode,
                value: Some(Box::new(FieldValue::Object(indexmap::indexmap! {
                    token => FieldValue::Literal(json!("secret"))
                }))),
            },
        );

        let p = FieldPath::parse("auth.value.token").unwrap();
        assert_eq!(vs.get_path(&p), Some(&FieldValue::Literal(json!("secret"))));
    }

    #[test]
    fn field_values_from_json_rejects_invalid_nested_key() {
        let err = FieldValues::from_json(json!({
            "user": {
                "bad-key": "x"
            }
        }))
        .unwrap_err();
        assert_eq!(err.code, "invalid_key");
    }

    #[test]
    fn field_value_from_json_does_not_drop_invalid_object_keys() {
        let raw = json!({"bad-key": 1, "ok_key": 2});
        let parsed = FieldValue::from_json(raw.clone());
        assert_eq!(parsed, FieldValue::Literal(raw));
    }

    #[test]
    fn try_set_raw_rejects_invalid_nested_key() {
        let mut values = FieldValues::new();
        let err = values
            .try_set_raw("config", json!({"bad-key": "x"}))
            .unwrap_err();
        assert_eq!(err.code, "invalid_key");
    }

    #[test]
    fn set_raw_parses_expression_wrapper() {
        let mut vs = FieldValues::new();
        vs.set_raw("expr", json!({"$expr":"{{ $x }}"}));
        assert!(matches!(
            vs.get(&FieldKey::new("expr").unwrap()),
            Some(FieldValue::Expression(_))
        ));
    }

    #[test]
    fn roundtrip_preserves_structure() {
        let src = json!({
            "a": 1,
            "b": [1, 2, {"x": true}],
            "c": {"$expr": "{{ $x }}"},
            "d": {"mode": "m", "value": "v"}
        });
        let parsed = FieldValue::from_json(src.clone());
        let back = parsed.to_json();
        assert_eq!(back, src);
    }
}
