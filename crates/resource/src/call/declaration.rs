//! The journal declaration the runtime derives from an operation: the
//! operation key rules, the canonical request, the developer idempotency
//! key part and the provider idempotency key a unit presents.
//!
//! An [`Operation`](super::Operation) declares only its key, version,
//! effect, key window and whether its output is recorded; everything the
//! execution owner needs besides ([`JournalIntent`](super::journal::JournalIntent))
//! is computed here, so no author writes a canonicalization or an
//! occurrence label by hand.

use std::{fmt, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nebula_core::ResourceKey;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::{
    canonical::{self, CanonicalError},
    cost::Effect,
    error::OperationError,
};
use crate::error::ErrorKind;

/// Longest operation key, in bytes.
const MAX_OPERATION_KEY_LEN: usize = 64;
/// Longest developer idempotency key part, in bytes.
const MAX_KEY_PART_LEN: usize = 256;
/// Longest provider idempotency key, in bytes.
const MAX_IDEMPOTENCY_KEY_LEN: usize = 64;
/// Largest canonical request, in bytes.
pub(super) const MAX_CANONICAL_REQUEST_LEN: usize = 1024 * 1024;
/// Largest output an owner records, in bytes: the operation ledger's
/// evidence cap. A larger output is recorded digest-only.
pub(super) const MAX_RECORDED_OUTPUT_LEN: usize = 1024 * 1024;

/// Domain separation of a library unit's local idempotency key.
const LOCAL_KEY_DOMAIN: &str = "nebula.idempotency.local.v1";

/// Whether `key` is an operation key: 1 to 64 bytes of ASCII alphanumerics,
/// `_`, `-` and `.`, starting and ending with an alphanumeric — the charset
/// of a domain key. Also the rule for a session name.
pub(crate) const fn is_valid_operation_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_OPERATION_KEY_LEN {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.') {
            return false;
        }
        index += 1;
    }
    true
}

/// Whether an operation's constants are well formed: a valid
/// [`KEY`](super::Operation::KEY), a `VERSION` of at least 1, and a
/// non-zero `KEY_WINDOW` for an [`Idempotent`](Effect::Idempotent) one.
/// Checked at compile time by every `submit`, and again at submit time.
pub(crate) const fn is_valid_declaration(
    key: &str,
    version: u32,
    effect: Effect,
    key_window: Duration,
) -> bool {
    is_valid_operation_key(key)
        && version >= 1
        && !(matches!(effect, Effect::Idempotent) && key_window.is_zero())
}

/// The submit-time refusal of a declaration that breaks
/// [`is_valid_declaration`], naming the broken rule.
pub(super) fn check_declaration(
    key: &str,
    version: u32,
    effect: Effect,
    key_window: Duration,
) -> Result<(), OperationError> {
    if !is_valid_operation_key(key) {
        return Err(OperationError::new(
            ErrorKind::Permanent,
            "operation key must be 1..=64 bytes of [A-Za-z0-9_.-], starting and ending alphanumeric",
        ));
    }
    if version == 0 {
        return Err(OperationError::new(
            ErrorKind::Permanent,
            "operation version must be at least 1",
        ));
    }
    if effect == Effect::Idempotent && key_window.is_zero() {
        return Err(OperationError::new(
            ErrorKind::Permanent,
            "an idempotent operation's key window must be non-zero",
        ));
    }
    Ok(())
}

/// Checks the developer part of the provider idempotency key: 1 to 256
/// bytes of visible ASCII (`0x21..=0x7E`).
pub(super) fn check_key_part(part: &str) -> Result<(), OperationError> {
    let bytes = part.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= MAX_KEY_PART_LEN
        && bytes.iter().all(|byte| (0x21..=0x7E).contains(byte));
    if valid {
        Ok(())
    } else {
        Err(OperationError::new(
            ErrorKind::Permanent,
            "idempotency key part must be 1..=256 bytes of visible ASCII",
        ))
    }
}

/// The canonical request of `request`: its JSON with every object's keys
/// sorted, re-emitted without whitespace, 1 byte to 1 MiB.
///
/// Independent of field declaration order and of `serde_json`'s map
/// ordering (`preserve_order` or not): the owner digests these bytes to
/// tell a resumed effect from a different one. Written straight from the
/// request's `Serialize` impl, never materialized as a JSON value: an
/// object that writes one key twice (a `#[serde(flatten)]` collision) is
/// refused rather than keeping its last member — two different requests
/// would otherwise share bytes — and a request is refused as soon as its
/// canonical form outgrows the cap.
pub(super) fn canonical_json<T: Serialize + ?Sized>(
    request: &T,
) -> Result<Vec<u8>, OperationError> {
    let detail = match canonical::to_canonical(request) {
        Ok(canonical) if !canonical.is_empty() => return Ok(canonical),
        Ok(_) | Err(CanonicalError::TooLarge) => "canonical request must be 1 byte to 1 MiB",
        Err(CanonicalError::DuplicateKey) => "operation request has an object with a duplicate key",
        // `to_canonical` writes a non-finite float `null`, as `serde_json`
        // does for the request a provider receives: it never refuses one.
        Err(CanonicalError::Unserializable | CanonicalError::NonFiniteFloat) => {
            "operation request does not serialize to JSON"
        },
    };
    Err(OperationError::new(ErrorKind::Permanent, detail))
}

/// The provider idempotency key of a unit no owner records that declared a
/// developer key `part`: base64url (43 bytes) of the SHA-256 of the framed
/// domain, resource key, operation key, version and part.
///
/// Deterministic, so every attempt, retry and resubmission of the same call
/// presents the same key; scoped to the resource and the operation's
/// version, so two operations never collide on one part.
pub(super) fn local_idempotency_key(
    resource: &ResourceKey,
    operation: &str,
    version: u32,
    part: &str,
) -> Result<IdempotencyKey, OperationError> {
    fn frame(hasher: &mut Sha256, bytes: &[u8]) {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    let mut hasher = Sha256::new();
    frame(&mut hasher, LOCAL_KEY_DOMAIN.as_bytes());
    frame(&mut hasher, resource.as_str().as_bytes());
    frame(&mut hasher, operation.as_bytes());
    frame(&mut hasher, &version.to_be_bytes());
    frame(&mut hasher, part.as_bytes());
    IdempotencyKey::new(&URL_SAFE_NO_PAD.encode(hasher.finalize()))
}

/// The provider idempotency key a unit presents: base64url, 1 to 64 bytes.
///
/// For a unit an execution owner records, the key the owner derived and
/// recorded before its first attempt; for another unit that declared a
/// developer key part, a key derived locally from the resource, the
/// operation and the part. The same for every attempt, retry and resume.
/// It is not a secret — [`Display`](fmt::Display) prints it — but it is not
/// authority either: holding one grants no provider call.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdempotencyKey {
    bytes: [u8; MAX_IDEMPOTENCY_KEY_LEN],
    len: u8,
}

impl IdempotencyKey {
    /// A key of `key`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Permanent`] when `key` is empty, longer than 64 bytes,
    /// or not base64url (`[A-Za-z0-9_-]`).
    pub fn new(key: &str) -> Result<Self, OperationError> {
        let raw = key.as_bytes();
        let valid = !raw.is_empty()
            && raw.len() <= MAX_IDEMPOTENCY_KEY_LEN
            && raw
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
        let Ok(len) = u8::try_from(raw.len()) else {
            return Err(invalid_idempotency_key());
        };
        if !valid {
            return Err(invalid_idempotency_key());
        }
        let mut bytes = [0; MAX_IDEMPOTENCY_KEY_LEN];
        bytes[..raw.len()].copy_from_slice(raw);
        Ok(Self { bytes, len })
    }

    /// The key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only base64url bytes are ever stored, so the prefix is UTF-8.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

fn invalid_idempotency_key() -> OperationError {
    OperationError::new(
        ErrorKind::Permanent,
        "idempotency key must be 1..=64 bytes of base64url",
    )
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for IdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("IdempotencyKey")
            .field(&self.as_str())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        collections::{BTreeMap, HashMap},
        time::Duration,
    };

    use nebula_core::resource_key;
    use serde::Serialize;
    use serde_json::{Value, value::RawValue};

    use super::{
        Effect, IdempotencyKey, MAX_CANONICAL_REQUEST_LEN, canonical_json, check_declaration,
        check_key_part, is_valid_declaration, is_valid_operation_key, local_idempotency_key,
    };
    use crate::ErrorKind;

    #[test]
    fn operation_keys_follow_the_domain_key_charset() {
        for valid in [
            "a",
            "mail.send",
            "http.post.keyed",
            "A_b-c.9",
            &"x".repeat(64),
        ] {
            assert!(is_valid_operation_key(valid), "{valid:?}");
        }
        for invalid in [
            "",
            ".send",
            "send.",
            "-send",
            "send_",
            "has space",
            "a/b",
            "uni\u{e9}",
            &"x".repeat(65),
        ] {
            assert!(!is_valid_operation_key(invalid), "{invalid:?}");
        }
        // Usable in a const context, as every `submit` does.
        const { assert!(is_valid_operation_key("http.get")) };
    }

    #[test]
    fn declarations_need_a_version_and_an_idempotent_window() {
        let day = Duration::from_hours(24);
        assert!(is_valid_declaration("op", 1, Effect::Write, day));
        assert!(is_valid_declaration("op", 1, Effect::Write, Duration::ZERO));
        assert!(!is_valid_declaration("op", 0, Effect::Write, day));
        assert!(!is_valid_declaration(
            "op",
            1,
            Effect::Idempotent,
            Duration::ZERO
        ));
        assert!(!is_valid_declaration("bad key", 1, Effect::Read, day));
        for (key, version, effect, window) in [
            ("bad key", 1, Effect::Read, day),
            ("op", 0, Effect::Write, day),
            ("op", 1, Effect::Idempotent, Duration::ZERO),
        ] {
            let error = check_declaration(key, version, effect, window).expect_err("refused");
            assert_eq!(*error.kind(), ErrorKind::Permanent);
        }
        check_declaration("op", 1, Effect::Idempotent, day).expect("valid");
    }

    #[test]
    fn key_parts_are_visible_ascii_and_bounded() {
        assert!(check_key_part("~!order#1").is_ok());
        assert!(check_key_part(&"x".repeat(256)).is_ok());
        assert!(check_key_part(&"x".repeat(257)).is_err());
        assert!(check_key_part("tab\t").is_err());
        assert!(check_key_part("a b").is_err());
        assert!(check_key_part("").is_err());
    }

    #[derive(Serialize)]
    struct Forward {
        alpha: u8,
        beta: Nested,
    }

    #[derive(Serialize)]
    struct Backward {
        beta: Nested,
        alpha: u8,
    }

    #[derive(Serialize)]
    struct Nested {
        zeta: BTreeMap<String, u8>,
        eta: Vec<&'static str>,
    }

    fn nested() -> Nested {
        Nested {
            zeta: BTreeMap::from([("b".to_owned(), 2), ("a".to_owned(), 1)]),
            eta: vec!["y", "x"],
        }
    }

    #[test]
    fn the_canonical_request_ignores_field_and_map_order() {
        let forward = canonical_json(&Forward {
            alpha: 1,
            beta: nested(),
        })
        .expect("canonical");
        let backward = canonical_json(&Backward {
            beta: nested(),
            alpha: 1,
        })
        .expect("canonical");
        assert_eq!(forward, backward);
        assert_eq!(
            String::from_utf8(forward).expect("utf-8"),
            r#"{"alpha":1,"beta":{"eta":["y","x"],"zeta":{"a":1,"b":2}}}"#,
            "sorted keys at every depth; arrays keep their order"
        );
        let unordered =
            serde_json::json!({ "b": { "d": 1, "c": [ { "f": 1, "e": 2 } ] }, "a": null });
        assert_eq!(
            canonical_json(&unordered).expect("canonical"),
            br#"{"a":null,"b":{"c":[{"e":2,"f":1}],"d":1}}"#
        );
        assert_eq!(canonical_json(&()).expect("unit"), b"null");
    }

    #[test]
    fn the_canonical_request_is_capped_at_one_mebibyte() {
        let fits = "x".repeat(1024 * 1024 - 2);
        assert_eq!(
            canonical_json(&fits).expect("fits").len(),
            1024 * 1024,
            "the quotes count"
        );
        let over = "x".repeat(1024 * 1024 - 1);
        let error = canonical_json(&over).expect_err("over the cap");
        assert_eq!(*error.kind(), ErrorKind::Permanent);
        assert_eq!(error.detail(), "canonical request must be 1 byte to 1 MiB");
    }

    /// The canonicalization before the streaming serializer: the request
    /// materialized as a JSON value, re-emitted with sorted keys. `None`
    /// when it does not serialize.
    fn legacy_canonical<T: Serialize + ?Sized>(request: &T) -> Option<Vec<u8>> {
        fn emit_sorted(value: &Value, out: &mut Vec<u8>) {
            match value {
                Value::Object(map) => {
                    let mut entries: Vec<_> = map.iter().collect();
                    entries.sort_unstable_by_key(|(key, _)| *key);
                    out.push(b'{');
                    for (index, (key, item)) in entries.into_iter().enumerate() {
                        if index > 0 {
                            out.push(b',');
                        }
                        serde_json::to_writer(&mut *out, key).expect("key");
                        out.push(b':');
                        emit_sorted(item, out);
                    }
                    out.push(b'}');
                },
                Value::Array(items) => {
                    out.push(b'[');
                    for (index, item) in items.iter().enumerate() {
                        if index > 0 {
                            out.push(b',');
                        }
                        emit_sorted(item, out);
                    }
                    out.push(b']');
                },
                scalar => serde_json::to_writer(&mut *out, scalar).expect("scalar"),
            }
        }
        let value = serde_json::to_value(request).ok()?;
        let mut out = Vec::new();
        emit_sorted(&value, &mut out);
        Some(out)
    }

    /// Asserts the canonical bytes of `request` are the legacy ones, or
    /// both refuse it.
    fn same_as_legacy<T: Serialize + ?Sized>(label: &str, request: &T) {
        match (canonical_json(request), legacy_canonical(request)) {
            (Ok(canonical), Some(legacy)) => assert_eq!(
                String::from_utf8_lossy(&canonical),
                String::from_utf8_lossy(&legacy),
                "{label}"
            ),
            (Err(error), None) => assert_eq!(
                error.detail(),
                "operation request does not serialize to JSON",
                "{label}"
            ),
            (canonical, legacy) => {
                panic!("{label}: canonical {canonical:?} but legacy {legacy:?}")
            },
        }
    }

    #[derive(Serialize)]
    enum Shape {
        Unit,
        Newtype(i32),
        Tuple(u8, &'static str),
        Struct { zed: bool, alpha: Option<u8> },
    }

    #[derive(Serialize)]
    struct UnitStruct;

    #[expect(
        clippy::empty_structs_with_brackets,
        reason = "serde writes a braced empty struct as `{}`, a unit struct as `null`"
    )]
    #[derive(Serialize)]
    struct Empty {}

    #[derive(Serialize)]
    struct Pair(i8, char);

    #[derive(Serialize)]
    struct Meters(f64);

    /// Serializes through `serialize_bytes`, not as a sequence.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Bytes(&'static [u8]);

    /// Serializes through `collect_str`, its text from `Display`.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Displayed(&'static str);

    impl std::fmt::Display for Displayed {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl Serialize for Displayed {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    /// A map keyed by floats, which a `BTreeMap` cannot hold.
    struct FloatKeys;

    impl Serialize for FloatKeys {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap as _;
            let mut map = serializer.serialize_map(Some(5))?;
            map.serialize_entry(&1.5_f64, &0)?;
            map.serialize_entry(&-0.0_f64, &1)?;
            map.serialize_entry(&1e300_f64, &2)?;
            map.serialize_entry(&0.1_f32, &3)?;
            map.serialize_entry(&Meters(2.0), &4)?;
            map.end()
        }
    }

    /// A map keyed by a NaN, which no JSON key can spell.
    struct NonFiniteKey;

    impl Serialize for NonFiniteKey {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap as _;
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(&f64::NAN, &0)?;
            map.end()
        }
    }

    impl Serialize for Bytes {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(self.0)
        }
    }

    #[derive(Serialize)]
    struct Flattened {
        zulu: u8,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
        alpha: &'static str,
    }

    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    enum KeyVariant {
        Beta,
        Alpha,
    }

    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    struct KeyNewtype(u16);

    #[test]
    fn the_canonical_bytes_are_the_legacy_bytes() {
        let raw: Box<RawValue> =
            RawValue::from_string(r#" { "z" : [ 1 , { "y": 2, "x": 1 } ], "a": "é" } "#.into())
                .expect("raw");
        same_as_legacy("null", &Value::Null);
        same_as_legacy("unit", &());
        same_as_legacy("unit struct", &UnitStruct);
        same_as_legacy("empty struct", &Empty {});
        same_as_legacy("empty array", &Vec::<u8>::new());
        same_as_legacy("empty map", &BTreeMap::<String, u8>::new());
        same_as_legacy("empty hash map", &HashMap::<String, u8>::new());
        same_as_legacy("bools", &(true, false));
        same_as_legacy(
            "integer extremes",
            &(
                i64::MIN,
                i64::MAX,
                u64::MAX,
                i8::MIN,
                u8::MAX,
                0_i32,
                -1_i16,
            ),
        );
        same_as_legacy(
            "128-bit in range",
            &(i128::from(i64::MIN), u128::from(u64::MAX)),
        );
        same_as_legacy("i128 out of range", &i128::MIN);
        same_as_legacy("u128 out of range", &u128::MAX);
        same_as_legacy(
            "floats",
            &(
                0.1_f64,
                -0.0_f64,
                1e300_f64,
                5e-324_f64,
                1.0_f64,
                123_456.789_f64,
            ),
        );
        same_as_legacy("f32 widened", &(0.1_f32, 3.5_f32, f32::MAX));
        same_as_legacy(
            "non-finite floats",
            &(f64::NAN, f64::INFINITY, f32::NEG_INFINITY),
        );
        same_as_legacy("newtype float", &Meters(2.5));
        same_as_legacy(
            "escapes",
            &"quote \" backslash \\ newline \n tab \t nul \u{0} unit \u{1f} del \u{7f} </script>",
        );
        same_as_legacy("unicode", &"h\u{e9}llo \u{2603} \u{1f600} \u{fffd}");
        same_as_legacy("chars", &('a', '\n', '\u{1f600}', '"'));
        same_as_legacy("bytes", &Bytes(b"\x00\x7f\xff"));
        same_as_legacy("empty bytes", &Bytes(b""));
        same_as_legacy("tuple struct", &Pair(-3, 'x'));
        same_as_legacy(
            "enum variants",
            &vec![
                Shape::Unit,
                Shape::Newtype(-7),
                Shape::Tuple(1, "t"),
                Shape::Struct {
                    zed: true,
                    alpha: None,
                },
            ],
        );
        same_as_legacy("options", &(Some(1_u8), None::<u8>, Some(Some("x"))));
        same_as_legacy(
            "nested maps",
            &serde_json::json!({
                "b": { "d": [ { "f": 1, "e": [] }, {} ], "c": null },
                "a": { "z": { "y": { "x": "deep" } } },
                "": "empty key",
                "\u{e9}": "unicode key",
                "Z": "upper",
                "z": "lower",
                "quote\"key": "escaped key",
            }),
        );
        same_as_legacy(
            "hash map order",
            &HashMap::from([("k3", 3), ("k1", 1), ("k2", 2), ("k10", 10)]),
        );
        same_as_legacy(
            "integer keys",
            &BTreeMap::from([(10_i64, "ten"), (-2, "minus two"), (3, "three")]),
        );
        same_as_legacy("bool keys", &BTreeMap::from([(true, 1), (false, 0)]));
        same_as_legacy("char keys", &BTreeMap::from([('b', 1), ('a', 0)]));
        same_as_legacy(
            "unit variant keys",
            &BTreeMap::from([(KeyVariant::Beta, 1), (KeyVariant::Alpha, 0)]),
        );
        same_as_legacy(
            "newtype keys",
            &BTreeMap::from([(KeyNewtype(20), 1), (KeyNewtype(3), 0)]),
        );
        same_as_legacy("tuple keys", &BTreeMap::from([((1, 2), "pair")]));
        same_as_legacy("option keys", &BTreeMap::from([(Some(1), "some")]));
        same_as_legacy("float keys", &FloatKeys);
        same_as_legacy(
            "displayed keys and values",
            &BTreeMap::from([
                (Displayed("b\n\u{e9}"), Displayed("value \"quoted\"")),
                (Displayed(""), Displayed("")),
                (Displayed("a"), Displayed("\u{1f600}")),
            ]),
        );
        same_as_legacy("unit keys", &BTreeMap::from([((), 1)]));
        same_as_legacy("bytes keys", &BTreeMap::from([(Bytes(b"k"), 1)]));
        same_as_legacy("sequence keys", &BTreeMap::from([(vec![1], 1)]));
        same_as_legacy("non-finite float keys", &NonFiniteKey);
        same_as_legacy("raw value", &raw);
        for text in [
            "null",
            "true",
            " false ",
            "0",
            "-0",
            "-0.0",
            "1e2",
            "1E-2",
            "-12345678901234567890",
            "18446744073709551615",
            "18446744073709551616",
            "123456789012345678901234567890",
            "0.1",
            "5e-324",
            "1.7976931348623157e308",
            r#""""#,
            r#""é\n\t\"\\\/😀\u0000\u001f\u007f""#,
            "[]",
            "{}",
            " [ 1 , [ ] , { } , [ [ null ] ] ] ",
            r#"{ "b" : [ { "d" : 1 , "c" : 2 } ] , "a" : { } , "" : "" , "é" : 1 , "Z" : 0 }"#,
            r#"[{"k":1},{"k":1}]"#,
        ] {
            let raw: Box<RawValue> = RawValue::from_string(text.into()).expect("raw");
            same_as_legacy(text, &raw);
            same_as_legacy(text, &BTreeMap::from([("z", &raw), ("a", &raw)]));
            same_as_legacy(text, &vec![raw.clone(), raw]);
        }
        same_as_legacy(
            "flatten without collision",
            &Flattened {
                zulu: 1,
                extra: BTreeMap::from([("mid".to_owned(), serde_json::json!({ "b": 1, "a": 2 }))]),
                alpha: "a",
            },
        );
        same_as_legacy(
            "structs at every depth",
            &Forward {
                alpha: 9,
                beta: nested(),
            },
        );
    }

    #[derive(Serialize)]
    struct Collides {
        id: u8,
        #[serde(flatten)]
        inner: Inner,
    }

    #[derive(Serialize)]
    struct Inner {
        id: u8,
    }

    /// Writes the field `key` twice, as a hand-written impl can.
    struct Twice(&'static str);

    impl Serialize for Twice {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct as _;
            let mut state = serializer.serialize_struct("Twice", 2)?;
            state.serialize_field(self.0, &1)?;
            state.serialize_field(self.0, &2)?;
            state.end()
        }
    }

    #[test]
    fn a_duplicate_key_is_refused_not_overwritten() {
        let collides = Collides {
            id: 1,
            inner: Inner { id: 2 },
        };
        assert_eq!(
            legacy_canonical(&collides).as_deref(),
            Some(&br#"{"id":2}"#[..]),
            "the materialized value kept only the last member"
        );
        for refused in [
            canonical_json(&collides),
            canonical_json(&Twice("secret-field")),
            canonical_json(&(serde_json::json!({}), vec![Twice("deep")])),
            canonical_json(&BTreeMap::from([("outer", Twice("nested"))])),
        ] {
            let error = refused.expect_err("duplicate key");
            assert_eq!(*error.kind(), ErrorKind::Permanent);
            assert_eq!(
                error.detail(),
                "operation request has an object with a duplicate key"
            );
            assert!(!error.to_string().contains("secret"), "{error}");
        }
    }

    #[test]
    fn a_duplicate_key_in_raw_json_is_refused_not_overwritten() {
        let raw = |text: &str| RawValue::from_string(text.into()).expect("raw");
        let collides = raw(r#"{"id":1,"id":2}"#);
        assert_eq!(
            legacy_canonical(&collides),
            legacy_canonical(&raw(r#"{"id":2}"#)),
            "the parsed value kept only the last member"
        );
        assert_eq!(
            canonical_json(&raw(r#"{"id":2}"#)).expect("no duplicate"),
            br#"{"id":2}"#
        );
        for refused in [
            collides,
            raw(r#"{"secret":1,"secret":1}"#),
            raw(r#"[0,{"a":{"deep":[],"deep":{}}}]"#),
            raw(r#"{"b":1,"a":2,"b":3}"#),
        ] {
            let error = canonical_json(&refused).expect_err("duplicate key");
            assert_eq!(*error.kind(), ErrorKind::Permanent);
            assert_eq!(
                error.detail(),
                "operation request has an object with a duplicate key"
            );
            assert!(!error.to_string().contains("secret"), "{error}");
            let nested = BTreeMap::from([("outer", vec![refused])]);
            assert_eq!(
                canonical_json(&nested)
                    .expect_err("nested duplicate")
                    .detail(),
                "operation request has an object with a duplicate key"
            );
        }
    }

    /// Serializes as `serde_json`'s `RawValue` does, without its check that
    /// the text is JSON.
    struct UncheckedRaw<T>(T);

    impl<T: Serialize> Serialize for UncheckedRaw<T> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct as _;
            const TOKEN: &str = "$serde_json::private::RawValue";
            let mut state = serializer.serialize_struct(TOKEN, 1)?;
            state.serialize_field(TOKEN, &self.0)?;
            state.end()
        }
    }

    #[test]
    fn raw_json_over_the_cap_is_refused_as_it_is_parsed() {
        // `[0,0,…]` is two bytes per element, twice what the cap holds.
        let zeros = "0,".repeat(MAX_CANONICAL_REQUEST_LEN);
        let valid = raw_text_of(&format!("[{zeros}0]"));
        let error = canonical_json(&valid).expect_err("over the cap");
        assert_eq!(error.detail(), "canonical request must be 1 byte to 1 MiB");

        // The text past the cap is never read: its broken tail is not
        // reached, so the refusal is the cap's, not a parse error.
        let error =
            canonical_json(&UncheckedRaw(format!("[{zeros} not JSON"))).expect_err("over the cap");
        assert_eq!(error.detail(), "canonical request must be 1 byte to 1 MiB");

        // Whitespace is not output: a padded value that fits is kept.
        let padded = raw_text_of(&format!("{}[1]", " ".repeat(2 * MAX_CANONICAL_REQUEST_LEN)));
        assert_eq!(canonical_json(&padded).expect("fits"), b"[1]");

        for broken in ["[1", "1 2", "{\"a\":}", ""] {
            assert_eq!(
                canonical_json(&UncheckedRaw(broken))
                    .expect_err("not JSON")
                    .detail(),
                "operation request does not serialize to JSON",
                "{broken}"
            );
        }
        assert_eq!(
            canonical_json(&UncheckedRaw(7_u8))
                .expect_err("not text")
                .detail(),
            "operation request does not serialize to JSON"
        );
    }

    fn raw_text_of(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).expect("raw")
    }

    /// An array of `len` zeros that counts the elements serialized.
    struct Counted {
        len: usize,
        serialized: Cell<usize>,
    }

    impl Serialize for Counted {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq as _;
            let mut seq = serializer.serialize_seq(Some(self.len))?;
            for _ in 0..self.len {
                self.serialized.set(self.serialized.get() + 1);
                seq.serialize_element(&0_u8)?;
            }
            seq.end()
        }
    }

    #[test]
    fn the_cap_stops_the_serialization_as_soon_as_it_is_crossed() {
        // `[0,0,…]` is two bytes per element: the cap holds about half a
        // million, far fewer than the request has.
        let huge = Counted {
            len: 64 * 1024 * 1024,
            serialized: Cell::new(0),
        };
        let error = canonical_json(&huge).expect_err("over the cap");
        assert_eq!(error.detail(), "canonical request must be 1 byte to 1 MiB");
        let serialized = huge.serialized.get();
        assert!(
            serialized <= MAX_CANONICAL_REQUEST_LEN / 2 + 1,
            "{serialized} elements serialized before the refusal"
        );

        // A string that cannot fit is refused before it is escaped.
        let error =
            canonical_json(&"\u{0}".repeat(MAX_CANONICAL_REQUEST_LEN)).expect_err("over the cap");
        assert_eq!(error.detail(), "canonical request must be 1 byte to 1 MiB");
    }

    /// `Display`s `chunks` chunks of 1 KiB through `collect_str`, counting
    /// the chunks formatted.
    struct Endless {
        chunks: usize,
        written: Cell<usize>,
    }

    impl std::fmt::Display for Endless {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let chunk = "k".repeat(1024);
            for _ in 0..self.chunks {
                self.written.set(self.written.get() + 1);
                formatter.write_str(&chunk)?;
            }
            Ok(())
        }
    }

    impl Serialize for Endless {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    /// A one-member map keyed by `key`.
    struct KeyedBy<'a, K>(&'a K);

    impl<K: Serialize> Serialize for KeyedBy<'_, K> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap as _;
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(self.0, &1)?;
            map.end()
        }
    }

    #[test]
    fn a_map_key_over_the_cap_is_refused_before_it_is_built() {
        let too_large = "canonical request must be 1 byte to 1 MiB";
        // `{"k…":1}`: the key alone takes the whole cap.
        let fits = "k".repeat(MAX_CANONICAL_REQUEST_LEN - 6);
        assert_eq!(
            canonical_json(&BTreeMap::from([(fits.as_str(), 1)]))
                .expect("exactly the cap")
                .len(),
            MAX_CANONICAL_REQUEST_LEN
        );
        let over = "k".repeat(MAX_CANONICAL_REQUEST_LEN - 1);
        let error = canonical_json(&BTreeMap::from([(over.as_str(), 1)])).expect_err("over");
        assert_eq!(error.detail(), too_large);
        let newtype = BTreeMap::from([(KeyNewtypeText(over), 1)]);
        assert_eq!(
            canonical_json(&newtype).expect_err("over").detail(),
            too_large
        );

        // A key formatted through `Display` stops at the cap, not after
        // its 64 MiB: only about the cap's worth of chunks is formatted.
        for keyed in [true, false] {
            let endless = Endless {
                chunks: 64 * 1024,
                written: Cell::new(0),
            };
            let error = if keyed {
                canonical_json(&KeyedBy(&endless))
            } else {
                canonical_json(&endless)
            }
            .expect_err("over the cap");
            assert_eq!(error.detail(), too_large, "keyed: {keyed}");
            let written = endless.written.get();
            assert!(
                written <= MAX_CANONICAL_REQUEST_LEN / 1024 + 1,
                "keyed: {keyed}: {written} chunks formatted before the refusal"
            );
        }
    }

    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    struct KeyNewtypeText(String);

    #[test]
    fn the_cap_counts_every_byte_of_a_nested_request_exactly_once() {
        let shape = |fill: usize| {
            serde_json::json!({
                "b": ["x".repeat(fill), 1, { "d": null, "c": [true, "\n"] }],
                "a": "\u{e9}",
                "e": {},
            })
        };
        let base = legacy_canonical(&shape(0)).expect("legacy").len();
        let fill = MAX_CANONICAL_REQUEST_LEN - base;
        let exact = canonical_json(&shape(fill)).expect("exactly the cap");
        assert_eq!(exact.len(), MAX_CANONICAL_REQUEST_LEN);
        assert_eq!(Some(exact), legacy_canonical(&shape(fill)));
        let error = canonical_json(&shape(fill + 1)).expect_err("one byte over");
        assert_eq!(error.detail(), "canonical request must be 1 byte to 1 MiB");
    }

    #[test]
    fn local_keys_are_deterministic_base64url_and_scoped() {
        let resource = resource_key!("billing.api");
        let other = resource_key!("billing.other");
        let key = local_idempotency_key(&resource, "charge", 1, "order-1").expect("key");
        assert_eq!(key.as_str().len(), 43);
        assert!(
            key.as_str()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        );
        assert_eq!(
            local_idempotency_key(&resource, "charge", 1, "order-1").expect("key"),
            key,
            "deterministic"
        );
        for different in [
            local_idempotency_key(&other, "charge", 1, "order-1"),
            local_idempotency_key(&resource, "refund", 1, "order-1"),
            local_idempotency_key(&resource, "charge", 2, "order-1"),
            local_idempotency_key(&resource, "charge", 1, "order-2"),
        ] {
            assert_ne!(different.expect("key"), key);
        }
        // The frames keep adjacent fields apart.
        assert_ne!(
            local_idempotency_key(&resource, "ab", 1, "c").expect("key"),
            local_idempotency_key(&resource, "a", 1, "bc").expect("key"),
        );
    }

    #[test]
    fn idempotency_keys_are_base64url_and_print() {
        let key = IdempotencyKey::new("AbC-_09").expect("valid");
        assert_eq!(key.as_str(), "AbC-_09");
        assert_eq!(key.to_string(), "AbC-_09");
        assert_eq!(format!("{key:?}"), "IdempotencyKey(\"AbC-_09\")");
        assert!(IdempotencyKey::new(&"a".repeat(64)).is_ok());
        for broken in ["", "a=", "a/b", "a+b", &"a".repeat(65)] {
            assert!(IdempotencyKey::new(broken).is_err(), "{broken:?}");
        }
    }
}
