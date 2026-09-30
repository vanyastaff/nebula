//! Execution-owned effects: the author's declaration of an effectful
//! operation whose outcome an execution owner records.
//!
//! An [`EffectOperation`] is an [`Operation`] that also names its
//! integration contract ([`EffectContract`]), how a repeat is recovered
//! ([`EffectRecovery`]), what the owner keeps of a success ([`Recorded`]),
//! its canonical request and, optionally, the developer part of the
//! provider idempotency key ([`IdempotencyKeyPart`]) and a stable
//! occurrence label ([`OccurrenceLabel`]). It is submitted with
//! [`ResourceHandle::submit_effect`](super::ResourceHandle::submit_effect); the
//! owner seam that records it lives in [`owner`](super::owner).

use std::{fmt, time::Duration};

use serde::{Serialize, de::DeserializeOwned};

use super::{Operation, cost::Effect, error::OperationError, pin::PinSlots};
use crate::{error::ErrorKind, resource::Provider};

/// Longest contract id, in bytes.
const MAX_CONTRACT_ID_LEN: usize = 128;
/// Longest developer idempotency key part, in bytes.
const MAX_KEY_PART_LEN: usize = 256;
/// Longest author occurrence label, in bytes.
const MAX_OCCURRENCE_LEN: usize = 128;
/// Longest operation key, in bytes.
const MAX_OPERATION_KEY_LEN: usize = 64;

/// The integration contract an effectful operation follows: a stable id and
/// the version of its request canonicalization.
///
/// The owner binds a recorded effect to it; changing either means a
/// different effect, so a resumed execution with another contract is
/// refused as a mismatch rather than replayed.
///
/// ```
/// use nebula_resource::call::EffectContract;
///
/// const SEND: EffectContract = EffectContract::new("mail.send/v1", 1);
/// assert!(SEND.validate().is_ok());
/// assert!(EffectContract::new("has space", 1).validate().is_err());
/// assert!(EffectContract::new("mail.send", 0).validate().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectContract {
    id: &'static str,
    canonicalization_version: u16,
}

impl EffectContract {
    /// A contract named `id` whose requests are canonicalized by version
    /// `canonicalization_version`. Checked by [`validate`](Self::validate)
    /// when a unit is submitted.
    #[must_use]
    pub const fn new(id: &'static str, canonicalization_version: u16) -> Self {
        Self {
            id,
            canonicalization_version,
        }
    }

    /// The contract id.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// The canonicalization version.
    #[must_use]
    pub const fn canonicalization_version(&self) -> u16 {
        self.canonicalization_version
    }

    /// Checks the contract: an id of 1 to 128 bytes of `[A-Za-z0-9._/-]`
    /// and a non-zero version — the rules of an action's remote effect
    /// contract.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Permanent`] naming the broken rule.
    pub fn validate(&self) -> Result<(), OperationError> {
        let id_is_valid = !self.id.is_empty()
            && self.id.len() <= MAX_CONTRACT_ID_LEN
            && self.id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
            });
        if !id_is_valid {
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "effect contract id must be 1..=128 bytes of [A-Za-z0-9._/-]",
            ));
        }
        if self.canonicalization_version == 0 {
            return Err(OperationError::new(
                ErrorKind::Permanent,
                "effect contract canonicalization version must be non-zero",
            ));
        }
        Ok(())
    }
}

/// How the owner recovers an effect whose outcome it does not know.
///
/// Must agree with the operation's [`Effect`]: an
/// [`Idempotent`](Effect::Idempotent) operation recovers by
/// [`StableKey`](Self::StableKey), a [`Write`](Effect::Write) is
/// [`Opaque`](Self::Opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EffectRecovery {
    /// The provider deduplicates requests carrying the same idempotency key
    /// for `window`: within it, an ambiguous attempt may be sent again
    /// with the same [`IdempotencyKey`].
    StableKey {
        /// How long the provider remembers a key. Non-zero.
        window: Duration,
    },
    /// Nothing tells a repeat apart: an ambiguous attempt is never sent
    /// again, its outcome is unknown until reconciled.
    Opaque,
}

impl EffectRecovery {
    /// Whether this recovery fits `effect`, and why not.
    pub(crate) fn check(self, effect: Effect) -> Result<(), OperationError> {
        match (effect, self) {
            (Effect::Read, _) => Err(OperationError::new(
                ErrorKind::Permanent,
                "a read is not an owned effect; submit it with submit",
            )),
            (Effect::Idempotent, Self::StableKey { window }) if window.is_zero() => Err(
                OperationError::new(ErrorKind::Permanent, "a stable-key window must be non-zero"),
            ),
            (Effect::Idempotent, Self::StableKey { .. }) | (Effect::Write, Self::Opaque) => Ok(()),
            _ => Err(OperationError::new(
                ErrorKind::Permanent,
                "effect recovery disagrees with the effect: idempotent needs a stable key, write is opaque",
            )),
        }
    }

    /// Stable lowercase name: `stable_key` or `opaque`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StableKey { .. } => "stable_key",
            Self::Opaque => "opaque",
        }
    }
}

/// What the owner records of a successful effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Recorded {
    /// The serialized output: a resumed execution replays it without a
    /// provider call.
    #[default]
    Output,
    /// Only that the effect applied: a resumed execution fails
    /// `Permanent` ("recorded without output") instead of replaying.
    DigestOnly,
}

/// Whether `bytes` is `1..=max` bytes of visible ASCII (`0x21..=0x7E`).
fn visible_ascii(bytes: &[u8], max: usize) -> bool {
    !bytes.is_empty() && bytes.len() <= max && bytes.iter().all(|byte| (0x21..=0x7E).contains(byte))
}

/// The developer part of the provider idempotency key.
///
/// Built deterministically from the operation's input or the action's
/// state — `order-123` — never random and never a retry number: every
/// attempt, retry and resume of one effect must present the same part. With
/// a part the owner deduplicates across executions (make it specific
/// enough); without one it scopes the key to the execution, node and
/// occurrence. 1 to 256 bytes of visible ASCII.
///
/// ```
/// use nebula_resource::call::IdempotencyKeyPart;
///
/// assert!(IdempotencyKeyPart::new("order-123").is_ok());
/// assert!(IdempotencyKeyPart::new("order 123").is_err());
/// assert!(IdempotencyKeyPart::new("").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKeyPart(String);

impl IdempotencyKeyPart {
    /// A key part of `part`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Permanent`] when `part` is empty, longer than 256
    /// bytes, or holds anything but visible ASCII.
    pub fn new(part: impl Into<String>) -> Result<Self, OperationError> {
        let part = part.into();
        if visible_ascii(part.as_bytes(), MAX_KEY_PART_LEN) {
            Ok(Self(part))
        } else {
            Err(OperationError::new(
                ErrorKind::Permanent,
                "idempotency key part must be 1..=256 bytes of visible ASCII",
            ))
        }
    }

    /// The part.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A stable author label for one effect of an action — `charge`,
/// `refund` — used instead of the submit ordinal. 1 to 128 bytes of visible
/// ASCII.
///
/// ```
/// use nebula_resource::call::OccurrenceLabel;
///
/// assert!(OccurrenceLabel::new("charge").is_ok());
/// assert!(OccurrenceLabel::new("a label").is_err());
/// assert!(OccurrenceLabel::new("x".repeat(129)).is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OccurrenceLabel(String);

impl OccurrenceLabel {
    /// A label of `label`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Permanent`] when `label` is empty, longer than 128
    /// bytes, or holds anything but visible ASCII.
    pub fn new(label: impl Into<String>) -> Result<Self, OperationError> {
        let label = label.into();
        if visible_ascii(label.as_bytes(), MAX_OCCURRENCE_LEN) {
            Ok(Self(label))
        } else {
            Err(OperationError::new(
                ErrorKind::Permanent,
                "occurrence label must be 1..=128 bytes of visible ASCII",
            ))
        }
    }

    /// The label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An [`Operation`] whose effect an execution owner records, submitted with
/// [`ResourceHandle::submit_effect`](super::ResourceHandle::submit_effect).
///
/// On a row an action got with effect-owner authority, the owner prepares
/// the effect before anything is checked out: a recorded success is
/// replayed from its output without a provider call, a recorded failure is
/// returned again, an effect whose outcome is unknown is refused
/// `OutcomeUnknown`. Otherwise every attempt is granted by the owner, and
/// the unit's outcome is recorded when it settles. [`OperationCx::idempotency_key`](super::OperationCx::idempotency_key)
/// is the provider idempotency key the owner derived: the same for every
/// attempt, retry and resume of the effect.
///
/// [`Operation::EFFECT`] must be [`Idempotent`](Effect::Idempotent) (with
/// [`EffectRecovery::StableKey`]) or [`Write`](Effect::Write) (with
/// [`EffectRecovery::Opaque`]); a disagreement, or a
/// [`Read`](Effect::Read), is refused `Permanent` / `NotSent` at submit.
///
/// ```
/// use std::time::Duration;
///
/// use nebula_resource::{
///     PinSlots, Provider,
///     call::{
///         Cost, Effect, EffectContract, EffectOperation, EffectRecovery, IdempotencyKeyPart, OperationCx,
///         OperationError, Operation, SentState,
///     },
/// };
///
/// /// Charges an order once, keyed by the order id.
/// struct Charge {
///     order: u64,
///     cents: u64,
/// }
///
/// impl<R: Provider + PinSlots> Operation<R> for Charge {
///     type Output = u64;
///     const EFFECT: Effect = Effect::Idempotent;
///
///     async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<u64, OperationError> {
///         // The provider's idempotency key: the same for every attempt,
///         // retry and resume. Read it before an attempt borrows `cx`.
///         let _key = cx.idempotency_key().copied();
///         let attempt = cx.attempt(Cost::ONE).await?;
///         attempt.settle(SentState::Sent);
///         Ok(self.cents)
///     }
/// }
///
/// impl<R: Provider + PinSlots> EffectOperation<R> for Charge {
///     const CONTRACT: EffectContract = EffectContract::new("billing.charge", 1);
///     const RECOVERY: EffectRecovery = EffectRecovery::StableKey {
///         window: Duration::from_secs(24 * 60 * 60),
///     };
///
///     fn canonical_request(&self) -> Result<Vec<u8>, OperationError> {
///         Ok(format!("{}:{}", self.order, self.cents).into_bytes())
///     }
///
///     fn idempotency_key(&self) -> Option<IdempotencyKeyPart> {
///         IdempotencyKeyPart::new(format!("order-{}", self.order)).ok()
///     }
/// }
/// ```
pub trait EffectOperation<R: Provider + PinSlots>:
    Operation<R, Output: Serialize + DeserializeOwned>
{
    /// The integration contract the effect follows.
    const CONTRACT: EffectContract;

    /// How an unknown outcome is recovered; must agree with
    /// [`Operation::EFFECT`] (see the trait docs).
    const RECOVERY: EffectRecovery;

    /// What the owner records of a success; the output by default.
    const RECORDED: Recorded = Recorded::Output;

    /// The canonical logical request — no credentials, signatures or
    /// timestamps — of 1 byte to 1 MiB. The owner digests it to tell a
    /// resumed effect from a different one; it is never stored.
    ///
    /// # Errors
    ///
    /// Any error refuses the unit unsent with that error.
    fn canonical_request(&self) -> Result<Vec<u8>, OperationError>;

    /// The developer part of the provider idempotency key; `None` scopes
    /// the key to the execution, node and occurrence.
    fn idempotency_key(&self) -> Option<IdempotencyKeyPart> {
        None
    }

    /// A stable label for this effect among the action's effects; `None`
    /// numbers it in submit order per resource and contract.
    fn occurrence(&self) -> Option<OccurrenceLabel> {
        None
    }
}

/// The provider idempotency key an owner derived for one effect and
/// records durably before its first attempt: base64url, 1 to 64 bytes.
///
/// The same for every attempt, retry and resume of the effect. It is not a
/// secret — [`Display`](fmt::Display) prints it — but it is not authority
/// either: holding one grants no provider call.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdempotencyKey {
    bytes: [u8; MAX_OPERATION_KEY_LEN],
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
            && raw.len() <= MAX_OPERATION_KEY_LEN
            && raw
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
        let Ok(len) = u8::try_from(raw.len()) else {
            return Err(invalid_idempotency_key());
        };
        if !valid {
            return Err(invalid_idempotency_key());
        }
        let mut bytes = [0; MAX_OPERATION_KEY_LEN];
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
        "operation key must be 1..=64 bytes of base64url",
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
    use std::time::Duration;

    use super::{
        Effect, EffectContract, EffectRecovery, IdempotencyKey, IdempotencyKeyPart, OccurrenceLabel,
    };
    use crate::ErrorKind;

    #[test]
    fn contracts_follow_the_remote_effect_rules() {
        assert!(EffectContract::new("a.b_c-d/e", 1).validate().is_ok());
        assert!(
            EffectContract::new("x".repeat(128).leak(), 1)
                .validate()
                .is_ok()
        );
        for broken in [
            EffectContract::new("", 1),
            EffectContract::new("sp ace", 1),
            EffectContract::new("uni\u{e9}", 1),
            EffectContract::new("x".repeat(129).leak(), 1),
            EffectContract::new("ok", 0),
        ] {
            let error = broken.validate().expect_err("refused");
            assert_eq!(*error.kind(), ErrorKind::Permanent);
        }
    }

    #[test]
    fn recovery_must_agree_with_the_effect() {
        let stable = EffectRecovery::StableKey {
            window: Duration::from_mins(1),
        };
        assert!(stable.check(Effect::Idempotent).is_ok());
        assert!(EffectRecovery::Opaque.check(Effect::Write).is_ok());
        assert!(stable.check(Effect::Write).is_err());
        assert!(EffectRecovery::Opaque.check(Effect::Idempotent).is_err());
        assert!(EffectRecovery::Opaque.check(Effect::Read).is_err());
        assert!(stable.check(Effect::Read).is_err());
        let zero = EffectRecovery::StableKey {
            window: Duration::ZERO,
        };
        assert!(zero.check(Effect::Idempotent).is_err());
    }

    #[test]
    fn key_parts_and_labels_are_visible_ascii_and_bounded() {
        assert!(IdempotencyKeyPart::new("~!order#1").is_ok());
        assert!(IdempotencyKeyPart::new("x".repeat(256)).is_ok());
        assert!(IdempotencyKeyPart::new("x".repeat(257)).is_err());
        assert!(IdempotencyKeyPart::new("tab\t").is_err());
        assert!(OccurrenceLabel::new("x".repeat(128)).is_ok());
        assert!(OccurrenceLabel::new("x".repeat(129)).is_err());
        assert!(OccurrenceLabel::new("a b").is_err());
        assert!(OccurrenceLabel::new("").is_err());
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
