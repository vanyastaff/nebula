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
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::{cost::Effect, error::OperationError};
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
/// tell a resumed effect from a different one.
pub(super) fn canonical_json<T: Serialize + ?Sized>(
    request: &T,
) -> Result<Vec<u8>, OperationError> {
    let unserializable = || {
        OperationError::new(
            ErrorKind::Permanent,
            "operation request does not serialize to JSON",
        )
    };
    let value = serde_json::to_value(request).map_err(|_| unserializable())?;
    let mut canonical = Vec::new();
    emit_sorted(&value, &mut canonical).map_err(|_| unserializable())?;
    if canonical.is_empty() || canonical.len() > MAX_CANONICAL_REQUEST_LEN {
        return Err(OperationError::new(
            ErrorKind::Permanent,
            "canonical request must be 1 byte to 1 MiB",
        ));
    }
    Ok(canonical)
}

/// Writes `value` as compact JSON with every object's keys sorted.
fn emit_sorted(value: &Value, out: &mut Vec<u8>) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| *key);
            out.push(b'{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                emit_sorted(item, out)?;
            }
            out.push(b'}');
        },
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                emit_sorted(item, out)?;
            }
            out.push(b']');
        },
        scalar => serde_json::to_writer(&mut *out, scalar)?,
    }
    Ok(())
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
    use std::{collections::BTreeMap, time::Duration};

    use nebula_core::resource_key;
    use serde::Serialize;

    use super::{
        Effect, IdempotencyKey, canonical_json, check_declaration, check_key_part,
        is_valid_declaration, is_valid_operation_key, local_idempotency_key,
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
