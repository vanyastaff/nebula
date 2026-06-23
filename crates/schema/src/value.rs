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

/// Maximum recursion depth permitted for user-provided value trees.
///
/// `validate_json_keys`, `validate_field`, `resolve_value`, and
/// `promote_secrets_in_value` all walk through `FieldValue::Object`,
/// `FieldValue::List`, and `FieldValue::Mode` containers without
/// natural bounds. Adversarial JSON with deeply nested objects can
/// otherwise overflow the call stack. Any recursion that exceeds this
/// depth produces a `recursion_limit` validation error and stops
/// descending. The limit is intentionally conservative (64) because
/// realistic schemas are flat (≤ 5–10 levels) and JSON-Schema Draft
/// 2020-12 does not encourage deeply nested shapes.
pub const MAX_VALUE_DEPTH: u8 = 64;

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
    /// Discriminated mode payload. JSON object `{"mode": "...", "value": ...}` — see
    /// [`crate::ModeField`] for how `value` is shaped (object, array, or scalar).
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
    ///
    /// If the value exceeds [`MAX_VALUE_DEPTH`], conversion stops at the
    /// offending subtree and preserves it as a literal JSON value. Use
    /// [`Self::try_from_json`] when runtime input must reject over-deep
    /// values instead of preserving them.
    pub fn from_json(value: Value) -> Self {
        Self::from_json_limited(value, 0)
    }

    /// Parse a raw JSON value into a typed tree, rejecting values that exceed
    /// [`MAX_VALUE_DEPTH`].
    ///
    /// This is the fallible variant for runtime input. Invalid object keys are
    /// still preserved as literal objects; [`FieldValues::from_json`] performs
    /// strict key validation before calling this helper.
    ///
    /// # Errors
    ///
    /// Returns `recursion_limit` when conversion would descend beyond
    /// [`MAX_VALUE_DEPTH`].
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn try_from_json(value: Value) -> Result<Self, crate::error::ValidationError> {
        Self::try_from_json_at(value, &FieldPath::root(), 0)
    }

    fn from_json_limited(value: Value, depth: u8) -> Self {
        if depth > MAX_VALUE_DEPTH {
            return Self::Literal(value);
        }
        match value {
            Value::Object(map) => {
                if map.len() == 1
                    && let Some(expr) = map.get(EXPRESSION_KEY).and_then(Value::as_str)
                {
                    return Self::Expression(Expression::new(expr));
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
                    out.insert(key, Self::from_json_limited(v, depth.saturating_add(1)));
                }
                Self::Object(out)
            },
            Value::Array(arr) => Self::List(
                arr.into_iter()
                    .map(|item| Self::from_json_limited(item, depth.saturating_add(1)))
                    .collect(),
            ),
            Value::String(text) if contains_expression_marker(&text) => {
                Self::Expression(Expression::new(text))
            },
            _ => Self::Literal(value),
        }
    }

    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    fn try_from_json_at(
        value: Value,
        path: &FieldPath,
        depth: u8,
    ) -> Result<Self, crate::error::ValidationError> {
        if depth > MAX_VALUE_DEPTH {
            return Err(recursion_limit_error(path));
        }
        match value {
            Value::Object(map) => {
                if map.len() == 1
                    && let Some(expr) = map.get(EXPRESSION_KEY).and_then(Value::as_str)
                {
                    return Ok(Self::Expression(Expression::new(expr)));
                }

                let Some(parsed_keys): Option<Vec<FieldKey>> = map
                    .keys()
                    .map(|key| FieldKey::new(key.as_str()).ok())
                    .collect()
                else {
                    validate_json_object_depth(&map, path, depth)?;
                    return Ok(Self::Literal(Value::Object(map)));
                };

                let mut out: IndexMap<FieldKey, Self> = IndexMap::with_capacity(map.len());
                for ((_, child), key) in map.into_iter().zip(parsed_keys) {
                    let child_path = path.clone().join(key.clone());
                    let child =
                        Self::try_from_json_at(child, &child_path, depth.saturating_add(1))?;
                    out.insert(key, child);
                }
                Ok(Self::Object(out))
            },
            Value::Array(arr) => {
                let mut out = Vec::with_capacity(arr.len());
                for (index, child) in arr.into_iter().enumerate() {
                    let child_path = path.clone().join(index);
                    out.push(Self::try_from_json_at(
                        child,
                        &child_path,
                        depth.saturating_add(1),
                    )?);
                }
                Ok(Self::List(out))
            },
            Value::String(text) if contains_expression_marker(&text) => {
                Ok(Self::Expression(Expression::new(text)))
            },
            _ => Ok(Self::Literal(value)),
        }
    }

    /// Encode into the JSON **wire** format (round-trips through serde).
    ///
    /// This is *not* a canonical / injective encoding — object key order is the
    /// insertion order and numbers keep their JSON spelling. For a stable
    /// content hash / dedup key use [`Self::canonical_bytes`].
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

    /// Injective canonical byte encoding of this value (content-addressing /
    /// dedup / idempotency key — **not** a wire format; see [`Self::to_json`]).
    ///
    /// Two values produce identical bytes **iff** they are canonically equal,
    /// independent of insertion order: object keys are emitted sorted, every
    /// variant carries a leading 1-byte tag (so a list and an object, or an
    /// expression and a string, can never collide), strings/bytes are
    /// length-prefixed (so `"a" + "b"` cannot alias `"ab"`), and counts are
    /// varint-framed. Numbers are normalized: an integral value is emitted as an
    /// integer regardless of JSON spelling, so `1`, `1.0`, and `-0.0` share one
    /// encoding (the `"1"`-vs-`"1.0"` dedup fix). The output is prefixed with a
    /// domain separator and [`VALUE_CANON_VERSION`], so bumping the version
    /// re-keys every hash.
    ///
    /// Canonical equality is therefore **coarser** than `FieldValue: PartialEq`:
    /// `Literal(1)` and `Literal(1.0)` are *not* `==` (distinct `serde_json::Number`s)
    /// yet share a canon / [`ContentId`]. Do not assume equal ids imply `==`.
    ///
    /// # Errors
    ///
    /// Returns `secret.not_hashable` if the value contains a
    /// [`SecretLiteral`](Self::SecretLiteral): secrets must never enter a
    /// content hash / dedup key (a deterministic hash of a low-entropy secret is
    /// a confirmation oracle, and a `<redacted>` placeholder would collide
    /// distinct secrets). Returns `value.non_canonical_float` for a non-finite
    /// float (defensive: `serde_json::Number` is always finite today).
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, crate::error::ValidationError> {
        let mut out = Vec::new();
        out.extend_from_slice(CANON_DOMAIN);
        out.extend_from_slice(&VALUE_CANON_VERSION.to_be_bytes());
        self.write_canonical(&mut out)?;
        Ok(out)
    }

    /// Content identifier — `blake3` over [`canonical_bytes`](Self::canonical_bytes).
    ///
    /// Equal structures yield equal ids automatically (Unison-style), so dedup /
    /// cache keys / versions need no name or semver. Field *names* still enter
    /// the id (object keys are part of the canon), unlike a pure structural hash.
    ///
    /// # Errors
    ///
    /// Propagates [`canonical_bytes`](Self::canonical_bytes) — a secret-bearing
    /// value has no content id by design.
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn content_id(&self) -> Result<ContentId, crate::error::ValidationError> {
        Ok(ContentId(blake3::hash(&self.canonical_bytes()?).into()))
    }

    /// Recursive canonical writer for a [`FieldValue`].
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    fn write_canonical(&self, out: &mut Vec<u8>) -> Result<(), crate::error::ValidationError> {
        match self {
            // A `Literal` carries a raw JSON value (including `Array`/`Object`
            // when keys were not valid `FieldKey`s — `from_json` preserves them),
            // canonicalized by the JSON writer below.
            Self::Literal(value) => write_canon_json(value, out)?,
            Self::Expression(expr) => {
                out.push(TAG_EXPRESSION);
                write_lp(out, expr.source().as_bytes());
            },
            Self::Object(map) => write_canon_object(map, out)?,
            Self::List(items) => {
                out.push(TAG_LIST);
                write_varint(out, items.len() as u64);
                for item in items {
                    item.write_canonical(out)?;
                }
            },
            Self::Mode { mode, value } => {
                out.push(TAG_MODE);
                write_lp(out, mode.as_str().as_bytes());
                match value {
                    Some(inner) => {
                        out.push(1);
                        inner.write_canonical(out)?;
                    },
                    None => out.push(0),
                }
            },
            // Secrets never enter a content hash (see `canonical_bytes` docs).
            Self::SecretLiteral(_) => return Err(secret_not_hashable()),
        }
        Ok(())
    }
}

// Note: `FieldValue` is intentionally `PartialEq` but not `Eq` here. Its derived
// `PartialEq` *is* a total equivalence (`serde_json::Number` is always finite, so
// no `NaN`), so `impl Eq` would be sound — but adding it ripples
// `clippy::derive_partial_eq_without_eq` onto every `PartialEq`-deriving type that
// transitively contains a `FieldValue`. That additive `Eq` rollout is a separate
// focused pass; content-addressing here keys off `canonical_bytes` / `ContentId`,
// not `FieldValue: Eq`.

/// Domain separator prepended to every [`FieldValue::canonical_bytes`] output,
/// so a value canon can never be confused with bytes from another protocol.
const CANON_DOMAIN: &[u8] = b"nbschema-value-v";

/// Version of the value-canonicalization format. **Separate** from the schema
/// wire version — a bump here re-keys every content hash / dedup bucket.
pub const VALUE_CANON_VERSION: u16 = 1;

// 1-byte variant tags, emitted before content so variants are non-confusable.
// `Literal` scalars reuse the JSON scalar tags; `Literal(Array/Object)` get
// JSON-container tags distinct from the typed `List`/`Object` tags.
const TAG_NULL: u8 = 0x01;
const TAG_BOOL: u8 = 0x02;
const TAG_INT: u8 = 0x03;
const TAG_FLOAT: u8 = 0x04;
const TAG_STRING: u8 = 0x05;
const TAG_LIST: u8 = 0x06;
const TAG_OBJECT: u8 = 0x07;
const TAG_EXPRESSION: u8 = 0x08;
const TAG_MODE: u8 = 0x09;
// 0x0A is reserved for an opt-in secret commitment, deferred. When implemented
// it MUST use a keyed / domain-separated hash (e.g. `blake3::keyed_hash` with an
// ephemeral per-process key), never the unkeyed `blake3::hash` used for content
// ids — an unkeyed hash of a low-entropy secret is a brute-forceable oracle,
// the exact property `secret.not_hashable` exists to prevent.
const TAG_JSON_ARRAY: u8 = 0x0B;
const TAG_JSON_OBJECT: u8 = 0x0C;

/// A 32-byte content identifier (`blake3` of a canonical encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentId([u8; 32]);

impl ContentId {
    /// Borrow the raw 32-byte digest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// `secret.not_hashable` — a secret cannot enter a content hash / dedup key.
fn secret_not_hashable() -> crate::error::ValidationError {
    crate::error::ValidationError::builder("secret.not_hashable")
        .message("secret values must not enter a content hash or dedup key")
        .build()
}

/// `value.non_canonical_float` — a non-finite float has no canonical encoding.
fn non_canonical_float() -> crate::error::ValidationError {
    crate::error::ValidationError::builder("value.non_canonical_float")
        .message("non-finite floats (NaN / ±Inf) have no canonical encoding")
        .build()
}

/// Append a length-prefixed byte string (varint length + bytes) — prevents
/// `"a" + "b"` from aliasing `"ab"`.
fn write_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Append an unsigned LEB128 varint.
fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Canonicalize a typed object body (`TAG_OBJECT` + sorted entries). Shared by
/// [`FieldValue::Object`]'s arm and [`FieldValues::canonical_bytes`] so the two
/// content-address identically *by construction*, not by a single example test.
#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn write_canon_object(
    map: &IndexMap<FieldKey, FieldValue>,
    out: &mut Vec<u8>,
) -> Result<(), crate::error::ValidationError> {
    out.push(TAG_OBJECT);
    let mut entries: Vec<(&FieldKey, &FieldValue)> = map.iter().collect();
    entries.sort_unstable_by(|(a, _), (b, _)| a.as_str().as_bytes().cmp(b.as_str().as_bytes()));
    write_varint(out, entries.len() as u64);
    for (key, value) in entries {
        write_lp(out, key.as_str().as_bytes());
        value.write_canonical(out)?;
    }
    Ok(())
}

/// Canonicalize a raw `serde_json::Value` (used for `FieldValue::Literal`).
#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn write_canon_json(value: &Value, out: &mut Vec<u8>) -> Result<(), crate::error::ValidationError> {
    match value {
        Value::Null => out.push(TAG_NULL),
        Value::Bool(b) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*b));
        },
        Value::Number(number) => write_canon_number(number, out)?,
        Value::String(string) => {
            out.push(TAG_STRING);
            write_lp(out, string.as_bytes());
        },
        Value::Array(items) => {
            out.push(TAG_JSON_ARRAY);
            write_varint(out, items.len() as u64);
            for item in items {
                write_canon_json(item, out)?;
            }
        },
        Value::Object(map) => {
            out.push(TAG_JSON_OBJECT);
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_unstable_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
            write_varint(out, entries.len() as u64);
            for (key, child) in entries {
                write_lp(out, key.as_bytes());
                write_canon_json(child, out)?;
            }
        },
    }
    Ok(())
}

/// Canonicalize a number: integers and integral floats normalize to one
/// `i128`-BE integer encoding (`1` ≡ `1.0` ≡ `-0.0`); other finite floats use
/// their IEEE-754 big-endian bits; non-finite floats are rejected.
#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn write_canon_number(
    number: &serde_json::Number,
    out: &mut Vec<u8>,
) -> Result<(), crate::error::ValidationError> {
    if let Some(int) = number.as_i64() {
        out.push(TAG_INT);
        out.extend_from_slice(&i128::from(int).to_be_bytes());
        return Ok(());
    }
    if let Some(uint) = number.as_u64() {
        out.push(TAG_INT);
        out.extend_from_slice(&i128::from(uint).to_be_bytes());
        return Ok(());
    }
    // Float (serde_json::Number is always finite, but guard defensively).
    let float = number.as_f64().ok_or_else(non_canonical_float)?;
    if !float.is_finite() {
        return Err(non_canonical_float());
    }
    // An integral float that fits in `i128` normalizes to the integer encoding,
    // so the same integer hashes identically however JSON spelled it (`5` ≡ `5.0`,
    // `-0.0` ≡ `0`). The bound must cover the whole `i64`/`u64` range (not just
    // 2^53), or an integer near 2^63 would diverge from its own float spelling:
    // `5e18 as i64` takes the integer path while `5e18_f64` would take the float
    // path. 2^127 is the `i128` limit; a larger integral float has no `i64`/`u64`
    // counterpart, so it stays a float (no spelling ambiguity to resolve).
    let i128_bound = 2.0_f64.powi(127);
    if float.fract() == 0.0 && float.abs() < i128_bound {
        out.push(TAG_INT);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "float is integral and |float| < 2^127 (the i128 bound), so the cast is exact"
        )]
        let as_int = float as i128;
        out.extend_from_slice(&as_int.to_be_bytes());
        return Ok(());
    }
    out.push(TAG_FLOAT);
    out.extend_from_slice(&float.to_be_bytes());
    Ok(())
}

impl Serialize for FieldValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for FieldValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::try_from_json(Value::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// Returns `true` only when `text` contains at least one `{{ … }}` pair that
/// has a `$` sigil somewhere between the opening `{{` and the closing `}}`.
///
/// Rules:
/// - Four consecutive braces (`{{{{`) are the escape sequence for a literal `{{`; they are skipped
///   without triggering detection.
/// - A `{{` with no matching `}}` never counts as an expression marker.
/// - A `{{ … }}` pair without a `$` between the delimiters is NOT an expression (e.g. `"use {{ and
///   }} in templates"`).
fn contains_expression_marker(text: &str) -> bool {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i + 1 < len {
        // Skip `{{{{` escape — represents a literal `{{`.
        if bytes[i] == b'{'
            && bytes[i + 1] == b'{'
            && i + 3 < len
            && bytes[i + 2] == b'{'
            && bytes[i + 3] == b'{'
        {
            i += 4;
            continue;
        }

        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Found `{{` — look for the matching `}}`.
            let start_inner = i + 2;
            let mut j = start_inner;
            while j + 1 < len {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    // Found `}}` — check for `$` in the interior.
                    let interior = &bytes[start_inner..j];
                    if interior.contains(&b'$') {
                        return true;
                    }
                    // No `$` — this pair is not an expression; resume after `}}`.
                    i = j + 2;
                    break;
                }
                j += 1;
            }
            if j + 1 >= len {
                // No closing `}}` found — not an expression.
                break;
            }
            continue;
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
        validate_json_keys(&value, &FieldPath::root(), 0)?;
        match FieldValue::try_from_json(value)? {
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

    /// Set a raw JSON value by string key. Validates nested object keys
    /// before insertion and returns `invalid_key` when any path segment
    /// violates [`FieldKey`] constraints.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::error::ValidationError`] when `key` or nested keys
    /// are invalid.
    ///
    /// # Example
    ///
    /// Use `.expect("known-good key")` in tests/migrations where the key is
    /// a static literal. For runtime input, propagate the error with `?`.
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
                .message(format!("try_set_raw: invalid key {key:?}: {e}"))
                .param("key", Value::String(key.to_owned()))
                .build()
        })?;
        let field_path = FieldPath::root().join(fk.clone());
        validate_json_keys(&value, &field_path, 0)?;
        self.0.insert(fk, FieldValue::try_from_json(value)?);
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

    /// Borrow the underlying ordered map.
    ///
    /// Used by the predicate-context builder to walk the value tree in
    /// lockstep with the schema field tree.
    #[inline]
    #[must_use]
    pub(crate) fn as_map(&self) -> &IndexMap<FieldKey, FieldValue> {
        &self.0
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

    /// Injective canonical byte encoding of this value store (insertion-order
    /// independent). Identical to the canon of the equivalent
    /// [`FieldValue::Object`], so the two content-address the same. See
    /// [`FieldValue::canonical_bytes`].
    ///
    /// # Errors
    ///
    /// Returns `secret.not_hashable` for a secret-bearing value, or
    /// `value.non_canonical_float` for a non-finite float.
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, crate::error::ValidationError> {
        let mut out = Vec::new();
        out.extend_from_slice(CANON_DOMAIN);
        out.extend_from_slice(&VALUE_CANON_VERSION.to_be_bytes());
        write_canon_object(&self.0, &mut out)?;
        Ok(out)
    }

    /// Content identifier — `blake3` over [`canonical_bytes`](Self::canonical_bytes).
    ///
    /// # Errors
    ///
    /// Propagates [`canonical_bytes`](Self::canonical_bytes).
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn content_id(&self) -> Result<ContentId, crate::error::ValidationError> {
        Ok(ContentId(blake3::hash(&self.canonical_bytes()?).into()))
    }
}

#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn validate_json_keys(
    value: &Value,
    path: &FieldPath,
    depth: u8,
) -> Result<(), crate::error::ValidationError> {
    if depth > MAX_VALUE_DEPTH {
        tracing::warn!(
            target: "nebula_schema::value",
            depth = %depth,
            path = %path,
            "value depth limit hit during validate_json_keys"
        );
        return Err(recursion_limit_error(path));
    }
    match value {
        Value::Object(map) => {
            if map.len() == 1 && map.get(EXPRESSION_KEY).is_some_and(Value::is_string) {
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
                validate_json_keys(child, &child_path, depth.saturating_add(1))?;
            }
            Ok(())
        },
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let item_path = path.clone().join(idx);
                validate_json_keys(item, &item_path, depth.saturating_add(1))?;
            }
            Ok(())
        },
        _ => Ok(()),
    }
}

#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn validate_json_object_depth(
    map: &Map<String, Value>,
    path: &FieldPath,
    depth: u8,
) -> Result<(), crate::error::ValidationError> {
    for child in map.values() {
        validate_json_depth(child, path, depth.saturating_add(1))?;
    }
    Ok(())
}

#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn validate_json_depth(
    value: &Value,
    path: &FieldPath,
    depth: u8,
) -> Result<(), crate::error::ValidationError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(recursion_limit_error(path));
    }
    match value {
        Value::Object(map) => validate_json_object_depth(map, path, depth),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let item_path = path.clone().join(index);
                validate_json_depth(item, &item_path, depth.saturating_add(1))?;
            }
            Ok(())
        },
        _ => Ok(()),
    }
}

fn recursion_limit_error(path: &FieldPath) -> crate::error::ValidationError {
    crate::error::ValidationError::builder("recursion_limit")
        .at(path.clone())
        .param("limit", serde_json::json!(MAX_VALUE_DEPTH))
        .message(format!(
            "value tree depth exceeds the {MAX_VALUE_DEPTH}-level limit at `{path}`"
        ))
        .build()
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

    // ── contains_expression_marker edge cases ────────────────────────────────

    #[test]
    fn no_dollar_in_braces_stays_literal() {
        // `{{ world }}` has no `$` → should be treated as a literal string.
        let v = FieldValue::from_json(json!("hello {{ world }}"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn multi_dollar_expr_is_expression() {
        // Both `$a` and `$b` are present → expression.
        let v = FieldValue::from_json(json!("{{ $a }} and {{ $b }}"));
        assert!(matches!(v, FieldValue::Expression(_)));
    }

    #[test]
    fn unclosed_braces_stays_literal() {
        // Opening `{{` but no closing `}}` → literal.
        let v = FieldValue::from_json(json!("text with {{ but no close"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn plain_text_stays_literal() {
        let v = FieldValue::from_json(json!("plain text"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn braces_with_no_dollar_stays_literal_new_heuristic() {
        // `{{ no_dollar }}` — balanced braces but no `$` → literal.
        let v = FieldValue::from_json(json!("{{ no_dollar }}"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn expr_wrapper_still_works() {
        // `$expr` wrapper is unconditional and must not be changed.
        let v = FieldValue::from_json(json!({"$expr": "anything"}));
        assert!(matches!(v, FieldValue::Expression(_)));
    }

    #[test]
    fn escaped_double_braces_stay_literal() {
        let v = FieldValue::from_json(json!("{{{{ x }}}}"));
        assert!(matches!(v, FieldValue::Literal(_)));
    }

    #[test]
    fn mode_like_object_stays_object() {
        let v = FieldValue::from_json(json!({"mode": "oauth2", "value": {"scope":"r"}}));
        assert!(matches!(v, FieldValue::Object(_)));
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
    fn try_set_raw_parses_expression_wrapper() {
        let mut vs = FieldValues::new();
        vs.try_set_raw("expr", json!({"$expr":"{{ $x }}"}))
            .expect("test-only known-good key");
        assert!(matches!(
            vs.get(&FieldKey::new("expr").unwrap()),
            Some(FieldValue::Expression(_))
        ));
    }

    /// Build a JSON object nested `depth` levels deep under `inner_key`.
    fn nested_object(depth: usize, inner_key: &str) -> Value {
        let mut current = json!({ "leaf": 1 });
        for _ in 0..depth {
            let mut wrapped = Map::with_capacity(1);
            wrapped.insert(inner_key.to_owned(), current);
            current = Value::Object(wrapped);
        }
        current
    }

    #[test]
    fn from_json_rejects_deeply_nested_object_with_recursion_limit() {
        // Wrap the nested object in another `{"top": ...}` so the input is a
        // top-level JSON object as required by `FieldValues::from_json`.
        let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
        let payload = json!({ "top": deep });
        let err = FieldValues::from_json(payload).expect_err("must reject");
        assert_eq!(err.code, "recursion_limit", "got: {}", err.message);
    }

    #[test]
    fn field_value_try_from_json_rejects_deeply_nested_input() {
        let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
        let err = FieldValue::try_from_json(deep).expect_err("must reject");
        assert_eq!(err.code, "recursion_limit");
    }

    #[test]
    fn field_value_deserialize_rejects_deeply_nested_input() {
        let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
        assert!(serde_json::from_value::<FieldValue>(deep).is_err());
    }

    #[test]
    fn field_value_try_from_json_checks_depth_inside_invalid_key_literals() {
        let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
        let err = FieldValue::try_from_json(json!({"bad-key": deep})).expect_err("must reject");
        assert_eq!(err.code, "recursion_limit");
    }

    #[test]
    fn field_value_from_json_preserves_over_deep_subtree_as_literal() {
        let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
        let parsed = FieldValue::from_json(deep);

        let key = FieldKey::new("n").unwrap();
        let mut current = &parsed;
        for _ in 0..=usize::from(MAX_VALUE_DEPTH) {
            let FieldValue::Object(map) = current else {
                panic!("expected object before the depth limit, got: {current:?}");
            };
            current = map.get(&key).expect("nested object should contain n");
        }
        assert!(
            matches!(current, FieldValue::Literal(Value::Object(_))),
            "expected over-depth subtree to be preserved as literal, got: {current:?}"
        );
    }

    #[test]
    fn from_json_accepts_at_recursion_limit() {
        // 60 < 64 — must stay valid.
        let ok = nested_object(60, "n");
        let payload = json!({ "top": ok });
        FieldValues::from_json(payload).expect("must accept under-limit nesting");
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

    // ── canonical_bytes / ContentId ──────────────────────────────────────────

    fn canon(value: &FieldValue) -> Vec<u8> {
        value.canonical_bytes().expect("canonicalizable")
    }

    fn fk(s: &str) -> FieldKey {
        FieldKey::new(s).unwrap()
    }

    /// Integers and integral floats share one encoding (`1` ≡ `1.0` ≡ `-0.0`),
    /// but a non-integral float is distinct. This is the `"1"`-vs-`"1.0"` dedup fix.
    #[test]
    fn canon_normalizes_integral_numbers() {
        let int = FieldValue::Literal(json!(1));
        let float = FieldValue::Literal(json!(1.0));
        let neg_zero = FieldValue::Literal(Value::from(-0.0_f64));
        let zero = FieldValue::Literal(json!(0));
        assert_eq!(canon(&int), canon(&float), "1 and 1.0 share a canon");
        assert_eq!(canon(&neg_zero), canon(&zero), "-0.0 and 0 share a canon");

        let frac = FieldValue::Literal(json!(1.5));
        assert_ne!(canon(&int), canon(&frac), "1 and 1.5 differ");
    }

    /// Length-prefixing prevents `["a","b"]` from aliasing `["ab"]`, and the
    /// per-variant tags keep a typed `List`/`Object` distinct from a JSON one.
    #[test]
    fn canon_is_injective_across_shapes() {
        let ab = FieldValue::List(vec![
            FieldValue::Literal(json!("a")),
            FieldValue::Literal(json!("b")),
        ]);
        let concat = FieldValue::List(vec![FieldValue::Literal(json!("ab"))]);
        assert_ne!(
            canon(&ab),
            canon(&concat),
            "length-prefix blocks concatenation alias"
        );

        let empty_list = FieldValue::List(vec![]);
        let empty_obj = FieldValue::Object(IndexMap::new());
        assert_ne!(
            canon(&empty_list),
            canon(&empty_obj),
            "list tag != object tag"
        );

        // A `Literal(json object)` (invalid-key path) is distinct from a typed
        // `Object`, even with the same logical content.
        let literal_obj = FieldValue::Literal(json!({"k": 1}));
        let mut typed = IndexMap::new();
        typed.insert(fk("k"), FieldValue::Literal(json!(1)));
        let typed_obj = FieldValue::Object(typed);
        assert_ne!(
            canon(&literal_obj),
            canon(&typed_obj),
            "JSON object tag != typed object tag"
        );
    }

    /// Object canon is independent of key insertion order.
    #[test]
    fn canon_object_key_order_invariant() {
        let mut forward = IndexMap::new();
        forward.insert(fk("alpha"), FieldValue::Literal(json!(1)));
        forward.insert(fk("beta"), FieldValue::Literal(json!(2)));
        forward.insert(fk("gamma"), FieldValue::Literal(json!(3)));

        let mut reversed = IndexMap::new();
        reversed.insert(fk("gamma"), FieldValue::Literal(json!(3)));
        reversed.insert(fk("beta"), FieldValue::Literal(json!(2)));
        reversed.insert(fk("alpha"), FieldValue::Literal(json!(1)));

        assert_eq!(
            canon(&FieldValue::Object(forward)),
            canon(&FieldValue::Object(reversed)),
            "key order must not affect the canon"
        );
    }

    /// A secret value has no canonical form — it must not enter a content hash.
    #[test]
    fn canon_rejects_secret() {
        let secret = FieldValue::SecretLiteral(SecretValue::String(
            crate::secret::SecretString::new("hunter2".to_owned()),
        ));
        let err = secret
            .canonical_bytes()
            .expect_err("secrets are not hashable");
        assert_eq!(err.code, "secret.not_hashable");
        assert!(
            secret.content_id().is_err(),
            "content_id propagates the rejection"
        );
    }

    #[test]
    fn content_id_is_deterministic_and_hex() {
        let value = FieldValue::Object({
            let mut map = IndexMap::new();
            map.insert(fk("k"), FieldValue::Literal(json!("v")));
            map
        });
        let id_a = value.content_id().unwrap();
        let id_b = value.content_id().unwrap();
        assert_eq!(id_a, id_b, "same value, same id");
        let hex = id_a.to_string();
        assert_eq!(hex.len(), 64, "32 bytes render as 64 hex chars");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// A `FieldValues` store canonicalizes identically to the equivalent typed
    /// `FieldValue::Object`.
    #[test]
    fn field_values_canon_matches_equivalent_object() {
        let values = FieldValues::from_json(json!({"a": 1, "b": "x"})).unwrap();
        let mut map = IndexMap::new();
        map.insert(fk("a"), FieldValue::Literal(json!(1)));
        map.insert(fk("b"), FieldValue::Literal(json!("x")));
        let object = FieldValue::Object(map);
        assert_eq!(
            values.canonical_bytes().unwrap(),
            canon(&object),
            "a value store and the equivalent object share a canon"
        );
    }

    /// The normalization invariant must hold across the WHOLE i64/u64 range, not
    /// only |x| < 2^53: an integer near 2^63 spelled as int vs as float must
    /// still share a canon. (Guards against the off-by-`<` at the 2^53 bound.)
    #[test]
    fn canon_normalizes_large_integers() {
        for n in [
            9_007_199_254_740_992_i64,     // 2^53 (the old boundary, exclusive)
            9_007_199_254_740_994_i64,     // 2^53 + 2 (next exact f64 integer above 2^53)
            4_503_599_627_370_497_i64,     // odd, within 2^53
            1_152_921_504_606_846_976_i64, // 2^60
        ] {
            // Only test values that round-trip exactly through f64 (so the float
            // spelling denotes the same integer).
            #[expect(clippy::cast_precision_loss, reason = "checked exact below")]
            let as_float = n as f64;
            #[expect(clippy::cast_possible_truncation, reason = "checked exact below")]
            let back = as_float as i64;
            if back != n {
                continue;
            }
            let int = FieldValue::Literal(json!(n));
            let float = FieldValue::Literal(Value::from(as_float));
            assert_eq!(
                canon(&int),
                canon(&float),
                "{n} as int and as float must share a canon"
            );
        }
    }

    /// A secret nested inside a container still rejects (recursion propagates it).
    #[test]
    fn canon_rejects_nested_secret() {
        let secret = || {
            FieldValue::SecretLiteral(SecretValue::String(crate::secret::SecretString::new(
                "s".to_owned(),
            )))
        };
        let in_list = FieldValue::List(vec![FieldValue::Literal(json!(1)), secret()]);
        let mut map = IndexMap::new();
        map.insert(fk("k"), secret());
        let in_object = FieldValue::Object(map);
        let in_mode = FieldValue::Mode {
            mode: fk("m"),
            value: Some(Box::new(secret())),
        };
        for value in [in_list, in_object, in_mode] {
            assert_eq!(
                value
                    .canonical_bytes()
                    .expect_err("nested secret rejects")
                    .code,
                "secret.not_hashable"
            );
        }
    }

    /// `Mode` discriminator: `None`, `Some`, and a different mode key are all
    /// distinct (the presence byte 0/1 and the mode key matter).
    #[test]
    fn canon_mode_variants_are_distinct() {
        let none = FieldValue::Mode {
            mode: fk("m"),
            value: None,
        };
        let some = FieldValue::Mode {
            mode: fk("m"),
            value: Some(Box::new(FieldValue::Literal(json!(0)))),
        };
        let other_key = FieldValue::Mode {
            mode: fk("n"),
            value: None,
        };
        assert_ne!(canon(&none), canon(&some));
        assert_ne!(canon(&none), canon(&other_key));
        assert_ne!(canon(&some), canon(&other_key));
    }

    /// Empty string, list, and object are mutually distinct and non-degenerate.
    #[test]
    fn canon_empty_containers_are_distinct() {
        let empty_string = FieldValue::Literal(json!(""));
        let empty_list = FieldValue::List(vec![]);
        let empty_object = FieldValue::Object(IndexMap::new());
        let canons = [
            canon(&empty_string),
            canon(&empty_list),
            canon(&empty_object),
        ];
        for (i, a) in canons.iter().enumerate() {
            for b in &canons[i + 1..] {
                assert_ne!(a, b, "empty containers must not alias");
            }
        }
    }

    /// A `Literal(JSON array)` (invalid-key path) must not collide with a typed
    /// `List` of the same items (`TAG_JSON_ARRAY` != `TAG_LIST`).
    #[test]
    fn canon_literal_array_distinct_from_typed_list() {
        let literal = FieldValue::Literal(json!([1, 2]));
        let typed = FieldValue::List(vec![
            FieldValue::Literal(json!(1)),
            FieldValue::Literal(json!(2)),
        ]);
        assert_ne!(canon(&literal), canon(&typed));
    }

    #[test]
    fn content_id_separates_distinct_values() {
        let a = FieldValues::from_json(json!({"n": 1})).unwrap();
        let b = FieldValues::from_json(json!({"n": 2})).unwrap();
        assert_ne!(
            a.content_id().unwrap(),
            b.content_id().unwrap(),
            "distinct values must have distinct content ids"
        );
    }

    /// Multi-byte varint: a 200-element string length encodes as LEB128
    /// `[0xC8, 0x01]` (200 = 0x48 | continuation, then 1).
    #[test]
    fn canon_varint_is_multibyte_for_large_lengths() {
        let long = "a".repeat(200);
        let bytes = canon(&FieldValue::Literal(json!(long)));
        // After domain (16) + version (2) + TAG_STRING (1) comes the varint length.
        assert_eq!(
            &bytes[19..21],
            &[0xC8, 0x01],
            "200 encodes as a 2-byte varint"
        );
    }

    /// Golden bytes — freeze the exact on-the-wire content-address format so any
    /// silent re-keying (tag value, framing order, version) is a loud failure.
    #[test]
    fn canon_golden_bytes() {
        let value = FieldValues::from_json(json!({"a": 1})).unwrap();
        let bytes = value.canonical_bytes().unwrap();

        let mut expected = b"nbschema-value-v".to_vec();
        expected.extend_from_slice(&[0x00, 0x01]); // VALUE_CANON_VERSION = 1
        expected.push(0x07); // TAG_OBJECT
        expected.push(0x01); // entry count (varint 1)
        expected.extend_from_slice(&[0x01, b'a']); // key "a" (len 1 + bytes)
        expected.push(0x03); // TAG_INT
        expected.extend_from_slice(&1_i128.to_be_bytes()); // value 1 (i128 BE)

        assert_eq!(bytes, expected, "canonical format must not drift");
        assert_eq!(
            &bytes[16..18],
            &VALUE_CANON_VERSION.to_be_bytes(),
            "the version prefix is pinned"
        );
    }
}
