//! Webhook trigger domain — DX trait, adapter, and HMAC signature primitives.
//!
//! Groups everything an action author needs to implement a webhook
//! trigger into one file:
//!
//! - [`WebhookAction`] — DX trait with the register / handle / unregister lifecycle. Implement this
//!   and register via `registry.register_webhook(action)`.
//! - [`WebhookTriggerAdapter`] — bridges a typed `WebhookAction` to the
//!   [`TriggerHandler`](crate::trigger::TriggerHandler) dyn contract. Stores state from
//!   `on_activate` in a `RwLock<Option<Arc<State>>>`, rejects double-start with
//!   `ActionError::Fatal`, and cleans up the orphan registration on lost-race rollback.
//! - [`verify_hmac_sha256`], [`hmac_sha256_compute`], [`verify_tag_constant_time`],
//!   [`SignatureOutcome`] — constant-time HMAC primitives. Use `verify_hmac_sha256` for
//!   GitHub-style `sha256=…` bare-hex signatures; reach for the lower-level pair for Stripe / Slack
//!   schemes that sign a derived payload.
//!
//! # Security
//!
//! **Never compare signatures with `==` or `str::eq`.** The byte-wise
//! short-circuit in `PartialEq` leaks the secret one prefix byte at a
//! time and is exploitable over the network. Always use the helpers in
//! this module.
//!
//! Stripe/Slack helpers are intentionally NOT provided: their correct
//! implementation requires a time source and a tolerance window to
//! prevent replay, and wrapping that correctly would pull platform
//! clocks into this module. Build them in your action on top of the
//! primitives.

use std::{fmt, future::Future, sync::Arc};

use async_trait::async_trait;
use hmac::{Hmac, KeyInit, Mac};
use parking_lot::RwLock;
use serde::{Serialize, de::DeserializeOwned};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::{
    action::Action,
    context::TriggerContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    trigger::{IncomingEvent, TriggerEventOutcome, TriggerHandler},
};

// ── DX trait ────────────────────────────────────────────────────────────────

/// Webhook trigger — register/handle/unregister lifecycle.
///
/// Implement `handle_request` (required), and optionally `on_activate`/`on_deactivate`.
/// Register via `registry.register_webhook(action)`.
///
/// State from `on_activate` is stored by the adapter and passed to `handle_request`
/// (by reference) and `on_deactivate` (by value). For mutable per-event state, wrap
/// fields in `Mutex` or atomic types inside `Self::State`.
///
/// # Example
///
/// Use [`verify_hmac_sha256`] for constant-time signature verification —
/// naive `==` comparison on HMAC digests leaks the secret via a
/// prefix-length timing side-channel.
///
/// ```rust,ignore
/// use nebula_action::webhook::{WebhookAction, verify_hmac_sha256};
/// use nebula_action::trigger::{IncomingEvent, TriggerEventOutcome};
///
/// struct GitHubWebhook { secret: Vec<u8> }
///
/// impl WebhookAction for GitHubWebhook {
///     type State = WebhookReg;
///
///     async fn on_activate(&self, ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
///         Ok(WebhookReg { hook_id: register(ctx).await? })
///     }
///
///     async fn handle_request(&self, event: &IncomingEvent, _state: &Self::State, _ctx: &TriggerContext)
///         -> Result<TriggerEventOutcome, ActionError> {
///         let outcome = verify_hmac_sha256(event, &self.secret, "X-Hub-Signature-256")?;
///         if !outcome.is_valid() {
///             return Ok(TriggerEventOutcome::skip());
///         }
///         Ok(TriggerEventOutcome::emit(event.body_json()?))
///     }
///
///     async fn on_deactivate(&self, state: WebhookReg, _ctx: &TriggerContext) -> Result<(), ActionError> {
///         delete_hook(&state.hook_id).await
///     }
/// }
/// ```
pub trait WebhookAction: Action + Send + Sync + 'static {
    /// Persisted state between activate/deactivate (e.g., webhook registration ID).
    type State: Serialize + DeserializeOwned + Default + Clone + Send + Sync;

    /// Register webhook with external service. Returns state to persist.
    ///
    /// Default: returns `State::default()` (no-op activation).
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if registration fails.
    fn on_activate(
        &self,
        _ctx: &TriggerContext,
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send {
        async { Ok(Self::State::default()) }
    }

    /// Handle an incoming event. Return `Emit` to start a workflow, `Skip` to filter.
    ///
    /// State from `on_activate` is passed by reference. The adapter clones an
    /// internal Arc cheaply before this call — no contention with start/stop.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if event processing fails.
    fn handle_request(
        &self,
        event: &IncomingEvent,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<TriggerEventOutcome, ActionError>> + Send;

    /// Unregister webhook on deactivation.
    ///
    /// Receives the state stored from `on_activate`. Default: no-op.
    /// Not called if `on_activate` was never called (stop without start).
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if unregistration fails.
    fn on_deactivate(
        &self,
        _state: Self::State,
        _ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }
}

// ── WebhookTriggerAdapter ────────────────────────────────────────────────────

/// Wraps a [`WebhookAction`] as a [`dyn TriggerHandler`] with state management.
///
/// Stores state from `on_activate` in a `RwLock<Option<Arc<State>>>`. `handle_event`
/// clones the `Arc` under the read lock and releases the lock BEFORE awaiting
/// `handle_request` — prevents deadlock with concurrent `start`/`stop` taking a
/// write lock (parking_lot RwLock is not reentrant and not async-aware).
///
/// `handle_event` before `start()` returns `ActionError::Fatal` (no silent default state).
///
/// Created automatically by `nebula_runtime::ActionRegistry::register_webhook`.
pub struct WebhookTriggerAdapter<A: WebhookAction> {
    action: A,
    state: RwLock<Option<Arc<A::State>>>,
}

impl<A: WebhookAction> WebhookTriggerAdapter<A> {
    /// Wrap a typed webhook action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self {
            action,
            state: RwLock::new(None),
        }
    }
}

#[async_trait]
impl<A> TriggerHandler for WebhookTriggerAdapter<A>
where
    A: WebhookAction + Send + Sync + 'static,
    A::State: Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        // Reject double-start: previous state must be stopped first.
        // Silently overwriting would leak external webhook registrations
        // (GitHub/Slack/Stripe) — the old hook stays live and stop() only
        // deactivates the last one.
        if self.state.read().is_some() {
            return Err(ActionError::fatal(
                "webhook trigger already started; call stop() before start() again",
            ));
        }

        let new_state = self.action.on_activate(ctx).await?;

        // Re-check under the write lock to close the race between the
        // read-guard drop above and the write below. The `rollback_state`
        // dance keeps the parking_lot guard strictly inside the block so
        // it cannot sit across the `.await` on `on_deactivate` — holding
        // a non-Send, non-async guard across a suspension point would
        // make the whole future `!Send`.
        let rollback_state = {
            let mut guard = self.state.write();
            if guard.is_some() {
                // Another task raced us and already stored state. We own
                // `new_state` — nobody else has the Arc yet — so we must
                // tear it down. Return it from the block for the await.
                Some(new_state)
            } else {
                *guard = Some(Arc::new(new_state));
                None
            }
        };

        if let Some(orphan) = rollback_state {
            // Tear down the state we just created on the lost-race
            // branch. We are already returning a Fatal to the caller,
            // so we must not mask that by propagating the rollback's
            // own error — but silently dropping it would hide a leaked
            // external webhook registration. Log and continue to
            // surface the original double-start error.
            if let Err(e) = self.action.on_deactivate(orphan, ctx).await {
                tracing::warn!(
                    action = %self.action.metadata().key,
                    error = %e,
                    "webhook rollback on_deactivate failed after double-start race; \
                     external hook may leak"
                );
            }
            return Err(ActionError::fatal(
                "webhook trigger already started; call stop() before start() again",
            ));
        }
        Ok(())
    }

    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
        let stored = self.state.write().take();
        match stored {
            Some(arc_state) => {
                // In normal flow no concurrent handle_event holds the Arc,
                // so `unwrap_or_clone` takes the cheap path; on a lost
                // race with an in-flight event it clones once.
                let owned = Arc::unwrap_or_clone(arc_state);
                self.action.on_deactivate(owned, ctx).await
            }
            // stop() without prior start() — no-op, nothing to deactivate.
            None => Ok(()),
        }
    }

    fn accepts_events(&self) -> bool {
        true
    }

    /// Route an incoming event to the typed `handle_request`.
    ///
    /// # Stop vs in-flight event race
    ///
    /// `handle_event` clones the `Arc<State>` under a read lock and
    /// releases the lock BEFORE awaiting `handle_request`. If `stop()`
    /// wins the race, it takes the owned `Arc` and calls
    /// `on_deactivate` on the result — meanwhile this in-flight
    /// request still holds its independent `Arc` clone and runs
    /// `handle_request` against it. Two observers of the same logical
    /// state briefly exist side by side.
    ///
    /// This is benign for webhook authors whose `State` does not
    /// invalidate shared substate on deactivation (the common case —
    /// deactivation typically just unregisters an external hook).
    /// Authors whose `on_deactivate` actively tears down shared
    /// resources (closes a pooled connection held inside `State`,
    /// etc.) must either wrap those resources in `Arc`-friendly
    /// handles that tolerate being read from a stale clone, or
    /// coordinate shutdown externally.
    async fn handle_event(
        &self,
        event: IncomingEvent,
        ctx: &TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        // Clone Arc under read lock; the guard drops at end of statement BEFORE
        // the await on handle_request. Holding a parking_lot guard across .await
        // would be unsound (non-Send) and risk re-entry panic with start/stop.
        let state = self.state.read().as_ref().cloned().ok_or_else(|| {
            ActionError::fatal("handle_event called before start — no state available")
        })?;

        self.action.handle_request(&event, &state, ctx).await
    }
}

impl<A: WebhookAction> fmt::Debug for WebhookTriggerAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookTriggerAdapter")
            .field("action", &self.action.metadata().key)
            .finish_non_exhaustive()
    }
}

// ── HMAC signature primitives ────────────────────────────────────────────────

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

    // Reason: `Hmac::new_from_slice` returns `InvalidLength` only for
    // block-cipher MACs (CMAC etc.). For HMAC (RFC 2104) any key
    // length is accepted — oversize keys are hashed to block size,
    // undersize keys are zero-padded. Surfacing this as
    // `ActionError::Fatal` would poison callers with an impossible
    // error variant. The empty-secret guard above is the only length
    // check HMAC actually needs.
    #[allow(clippy::expect_used)]
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length (RFC 2104)");
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
    // Reason: see `verify_hmac_sha256` — HMAC construction is
    // structurally infallible (RFC 2104). Returning `Result` from a
    // pure primitive for an unreachable error case would force every
    // caller to handle an impossibility.
    #[allow(clippy::expect_used)]
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length (RFC 2104)");
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
