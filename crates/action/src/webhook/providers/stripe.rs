//! Stripe webhook action.
//!
//! Stripe signs requests with `Stripe-Signature: t=<unix>,v1=<hex>`
//! (one or more `v1=` entries during signature rotation). The signed
//! payload is `{ts}.{body}`. Default tolerance 5 min — see
//! <https://stripe.com/docs/webhooks/signatures>.

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use http::{HeaderName, StatusCode};
use nebula_core::{Dependencies, action_key};
use tracing::debug;

use crate::{
    action::Action,
    context::TriggerContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    trigger::TriggerEventOutcome,
    webhook::{
        BuiltWebhookHandler, FactoryError, PreHandleOutcome, SignatureOutcome, SignaturePolicy,
        WebhookAction, WebhookActionFactory, WebhookActivationSpec, WebhookConfig,
        WebhookHttpResponse, WebhookProvider, WebhookRequest, WebhookResponse,
        WebhookTriggerAdapter, hmac_sha256_compute, verify_tag_constant_time,
    },
};

const STRIPE_SIG_HEADER: HeaderName = HeaderName::from_static("stripe-signature");

/// Stripe webhook provider.
#[derive(Clone)]
pub struct StripeWebhookAction {
    secret: Arc<[u8]>,
    tolerance: Duration,
}

impl StripeWebhookAction {
    /// Construct a Stripe webhook with the given signing secret.
    #[must_use]
    pub fn new(secret: impl Into<Arc<[u8]>>) -> Self {
        Self {
            secret: secret.into(),
            tolerance: Duration::from_mins(5),
        }
    }

    /// Replace the tolerance window. Stripe's published default is 5
    /// min.
    #[must_use]
    pub fn with_tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = tolerance;
        self
    }
}

impl Action for StripeWebhookAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("nebula.webhook.stripe"),
                "Stripe Webhook",
                "Stripe-flavoured signed webhook trigger.",
            )
        })
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl WebhookAction for StripeWebhookAction {
    type State = ();

    async fn on_activate(
        &self,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<Self::State, ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        request: &WebhookRequest,
        _state: &Self::State,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        let payload = if request.body().is_empty() {
            serde_json::Value::Null
        } else {
            request.body_json::<serde_json::Value>().map_err(|e| {
                ActionError::validation(
                    "body",
                    ValidationReason::MalformedJson,
                    Some(e.to_string()),
                )
            })?
        };
        Ok(WebhookResponse::accept(TriggerEventOutcome::emit(payload)))
    }

    async fn pre_handle(
        &self,
        request: &WebhookRequest,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<PreHandleOutcome, ActionError> {
        // Stripe's URL-verification ping: a normal webhook envelope
        // whose `data.object` is empty / absent. Cap the body peek
        // for safety — real events typically exceed a few KiB.
        if request.body().len() > 4 * 1024 {
            return Ok(PreHandleOutcome::Continue);
        }
        let Ok(envelope) = request.body_json::<serde_json::Value>() else {
            return Ok(PreHandleOutcome::Continue);
        };
        if !is_pending_webhook_ping(&envelope) {
            return Ok(PreHandleOutcome::Continue);
        }
        debug!(provider = "stripe", "pending_webhook ping intercepted");
        Ok(PreHandleOutcome::RespondNow(WebhookHttpResponse::new(
            StatusCode::OK,
            Bytes::new(),
        )))
    }

    fn config(&self) -> WebhookConfig {
        let secret = Arc::clone(&self.secret);
        let tolerance = self.tolerance;
        let verifier = move |req: &WebhookRequest| -> SignatureOutcome {
            verify_stripe_signature(req, &secret, tolerance)
        };
        WebhookConfig::new()
            .with_signature_policy(SignaturePolicy::custom(verifier))
            .with_provider(WebhookProvider::Stripe)
    }
}

fn is_pending_webhook_ping(envelope: &serde_json::Value) -> bool {
    let obj = match envelope {
        serde_json::Value::Object(o) => o,
        _ => return false,
    };
    // Heuristic: Stripe pings carry `data.object` but the inner
    // object is empty / lacks identity fields. Treat an explicit
    // empty `data.object` or a top-level `type` == `pending_webhook`
    // as the ping flow.
    if obj.get("type").and_then(|v| v.as_str()) == Some("pending_webhook") {
        return true;
    }
    if let Some(serde_json::Value::Object(data)) = obj.get("data")
        && let Some(serde_json::Value::Object(inner)) = data.get("object")
    {
        return inner.is_empty();
    }
    false
}

fn verify_stripe_signature(
    request: &WebhookRequest,
    secret: &[u8],
    tolerance: Duration,
) -> SignatureOutcome {
    if secret.is_empty() {
        return SignatureOutcome::Invalid;
    }
    let header_value = match request.headers().get(&STRIPE_SIG_HEADER) {
        Some(v) => match v.to_str() {
            Ok(s) => s,
            Err(_) => return SignatureOutcome::Invalid,
        },
        None => return SignatureOutcome::Missing,
    };

    let parsed = match parse_stripe_signature(header_value) {
        Some(p) => p,
        None => return SignatureOutcome::Invalid,
    };

    // Replay window: against the request's received_at, not the wall
    // clock — same discipline as `verify_hmac_sha256_with_timestamp`.
    let received_secs: i64 = match request.received_at().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return SignatureOutcome::Invalid,
    };
    let skew = received_secs.saturating_sub(parsed.t);
    let tol_secs = tolerance.as_secs() as i64;
    const FUTURE_SKEW_SECS: i64 = 60;
    if skew > tol_secs || skew < -FUTURE_SKEW_SECS {
        return SignatureOutcome::Invalid;
    }

    // Canonical Stripe payload: `{t}.{body}`.
    let mut canonical = Vec::with_capacity(parsed.t_str.len() + 1 + request.body().len());
    canonical.extend_from_slice(parsed.t_str.as_bytes());
    canonical.push(b'.');
    canonical.extend_from_slice(request.body());

    let expected = hmac_sha256_compute(secret, &canonical);

    for v1 in &parsed.v1s {
        let Ok(decoded) = hex_decode(v1) else {
            continue;
        };
        if verify_tag_constant_time(&expected, &decoded) {
            return SignatureOutcome::Valid;
        }
    }
    SignatureOutcome::Invalid
}

struct ParsedStripeHeader<'a> {
    t: i64,
    t_str: &'a str,
    v1s: Vec<&'a str>,
}

fn parse_stripe_signature(s: &str) -> Option<ParsedStripeHeader<'_>> {
    let mut t_str: Option<&str> = None;
    let mut v1s: Vec<&str> = Vec::new();
    for part in s.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        match key {
            "t" => t_str = Some(value),
            "v1" => v1s.push(value),
            _ => {},
        }
    }
    let t_str = t_str?;
    let t: i64 = t_str.parse().ok()?;
    if v1s.is_empty() {
        return None;
    }
    Some(ParsedStripeHeader { t, t_str, v1s })
}

fn hex_decode(s: &str) -> Result<Vec<u8>, ()> {
    if !s.len().is_multiple_of(2) {
        return Err(());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = nibble(bytes[i]).ok_or(())?;
        let lo = nibble(bytes[i + 1]).ok_or(())?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

const fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── Factory ──────────────────────────────────────────────────────────────

/// [`WebhookActionFactory`] for [`StripeWebhookAction`]. Registered
/// by the engine runtime under the `"stripe"` provider kind.
#[derive(Debug, Default)]
pub struct StripeWebhookActionFactory;

impl StripeWebhookActionFactory {
    /// Construct an instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl WebhookActionFactory for StripeWebhookActionFactory {
    fn kind(&self) -> &'static str {
        "stripe"
    }

    fn build(&self, spec: &WebhookActivationSpec) -> Result<BuiltWebhookHandler, FactoryError> {
        let mut action = StripeWebhookAction::new(spec.secret.clone());
        if let Some(secs) = spec.replay_window_secs {
            action = action.with_tolerance(Duration::from_secs(secs));
        }
        let config = action.config();
        Ok(BuiltWebhookHandler {
            handler: Arc::new(WebhookTriggerAdapter::new(action)),
            config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stripe_signature_single_v1() {
        let parsed = parse_stripe_signature("t=1700000000,v1=deadbeef").unwrap();
        assert_eq!(parsed.t, 1_700_000_000);
        assert_eq!(parsed.v1s, vec!["deadbeef"]);
    }

    #[test]
    fn parse_stripe_signature_multi_v1() {
        let parsed = parse_stripe_signature("t=1700000000,v1=aa,v1=bb,v0=skipped").unwrap();
        assert_eq!(parsed.v1s, vec!["aa", "bb"]);
    }

    #[test]
    fn pending_webhook_detection() {
        let ping = serde_json::json!({
            "id": "evt_pending",
            "type": "charge.succeeded",
            "data": { "object": {} }
        });
        assert!(is_pending_webhook_ping(&ping));

        let normal = serde_json::json!({
            "type": "charge.succeeded",
            "data": { "object": { "id": "ch_x", "amount": 100 } },
        });
        assert!(!is_pending_webhook_ping(&normal));
    }
}
