//! Opt-in **keyed commitment** of secret material for content-addressing.
//!
//! By default [`FieldValue::canonical_bytes`](crate::FieldValue::canonical_bytes)
//! *rejects* a secret-bearing value (`secret.not_hashable`): a deterministic,
//! unkeyed hash of a low-entropy secret is a brute-forceable confirmation oracle.
//! When a caller genuinely needs a dedup / content key for a structure that
//! contains a secret, the committing variants
//! ([`FieldValue::canonical_bytes_committing`](crate::FieldValue::canonical_bytes_committing))
//! emit a **keyed** commitment instead — `blake3::keyed_hash` under a
//! [`CommitmentKey`] the attacker does not have, so the digest is a PRF output,
//! not an invertible hash of the secret.
//!
//! # Security model (what this does and does NOT give you)
//!
//! - A [`CommitmentKey`] is **process-scoped and ephemeral** by design:
//!   [`CommitmentKey::ephemeral`] mints a fresh 256-bit CSPRNG key held only in
//!   memory (zeroized on drop, never serialized, cloned, or logged). Commitments
//!   are therefore comparable **only within the lifetime of one key** — they are
//!   meaningless across process restarts and are *not* a durable address (hence
//!   [`CommitmentId`] is deliberately distinct from
//!   [`ContentId`](crate::ContentId) and is not serializable).
//! - Equality of two commitments under one key reveals only "these two secrets
//!   are equal". That is the same fact [`SecretValue`]'s constant-time `PartialEq`
//!   already answers; do **not** use a commitment as an equality oracle against
//!   attacker-supplied candidates.
//! - A single key gives intra-process equality across *all* commitments it
//!   produces — it is **not** per-tenant. A caller that must keep tenants
//!   mutually unlinkable has to derive a separate [`CommitmentKey`] per tenant
//!   (key derivation belongs to `nebula-credential`, not here).

use std::fmt;

use zeroize::Zeroizing;

use crate::secret::SecretValue;
use crate::value::VALUE_CANON_VERSION;

/// The reserved value-canon tag for an opt-in secret commitment (see the tag
/// table in `crate::value`). A commitment frame is `TAG + kind(1) + digest(32)`.
pub(crate) const TAG_SECRET_COMMITMENT: u8 = 0x0A;

/// Domain separator folded *inside* the keyed hash (distinct from the value-canon
/// domain so a commitment preimage can never alias an ordinary canon stream).
const SECRET_COMMIT_DOMAIN: &[u8] = b"nbschema-secret-commit-v";

/// Per-variant discriminator, folded into the keyed hash so a `String("abc")` and
/// a `Bytes(b"abc")` — identical raw bytes — commit to different digests.
const COMMIT_KIND_STRING: u8 = 0x01;
const COMMIT_KIND_BYTES: u8 = 0x02;

/// A process-scoped, ephemeral 256-bit key for [secret commitments](self).
///
/// The key is the entire security property: with it a commitment is just a hash
/// of the secret, so it must never be persisted, serialized, cloned into another
/// scope, or logged. The type enforces that structurally — it has no
/// `Serialize`/`Deserialize`/`Clone`/`Display`, a redacted `Debug`, and is
/// zeroized on drop. The only constructor that takes raw bytes is
/// [`CommitmentKey::for_testing`], whose name announces that production code must
/// use [`CommitmentKey::ephemeral`] instead.
pub struct CommitmentKey(Zeroizing<[u8; 32]>);

impl CommitmentKey {
    /// Mint a fresh process-scoped key from the thread CSPRNG. Use in production.
    #[must_use]
    pub fn ephemeral() -> Self {
        Self(Zeroizing::new(rand::random::<[u8; 32]>()))
    }

    /// Construct a key from fixed bytes — **tests only**. The name is the
    /// structural signal: a fixed/persisted key turns every commitment under it
    /// back into a brute-forceable oracle, so there is deliberately no
    /// `from_bytes` / `persistent` constructor for production use.
    #[must_use]
    pub fn for_testing(key: [u8; 32]) -> Self {
        Self(Zeroizing::new(key))
    }

    /// Borrow the raw key for `blake3::keyed_hash` (crate-internal only).
    pub(crate) fn raw(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CommitmentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CommitmentKey(<ephemeral>)")
    }
}

/// A 32-byte, **process-scoped** secret-commitment digest.
///
/// Distinct from [`ContentId`](crate::ContentId) by type so it can never be
/// stored where a durable content address is expected: it is keyed by an
/// ephemeral [`CommitmentKey`], so it is meaningless outside that key's lifetime
/// and is intentionally **not** serializable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommitmentId([u8; 32]);

impl CommitmentId {
    /// Construct from a raw digest (crate-internal: produced by the keyed hash).
    pub(crate) fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// Borrow the raw 32-byte digest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for CommitmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Whether a secret-bearing value may enter a canonical encoding, and how.
///
/// Threaded through the single `write_canonical` traversal so the default
/// (`Reject`) and committing (`Keyed`) paths share one code path. A future
/// scheme (e.g. a tenant-domained variant) is an additive enum arm, not a new
/// public method.
pub(crate) enum SecretCommitmentPolicy<'k> {
    /// Secrets have no canonical form — return `secret.not_hashable`.
    Reject,
    /// Commit each secret with `blake3::keyed_hash` under this key.
    Keyed(&'k CommitmentKey),
}

/// Write a keyed secret-commitment frame (`TAG 0x0A + kind + digest(32)`) into
/// `out`. The plaintext and its length live only inside a `Zeroizing` preimage
/// that is wiped immediately after hashing — the visible footprint is a
/// fixed-width 34-byte frame with no cleartext length.
pub(crate) fn write_secret_commitment(
    secret: &SecretValue,
    key: &CommitmentKey,
    out: &mut Vec<u8>,
) {
    let (kind, raw): (u8, &[u8]) = match secret {
        SecretValue::String(s) => (COMMIT_KIND_STRING, s.expose().as_bytes()),
        SecretValue::Bytes(b) => (COMMIT_KIND_BYTES, b.expose()),
    };

    // Preimage (invisible to observers; hashed under the key):
    //   domain || VALUE_CANON_VERSION || kind || len(u64) || raw
    // Fixed-width framing before `raw` keeps the encoding injective; the length
    // is inside the hash, never emitted as cleartext.
    let mut preimage: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(
        SECRET_COMMIT_DOMAIN.len() + 11 + raw.len(),
    ));
    preimage.extend_from_slice(SECRET_COMMIT_DOMAIN);
    preimage.extend_from_slice(&VALUE_CANON_VERSION.to_be_bytes());
    preimage.push(kind);
    preimage.extend_from_slice(&(raw.len() as u64).to_be_bytes());
    preimage.extend_from_slice(raw);

    let digest = blake3::keyed_hash(key.raw(), &preimage);

    out.push(TAG_SECRET_COMMITMENT);
    out.push(kind);
    out.extend_from_slice(digest.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FieldValue;

    fn fixed_key() -> CommitmentKey {
        CommitmentKey::for_testing([7u8; 32])
    }

    #[test]
    fn ephemeral_keys_differ() {
        // Two freshly minted keys must commit the same secret differently.
        let secret = FieldValue::SecretLiteral(SecretValue::string("hunter2".to_owned()));
        let a = secret
            .canonical_bytes_committing(&CommitmentKey::ephemeral())
            .expect("commit");
        let b = secret
            .canonical_bytes_committing(&CommitmentKey::ephemeral())
            .expect("commit");
        assert_ne!(a, b, "distinct keys must produce distinct commitments");
    }

    #[test]
    fn key_debug_does_not_leak_bytes() {
        let key = CommitmentKey::for_testing([0xABu8; 32]);
        let rendered = format!("{key:?}");
        assert_eq!(rendered, "CommitmentKey(<ephemeral>)");
        assert!(!rendered.contains("ab"), "Debug must not print key bytes");
    }

    #[test]
    fn string_and_bytes_with_same_raw_commit_differently() {
        let key = fixed_key();
        let as_string = FieldValue::SecretLiteral(SecretValue::string("abc".to_owned()))
            .canonical_bytes_committing(&key)
            .expect("commit");
        let as_bytes = FieldValue::SecretLiteral(SecretValue::bytes(b"abc".to_vec()))
            .canonical_bytes_committing(&key)
            .expect("commit");
        assert_ne!(
            as_string, as_bytes,
            "kind discriminator must domain-separate String from Bytes"
        );
    }

    #[test]
    fn same_key_same_secret_is_deterministic() {
        let key = fixed_key();
        let secret = FieldValue::SecretLiteral(SecretValue::string("s".to_owned()));
        assert_eq!(
            secret.canonical_bytes_committing(&key).expect("commit"),
            secret.canonical_bytes_committing(&key).expect("commit"),
        );
    }

    #[test]
    fn commitment_frame_shape_and_golden_hash() {
        // Freeze the exact frame so any preimage/format drift is caught.
        let key = CommitmentKey::for_testing([0u8; 32]);
        let bytes = FieldValue::SecretLiteral(SecretValue::string("x".to_owned()))
            .canonical_bytes_committing(&key)
            .expect("commit");
        // outer canon: domain(16) + version(2) + TAG(1) + kind(1) + digest(32) = 52
        assert_eq!(bytes.len(), 52, "frame size");
        assert_eq!(&bytes[0..16], b"nbschema-value-v", "outer canon domain");
        assert_eq!(&bytes[16..18], &[0x00, 0x01], "VALUE_CANON_VERSION = 1");
        assert_eq!(bytes[18], TAG_SECRET_COMMITMENT, "tag 0x0A");
        assert_eq!(bytes[19], COMMIT_KIND_STRING, "kind = string");

        let mut preimage = SECRET_COMMIT_DOMAIN.to_vec();
        preimage.extend_from_slice(&VALUE_CANON_VERSION.to_be_bytes());
        preimage.push(COMMIT_KIND_STRING);
        preimage.extend_from_slice(&1u64.to_be_bytes());
        preimage.push(b'x');
        let expected = blake3::keyed_hash(&[0u8; 32], &preimage);
        assert_eq!(&bytes[20..52], expected.as_bytes(), "commitment digest");
    }

    #[test]
    fn plaintext_never_appears_in_commitment() {
        let key = fixed_key();
        let bytes = FieldValue::SecretLiteral(SecretValue::string("hunter2".to_owned()))
            .canonical_bytes_committing(&key)
            .expect("commit");
        assert!(
            bytes.windows(7).all(|w| w != b"hunter2"),
            "the plaintext secret must never appear in the commitment"
        );
    }

    #[test]
    fn empty_secret_commits_to_full_width_frame() {
        let key = fixed_key();
        let bytes = FieldValue::SecretLiteral(SecretValue::string(String::new()))
            .canonical_bytes_committing(&key)
            .expect("commit");
        // No special-case: an empty secret still yields a 52-byte frame (no
        // length side-channel distinguishing empty from non-empty).
        assert_eq!(bytes.len(), 52);
    }
}
