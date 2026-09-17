//! Version-envelope wire format for persisted credential state (ADR-0107
//! Seam 2 — the owned record shape `{interface_version, schema_fingerprint,
//! kind_tag, body}`).
//!
//! `CredentialState` rows store the state JSON **wrapped** in the envelope
//! before encryption: `body` holds exactly the state JSON a pre-envelope row
//! held. Row columns (`state_kind` / `state_version`) are unchanged — they
//! remain the queryable axes; the envelope carries the axes the row cannot:
//! the schema fingerprint and a self-describing record inside the encrypted
//! body.
//!
//! # Decode contract ([`decode_state_payload`])
//!
//! Every persisted-state decode site in this crate funnels through
//! [`decode_state_payload`] — the single fail-closed choke point. Check
//! order:
//!
//! 1. `plaintext.len() >` [`MAX_STATE_PLAINTEXT_BYTES`] →
//!    [`StateEnvelopeError::StateTooLarge`] — the resource bound fires before
//!    any parse or materialization.
//! 2. Parse as an envelope. On parse failure, or a missing required field,
//!    the payload is a **legacy** (pre-envelope) row: refuse fail-closed if
//!    the row's `state_version` exceeds this build's `S::VERSION`, otherwise
//!    structurally validate the plaintext as JSON (iteratively, nothing
//!    materializes) and hand it through as the body. This decode-order
//!    fallback **is** the ordered migration — no DDL, and legacy rows decode
//!    forever.
//! 3. `interface_version > S::VERSION` → [`StateEnvelopeError::UnknownSchemaVersion`].
//! 4. `interface_version != state_version` (the row's axis) → [`StateEnvelopeError::VersionAxesDisagree`].
//! 5. `kind_tag != state_kind` (the row's axis) → [`StateEnvelopeError::KindMismatch`].
//! 6. `schema_fingerprint != S::SCHEMA_FINGERPRINT` → [`StateEnvelopeError::SchemaFingerprintMismatch`].
//! 7. Otherwise the body is returned as the opaque [`StateBody`], a **borrowed
//!    raw fragment** of the plaintext — no `serde_json::Value` materializes
//!    for the secret body bytes on either path. [`StateBody::into_state`]
//!    typed-decodes directly from the borrowed slice, the only place the body
//!    is parsed.
//!
//! **Recursion bound.** Every materializing parse runs under serde_json's
//! built-in recursion limit (depth 128, active in `from_slice`): the envelope
//! metadata parse and the typed body decode. The raw-fragment capture and the
//! legacy structural validation never recurse, so hostile nesting is refused
//! exactly where structure is built (the typed decode), never at a choke
//! point that would otherwise materialize it.
//!
//! Older `interface_version`s (≤ `S::VERSION`, and agreeing with the row
//! axis) decode forgivingly with the current reader — only newer shapes
//! refuse. A wire-shape change must therefore bump `CredentialState::VERSION`
//! **and** update the per-state fingerprint pins in the same change; rows
//! written before the shape change then refuse on fingerprint (fail-closed),
//! which is correct — their bytes do not decode with the new struct.
//!
//! Unknown **extra** envelope keys are tolerated (no `deny_unknown_fields`):
//! a future envelope field must not silently turn every new row into a
//! legacy decode. Only a **missing** required field falls back.
//!
//! ## Ambiguity edge
//!
//! A legacy payload that coincidentally carries all four envelope keys with
//! matching JSON types would be read as an envelope and refused on one of the
//! checks above. No current or past built-in wire shape contains those keys,
//! and encode never produces legacy output, so the edge is unreachable in
//! practice — but it is inherent to the decode-order fallback and documented
//! here rather than papered over.
//!
//! # Encode contract ([`encode_state_payload`])
//!
//! `interface_version` = `S::VERSION` (the same value the row's
//! `state_version` column records — the two axes must agree and the decode
//! side checks that), `kind_tag` = `S::KIND`, `schema_fingerprint` =
//! `S::SCHEMA_FINGERPRINT` (little-endian), `body` = the state itself.
//! Serialization is streaming (`serde_json::Serializer` +
//! `serialize_map`), so no plaintext `Value` intermediate materializes and
//! the whole buffer lives in `Zeroizing<Vec<u8>>`; callers invoke it inside
//! `serde_secret::expose_for_serialization`. Encode never produces legacy.

use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::value::RawValue;
use zeroize::Zeroizing;

use crate::contract::{CredentialState, StateWireFingerprint};

/// Upper bound on the decrypted plaintext this module will even look at,
/// checked before any parse or materialization. Generous against every real
/// state — the largest built-in is OAuth2's token set, a handful of small
/// strings, orders of magnitude below this — while a decrypting writer that
/// stored an arbitrarily large body is capped here, not at the typed decode.
pub(crate) const MAX_STATE_PLAINTEXT_BYTES: usize = 1 << 20; // 1 MiB

/// The envelope record shape (ADR-0107 Seam 2) as it rides inside the
/// encrypted credential row, read borrowingly: `kind_tag` and `body` borrow
/// the caller's (zeroizing) plaintext buffer, and `body` is captured as a raw
/// fragment — no `serde_json::Value` materializes for the secret body bytes.
/// Private: the module's two functions are the only entry and exit.
#[derive(Debug, Deserialize)]
struct StateEnvelope<'a> {
    /// Producer's state-shape version — the same value the row's
    /// `state_version` column records.
    interface_version: u32,
    /// FNV-1a 64 of the producer's wire shape, little-endian bytes.
    schema_fingerprint: [u8; 8],
    /// The state kind the producer wrote (`CredentialState::KIND`).
    #[serde(borrow)]
    kind_tag: &'a str,
    /// The state JSON, exactly as a pre-envelope row held it — borrowed raw,
    /// never materialized.
    #[serde(borrow)]
    body: &'a RawValue,
}

/// Fail-closed envelope check failures.
///
/// Every payload is deliberately `Copy`-friendly (numeric or `&'static str`
/// payloads only): [`crate::CredentialSlotResolveError`] carries this type by
/// value and keeps its `Copy` derive. The **found** kind tag is therefore not
/// echoed — only the expected kind — and the decode caller's parse failure on
/// the legacy path keeps its own classification (see
/// [`StateEnvelopeError::LegacyStateParseFailed`]).
///
/// `pub` because it rides in the public
/// [`crate::CredentialSlotResolveError::StoredStateRefused`] variant; the
/// module itself and its functions stay `pub(crate)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StateEnvelopeError {
    /// The stored state's shape version is newer than this build supports.
    /// Fail closed: the row is left untouched.
    #[error(
        "stored credential state version {stored_version} is newer than the version this build \
         supports ({supported_version})"
    )]
    UnknownSchemaVersion {
        /// Version recorded by the producer (the envelope's
        /// `interface_version`, or the row's `state_version` on the legacy
        /// path).
        stored_version: u32,
        /// This build's `CredentialState::VERSION` for the state type.
        supported_version: u32,
    },
    /// The envelope's `interface_version` disagrees with the row's
    /// `state_version` axis. The two must agree (the envelope carries the
    /// same value the row column records).
    #[error(
        "credential state envelope version {envelope_version} disagrees with the stored row's \
         state_version {row_version}"
    )]
    VersionAxesDisagree {
        /// The envelope's `interface_version`.
        envelope_version: u32,
        /// The row column's `state_version`.
        row_version: u32,
    },
    /// The envelope's `kind_tag` is not the kind this reader was invoked for.
    /// The found value is not echoed (it is a stored string, potentially
    /// attacker-adjacent); only the expected kind is shown.
    #[error("stored credential state kind does not match the expected kind {expected}")]
    KindMismatch {
        /// `CredentialState::KIND` of the reader's state type.
        expected: &'static str,
    },
    /// The envelope's schema fingerprint is not this build's fingerprint for
    /// the state type — the stored wire shape is not the shape this build
    /// understands. Fail closed before deserialization.
    #[error(
        "stored credential state schema fingerprint {stored:#018x} does not match the \
         fingerprint this build expects ({expected:#018x})"
    )]
    SchemaFingerprintMismatch {
        /// This build's `SCHEMA_FINGERPRINT`.
        expected: u64,
        /// The fingerprint read from the envelope.
        stored: u64,
    },
    /// The payload is not an envelope and does not parse as legacy state
    /// JSON. Kept distinct from the envelope checks: decode callers map it
    /// back to their existing corrupt-row classification rather than the new
    /// envelope-refusal category.
    #[error("stored credential state is neither a valid state envelope nor legacy state JSON")]
    LegacyStateParseFailed,
    /// The decrypted plaintext exceeds `MAX_STATE_PLAINTEXT_BYTES`. Checked
    /// before any parse or materialization, so an oversized stored body is
    /// refused without touching it.
    #[error(
        "stored credential state plaintext is {bytes} bytes, above the supported bound of \
         {limit} bytes"
    )]
    StateTooLarge {
        /// The plaintext length observed.
        bytes: usize,
        /// `MAX_STATE_PLAINTEXT_BYTES`.
        limit: usize,
    },
}

/// Body of a decoded state payload — the state JSON exactly as a pre-envelope
/// row held it, borrowing the caller's decrypted plaintext. No
/// `serde_json::Value` ever materializes for these bytes: the legacy path
/// hands the validated plaintext slice through directly, the envelope path
/// hands the raw fragment the envelope parse captured. Opaque outside this
/// module: only [`decode_state_payload`] produces it and only
/// [`StateBody::into_state`] consumes it, so typed reinterpretation of
/// generic state stays confined to the wire-format owner (the architecture
/// ratchet in `tests/refresh_routing_architecture.rs` forbids the resolver
/// from round-tripping generic state through a cleartext `serde_json::Value`).
pub(crate) struct StateBody<'a>(&'a [u8]);

impl StateBody<'_> {
    /// Typed decode of the body into the reader's state type, straight from
    /// the borrowed slice — the only place the body bytes are parsed.
    pub(crate) fn into_state<S: DeserializeOwned>(self) -> Result<S, serde_json::Error> {
        serde_json::from_slice(self.0)
    }
}

/// Decode a persisted-state plaintext through the single fail-closed choke
/// point; see the module docs for the exact check order.
///
/// `state_kind` and `state_version` are the row's queryable axes.
/// `S` is the reader's state type — in practice its `KIND`/`VERSION` equal
/// the row's, because credential dispatch selects `S` by the row's kind
/// before decoding.
pub(crate) fn decode_state_payload<'a, S: CredentialState + StateWireFingerprint>(
    plaintext: &'a [u8],
    state_kind: &str,
    state_version: u32,
) -> Result<StateBody<'a>, StateEnvelopeError> {
    // Resource bound before anything else: a decrypting writer may have
    // stored an arbitrarily large body, and the bound is the last word on
    // what this reader will materialize from it.
    if plaintext.len() > MAX_STATE_PLAINTEXT_BYTES {
        return Err(StateEnvelopeError::StateTooLarge {
            bytes: plaintext.len(),
            limit: MAX_STATE_PLAINTEXT_BYTES,
        });
    }

    // Not an envelope (parse failure or missing required field): legacy row.
    // The row axis is still checked fail-closed — a legacy row cannot claim a
    // version newer than this reader. The envelope's borrow lifetime is left
    // to inference: the derived `Deserialize` impl requires the deserializer's
    // lifetime to outlive the envelope's, which ties it exactly to
    // `plaintext`'s borrow — the same lifetime `StateBody` carries back out.
    let envelope: StateEnvelope<'_> =
        if let Ok(envelope) = serde_json::from_slice::<StateEnvelope<'_>>(plaintext) {
            envelope
        } else {
            if state_version > S::VERSION {
                return Err(StateEnvelopeError::UnknownSchemaVersion {
                    stored_version: state_version,
                    supported_version: S::VERSION,
                });
            }
            // Structural validation only — the legacy body is plaintext secret
            // material and must not materialize as a `Value`. `IgnoredAny` walks
            // the document iteratively (no recursion); the typed decode below
            // runs under serde_json's built-in recursion limit, and this pass
            // exists to keep the `LegacyStateParseFailed` classification for
            // bytes that are not JSON at all.
            let mut deserializer = serde_json::Deserializer::from_slice(plaintext);
            if IgnoredAny::deserialize(&mut deserializer)
                .and_then(|_| deserializer.end())
                .is_err()
            {
                return Err(StateEnvelopeError::LegacyStateParseFailed);
            }
            return Ok(StateBody(plaintext));
        };

    if envelope.interface_version > S::VERSION {
        return Err(StateEnvelopeError::UnknownSchemaVersion {
            stored_version: envelope.interface_version,
            supported_version: S::VERSION,
        });
    }
    if envelope.interface_version != state_version {
        return Err(StateEnvelopeError::VersionAxesDisagree {
            envelope_version: envelope.interface_version,
            row_version: state_version,
        });
    }
    if envelope.kind_tag != state_kind {
        return Err(StateEnvelopeError::KindMismatch { expected: S::KIND });
    }
    let stored = u64::from_le_bytes(envelope.schema_fingerprint);
    if stored != S::SCHEMA_FINGERPRINT {
        return Err(StateEnvelopeError::SchemaFingerprintMismatch {
            expected: S::SCHEMA_FINGERPRINT,
            stored,
        });
    }

    Ok(StateBody(envelope.body.get().as_bytes()))
}

/// Wrap a state in the envelope and serialize it, streaming into a
/// `Zeroizing` buffer (no plaintext `Value` intermediate). Call inside
/// `serde_secret::expose_for_serialization`.
pub(crate) fn encode_state_payload<S: CredentialState + StateWireFingerprint>(
    state: &S,
) -> Result<Zeroizing<Vec<u8>>, serde_json::Error> {
    use serde::ser::{SerializeMap, Serializer as _};

    let mut buffer = Zeroizing::new(Vec::new());
    let mut serializer = serde_json::Serializer::new(&mut *buffer);
    let mut map = serializer.serialize_map(Some(4))?;
    map.serialize_entry("interface_version", &S::VERSION)?;
    map.serialize_entry("schema_fingerprint", &S::SCHEMA_FINGERPRINT.to_le_bytes())?;
    map.serialize_entry("kind_tag", S::KIND)?;
    map.serialize_entry("body", state)?;
    map.end()?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use zeroize::{Zeroize, ZeroizeOnDrop};

    /// Pin the wire-shape fingerprint of every built-in persisted state type.
    ///
    /// The values are computed by the `StateWireFingerprint` derive over the
    /// field projection (`<name>\n<req|opt>\n<type-path>` per field, FNV-1a
    /// 64). Any change to a pinned value means the wire shape changed: either
    /// revert the shape change or bump `CredentialState::VERSION` together
    /// with it. Doc comments and authored annotations must NOT move these
    /// values.
    #[test]
    fn built_in_state_wire_fingerprints_are_pinned() {
        assert_eq!(
            <crate::scheme::SecretToken as StateWireFingerprint>::SCHEMA_FINGERPRINT,
            0x88e3_98a4_66da_8c57,
            "SecretToken"
        );
        assert_eq!(
            <crate::scheme::SharedKey as StateWireFingerprint>::SCHEMA_FINGERPRINT,
            0x95d8_6e01_11f2_ba58,
            "SharedKey"
        );
        assert_eq!(
            <crate::scheme::SigningKey as StateWireFingerprint>::SCHEMA_FINGERPRINT,
            0x28df_8b47_d683_c547,
            "SigningKey"
        );
        assert_eq!(
            <crate::scheme::IdentityPassword as StateWireFingerprint>::SCHEMA_FINGERPRINT,
            0x7e74_777b_2eba_fc52,
            "IdentityPassword"
        );
        assert_eq!(
            <crate::OAuth2State as StateWireFingerprint>::SCHEMA_FINGERPRINT,
            0x3ef6_0bc8_24fe_22ee,
            "OAuth2State"
        );
        assert_eq!(
            <crate::NoCredentialState as StateWireFingerprint>::SCHEMA_FINGERPRINT,
            0xcbf2_9ce4_8422_2325,
            "NoCredentialState"
        );
    }

    /// The typed body decode must run under serde_json's built-in recursion
    /// limit (depth 128): a hostile body nested past it fails there instead
    /// of materializing unbounded structure. The envelope metadata parse
    /// cannot see the depth (the body is captured as a raw fragment by
    /// design), so the limit is live exactly where the recursion happens.
    #[test]
    fn typed_body_decode_respects_the_serde_json_recursion_limit() {
        #[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop, Debug)]
        struct DeepProbeState {
            // The probe exists to prove the typed decode's recursion limit;
            // the nested shape itself holds no secret worth zeroizing.
            #[zeroize(skip)]
            nested: Value,
        }

        impl CredentialState for DeepProbeState {
            const KIND: &'static str = "deep_probe";
            const VERSION: u32 = 1;
        }

        impl StateWireFingerprint for DeepProbeState {
            const SCHEMA_FINGERPRINT: u64 = 0;
        }

        let deep_body = format!("{}{}", "[".repeat(300), "]".repeat(300));
        // The envelope is assembled as literal text: the raw body capture is
        // iterative and never trips the recursion limit, so the probe must
        // reach the typed decode with a body the decoder itself rejects.
        let envelope_text = format!(
            r#"{{"interface_version":1,"schema_fingerprint":[0,0,0,0,0,0,0,0],"kind_tag":"deep_probe","body":{deep_body}}}"#
        );
        let plaintext = envelope_text.as_bytes();

        let body = match decode_state_payload::<DeepProbeState>(plaintext, "deep_probe", 1) {
            Ok(body) => body,
            // Deliberately not `.expect(...)`: `StateBody` borrows plaintext
            // secret bytes and must never gain a `Debug` impl.
            Err(error) => {
                panic!("the envelope checks pass; the hostile depth lives in the body: {error}");
            },
        };
        let err = body
            .into_state::<DeepProbeState>()
            .expect_err("a body nested past serde_json's depth limit must refuse");
        assert!(
            err.to_string().contains("recursion limit exceeded"),
            "the typed decode must fail on the recursion limit, got {err}"
        );
    }

    /// The plaintext size bound fires before any parse: an oversized stored
    /// body is refused without ever being read as JSON (a decrypting writer
    /// may store an arbitrarily large body, and materialization is capped at
    /// the choke point).
    #[test]
    fn oversized_plaintext_refuses_before_any_parse() {
        // Deliberately not valid JSON in any form: if the size bound did not
        // fire first, this would fall to LegacyStateParseFailed instead.
        let oversized = vec![b'{'; MAX_STATE_PLAINTEXT_BYTES + 1];
        let err = match decode_state_payload::<crate::scheme::SecretToken>(
            &oversized,
            <crate::scheme::SecretToken as CredentialState>::KIND,
            <crate::scheme::SecretToken as CredentialState>::VERSION,
        ) {
            // No `.expect_err(...)`: `StateBody` borrows plaintext secret
            // bytes and deliberately has no `Debug` impl.
            Err(error) => error,
            Ok(_) => panic!("an oversized plaintext must refuse at the choke point"),
        };
        assert_eq!(
            err,
            StateEnvelopeError::StateTooLarge {
                bytes: MAX_STATE_PLAINTEXT_BYTES + 1,
                limit: MAX_STATE_PLAINTEXT_BYTES,
            },
            "the refusal must name the observed size and the bound"
        );
    }
}
