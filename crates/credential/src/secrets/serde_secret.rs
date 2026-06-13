//! Sink-aware serde helpers for [`SecretString`].
//!
//! Use with `#[serde(with = "nebula_credential::serde_secret")]` for
//! `SecretString` fields or `#[serde(with = "nebula_credential::serde_secret::option")]`
//! for `Option<SecretString>` fields.
//!
//! # Why this exists
//!
//! [`SecretString`]'s own `Serialize` always writes the `[REDACTED]` sentinel,
//! which is correct for logs and API responses but loses the value for
//! encrypted-at-rest persistence. The naive fix — a helper that *always* writes
//! the cleartext value — over-corrects: a serializer cannot inspect its sink,
//! so an "always cleartext" field is emitted verbatim by any incidental
//! `serde_json::to_string`, `tracing` value, or response serializer. That is a
//! silent exfiltration path.
//!
//! These helpers therefore gate cleartext on an explicit thread-local scope
//! ([`expose_for_serialization`]): inside the scope a field serializes its real
//! value; outside it the field redacts to `[REDACTED]`, identical to
//! [`SecretString`]'s default. The safe behavior is the default; cleartext is
//! opt-in and greppable.

use std::cell::Cell;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::SecretString;

thread_local! {
    /// Re-entrant depth of the active cleartext-serialization scope on this
    /// thread. `0` ⇒ secrets redact to the `[REDACTED]` sentinel (the safe
    /// default); `> 0` ⇒ secrets serialize their cleartext value. A counter,
    /// not a bool, so nested scopes (e.g. a sealed state whose fields are
    /// themselves sealed) compose without the inner scope's exit re-enabling
    /// redaction for the outer one.
    static EXPOSE_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// `true` while the current thread is inside an [`expose_for_serialization`]
/// scope, i.e. while `#[serde(with = "serde_secret")]` fields should emit
/// cleartext rather than the `[REDACTED]` sentinel.
fn cleartext_enabled() -> bool {
    EXPOSE_DEPTH.with(|depth| depth.get() > 0)
}

/// RAII guard that raises the cleartext-serialization depth for its lifetime
/// and lowers it on drop — including on unwind, so a panic mid-serialization
/// cannot leave the thread stuck in cleartext mode.
struct DepthGuard;

impl DepthGuard {
    fn enter() -> Self {
        EXPOSE_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        Self
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        EXPOSE_DEPTH.with(|depth| {
            let current = depth.get();
            debug_assert!(
                current > 0,
                "cleartext-serialization depth underflow: a DepthGuard dropped \
                 while depth was 0 — guards must be balanced and never forged"
            );
            depth.set(current.saturating_sub(1));
        });
    }
}

/// Run `f` with cleartext secret serialization enabled on the current thread,
/// then restore the prior (redacting) behavior.
///
/// This is the **only** way to make a `#[serde(with = "serde_secret")]` field
/// emit its real value. It marks the call site as a trusted full-fidelity sink:
/// the encrypted-at-rest persistence path (the produced bytes are handed
/// straight to the storage encryption layer) and internal full-fidelity state
/// round-trips (e.g. OAuth2 refresh-state transforms). It is **not** for
/// telemetry, logs, API responses, or debug output — those must redact.
///
/// The scope is thread-local and re-entrant. Wrap **only the synchronous
/// `serialize` call**; never hold it across an `.await`, or an unrelated future
/// resumed on the same worker thread could serialize cleartext. Every cleartext
/// site is greppable (`rg expose_for_serialization`), exactly like every
/// plaintext read is greppable via `expose_secret`.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::SecretToken, serde_secret};
///
/// let token = SecretToken::new(SecretString::new("sk-123"));
///
/// // Default sink (logs, responses, debug dumps): redacted.
/// let redacted = serde_json::to_string(&token).expect("serialize");
/// assert!(!redacted.contains("sk-123"));
///
/// // Explicit storage scope: cleartext, for encrypted-at-rest persistence.
/// let cleartext =
///     serde_secret::expose_for_serialization(|| serde_json::to_string(&token)).expect("serialize");
/// assert!(cleartext.contains("sk-123"));
/// ```
pub fn expose_for_serialization<R>(f: impl FnOnce() -> R) -> R {
    let _guard = DepthGuard::enter();
    f()
}

/// Serialize a secret value: cleartext inside an [`expose_for_serialization`]
/// scope, otherwise the `[REDACTED]` sentinel (delegating to
/// [`SecretString`]'s own redacting `Serialize`).
///
/// # Permitted cleartext sink
///
/// Cleartext is emitted only while [`expose_for_serialization`] is active on
/// the current thread. The caller proves the sink is trusted (encrypted-at-rest
/// storage or an internal full-fidelity round-trip) by entering that scope; a
/// telemetry/logging/response serializer is not a permitted sink and, by not
/// entering the scope, receives the redacted sentinel.
pub fn serialize<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    if cleartext_enabled() {
        s.serialize_str(secret.expose_secret())
    } else {
        // Delegate to SecretString's own `Serialize`, the single source of the
        // `[REDACTED]` sentinel.
        secret.serialize(s)
    }
}

/// Deserialize a string into a `SecretString`, **rejecting the `[REDACTED]`
/// sentinel**.
///
/// Delegates to [`SecretString`]'s own `Deserialize`. The rejection is the
/// loud-failure half of the gate: if a persist site ever serializes outside an
/// [`expose_for_serialization`] scope, it writes the sentinel; reading that
/// blob back then errors here instead of silently loading `[REDACTED]` as the
/// real secret.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
    SecretString::deserialize(d)
}

/// Serde helpers for `Option<SecretString>`. Use as:
/// `#[serde(with = "nebula_credential::serde_secret::option")]`.
pub mod option {
    use super::{Serialize, Serializer, cleartext_enabled};
    use crate::SecretString;
    use serde::{Deserialize, Deserializer};

    /// Serialize an optional secret: cleartext inside an
    /// [`expose_for_serialization`](super::expose_for_serialization) scope,
    /// otherwise the `[REDACTED]` sentinel for `Some` and `null` for `None`.
    pub fn serialize<S: Serializer>(
        secret: &Option<SecretString>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match secret {
            Some(secret) if cleartext_enabled() => s.serialize_str(secret.expose_secret()),
            // Redacting path delegates to `SecretString`'s own `Serialize` so
            // the `Some(_)` field still appears (redacted), preserving shape.
            Some(secret) => secret.serialize(s),
            None => s.serialize_none(),
        }
    }

    /// Deserialize an optional string into an `Option<SecretString>`, rejecting
    /// the `[REDACTED]` sentinel in the `Some` case (see the non-`option`
    /// [`deserialize`](super::deserialize) for why).
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
        Option::<SecretString>::deserialize(d)
    }
}
