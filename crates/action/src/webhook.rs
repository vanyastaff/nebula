//! Webhook signature verification helpers.
//!
//! Provides constant-time HMAC primitives for action authors implementing
//! [`WebhookAction::handle_request`](crate::trigger::WebhookAction::handle_request).
//! All comparisons delegate to `hmac::Mac::verify_slice`, which in turn
//! uses `subtle::ConstantTimeEq` — the prefix-length timing side-channel
//! that defeats naïve `==` comparison on HMAC digests is closed here.
//!
//! # Security
//!
//! **Never compare signatures with `==` or `str::eq`.** The byte-wise
//! short-circuit in `PartialEq` leaks the secret one prefix byte at a
//! time and is exploitable over the network. Always use the helpers in
//! this module.
//!
//! ## Supported schemes
//!
//! - [`verify_hmac_sha256`] — bare hex digest OR `sha256=…` prefixed form
//!   (GitHub webhook style). Single call, tri-state outcome.
//! - [`hmac_sha256_compute`] + [`verify_tag_constant_time`] — escape hatch
//!   for schemes that sign a derived payload rather than the raw body
//!   (Stripe `t=…,v1=…`, Slack `v0:{timestamp}:{body}`, etc). You build
//!   the signed payload and compare the tag yourself.
//!
//! Stripe/Slack helpers are intentionally NOT provided: their correct
//! implementation requires a time source and a tolerance window to
//! prevent replay, and wrapping that correctly would pull platform
//! clocks into this module. Build them in your action on top of the
//! primitives.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::{ActionError, ValidationReason};
use crate::handler::IncomingEvent;

type HmacSha256 = Hmac<Sha256>;

/// Outcome of a signature verification attempt.
///
/// `Missing` and `Invalid` are distinct so callers can decide policy:
/// a multi-tenant webhook endpoint may want to `Skip` on `Missing`
/// (not our event) but `Skip` on `Invalid` too (tampered), while a
/// strict endpoint may want to log or reject on `Invalid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignatureOutcome {
    /// Signature header present and matches the computed HMAC.
    Valid,
    /// Signature header is absent from the event.
    Missing,
    /// Signature header is present but does not match — bad hex, wrong
    /// length, or mismatched digest.
    Invalid,
}

impl SignatureOutcome {
    /// `true` only if the signature was present AND matched.
    ///
    /// Use this as the default "is it safe to emit the event" guard:
    ///
    /// ```ignore
    /// if !verify_hmac_sha256(event, secret, "X-Hub-Signature-256")?.is_valid() {
    ///     return Ok(TriggerEventOutcome::skip());
    /// }
    /// ```
    #[must_use]
    pub fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Verify an HMAC-SHA256 signature from a named header against the
/// event body.
///
/// Accepts either bare hex (`"abcd1234…"`) or a prefixed form
/// (`"sha256=abcd…"`). The prefix, if any, is stripped before hex
/// decoding.
///
/// # Arguments
///
/// - `event`  — the incoming webhook event (body + headers)
/// - `secret` — shared HMAC key (typically from a credential)
/// - `header` — header name carrying the signature, e.g. `"X-Hub-Signature-256"`
///
/// Header lookup is case-insensitive.
///
/// # Returns
///
/// [`SignatureOutcome::Valid`] / `Missing` / `Invalid`. Never panics,
/// never leaks length via timing — digest comparison delegates to
/// [`hmac::Mac::verify_slice`] which uses `subtle::ConstantTimeEq`.
///
/// # Errors
///
/// Returns [`ActionError::Validation`] only if `secret` is empty. An
/// empty HMAC key silently produces a valid MAC for any input — almost
/// always a misconfiguration, worth surfacing early as a fatal-for-this-
/// event failure rather than a silent accept.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_action::webhook::verify_hmac_sha256;
///
/// async fn handle_request(
///     &self,
///     event: &IncomingEvent,
///     state: &Self::State,
///     _ctx: &TriggerContext,
/// ) -> Result<TriggerEventOutcome, ActionError> {
///     let outcome = verify_hmac_sha256(event, state.secret.as_bytes(), "X-Hub-Signature-256")?;
///     if !outcome.is_valid() {
///         return Ok(TriggerEventOutcome::skip());
///     }
///     Ok(TriggerEventOutcome::emit(event.body_json()?))
/// }
/// ```
pub fn verify_hmac_sha256(
    event: &IncomingEvent,
    secret: &[u8],
    header: &str,
) -> Result<SignatureOutcome, ActionError> {
    if secret.is_empty() {
        return Err(ActionError::validation(
            "webhook.secret",
            ValidationReason::MissingField,
            Some("webhook signature verification requires a non-empty HMAC secret".to_string()),
        ));
    }

    let Some(sig_header) = event.header(header) else {
        return Ok(SignatureOutcome::Missing);
    };

    // Strip the common GitHub-style prefix. Other schemes that embed
    // metadata in the header (Stripe `t=…,v1=…`) are not handled here —
    // use `hmac_sha256_compute` + `verify_tag_constant_time` directly.
    let sig_hex = sig_header
        .strip_prefix("sha256=")
        .unwrap_or(sig_header)
        .trim();

    let Ok(expected) = hex::decode(sig_hex) else {
        return Ok(SignatureOutcome::Invalid);
    };

    // `new_from_slice` is infallible for HMAC — it accepts any key
    // length, including longer-than-block-size (the implementation
    // hashes oversize keys into block size). The empty-secret guard
    // above is the only length check we actually need.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&event.body);

    Ok(match mac.verify_slice(&expected) {
        Ok(()) => SignatureOutcome::Valid,
        Err(_) => SignatureOutcome::Invalid,
    })
}

/// Compute a raw HMAC-SHA256 tag over arbitrary bytes.
///
/// Escape hatch for signature schemes not handled by
/// [`verify_hmac_sha256`]. Build the signed payload yourself (for
/// example, Stripe's `{timestamp}.{body}` or Slack's
/// `v0:{timestamp}:{body}`), then compare the result against the
/// header-provided tag with [`verify_tag_constant_time`].
///
/// # Panics
///
/// Never — `Hmac::new_from_slice` accepts any key length for HMAC.
#[must_use]
pub fn hmac_sha256_compute(secret: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(payload);
    mac.finalize().into_bytes().into()
}

/// Constant-time tag comparison.
///
/// Use with [`hmac_sha256_compute`] for custom signature schemes.
/// Delegates to `subtle::ConstantTimeEq`; returns `false` on length
/// mismatch without branching on content, so neither the length nor
/// the bytes leak via timing.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_action::webhook::{hmac_sha256_compute, verify_tag_constant_time};
///
/// // Stripe-style "t=…,v1=…" signature.
/// let signed_payload = format!("{timestamp}.{}", std::str::from_utf8(body).unwrap());
/// let expected = hmac_sha256_compute(secret, signed_payload.as_bytes());
/// let provided = hex::decode(header_v1).unwrap_or_default();
/// if !verify_tag_constant_time(&expected, &provided) {
///     return Ok(TriggerEventOutcome::skip());
/// }
/// ```
#[must_use]
pub fn verify_tag_constant_time(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}
