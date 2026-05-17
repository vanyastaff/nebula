//! Slack webhook action.
//!
//! Slack signs requests as
//! `v0=<hex hmac_sha256(secret, "v0:" + ts + ":" + body)>` in the
//! `X-Slack-Signature` header, with the request timestamp in
//! `X-Slack-Request-Timestamp` (Unix seconds, replay window 5 min).
//! See <https://api.slack.com/authentication/verifying-requests-from-slack>.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use nebula_core::{Dependencies, action_key};
use serde::Deserialize;
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
        WebhookTriggerAdapter, verify_hmac_sha256_with_timestamp,
    },
};

const SLACK_SIG_HEADER: &str = "x-slack-signature";
const SLACK_TS_HEADER: &str = "x-slack-request-timestamp";

/// Slack webhook provider.
#[derive(Clone)]
pub struct SlackWebhookAction {
    secret: Arc<[u8]>,
    replay_window: Duration,
}

impl SlackWebhookAction {
    /// Construct a Slack webhook with the given signing secret.
    #[must_use]
    pub fn new(secret: impl Into<Arc<[u8]>>) -> Self {
        Self {
            secret: secret.into(),
            replay_window: Duration::from_mins(5),
        }
    }

    /// Replace the replay window. Slack's published default is 5 min.
    #[must_use]
    pub fn with_replay_window(mut self, window: Duration) -> Self {
        self.replay_window = window;
        self
    }
}

impl Action for SlackWebhookAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("nebula.webhook.slack"),
                "Slack Webhook",
                "Slack-flavoured signed webhook trigger.",
            )
        })
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl WebhookAction for SlackWebhookAction {
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
        // Slack expects 2xx within 3 s.
        Ok(WebhookResponse::accept(TriggerEventOutcome::emit(payload)))
    }

    async fn pre_handle(
        &self,
        request: &WebhookRequest,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<PreHandleOutcome, ActionError> {
        // Slack's `url_verification` envelope:
        // `{"type":"url_verification","challenge":"…"}`.
        // Body cap for this peek matches our typical handshake size
        // (< 1 KiB) — anything bigger is not the verification flow.
        if request.body().len() > 1024 {
            return Ok(PreHandleOutcome::Continue);
        }
        let Ok(envelope) = serde_json::from_slice::<UrlVerification>(request.body()) else {
            return Ok(PreHandleOutcome::Continue);
        };
        if envelope.kind != "url_verification" {
            return Ok(PreHandleOutcome::Continue);
        }
        debug!(provider = "slack", "url_verification handshake intercepted");
        let body = serde_json::to_vec(&serde_json::json!({
            "challenge": envelope.challenge,
        }))
        .map_err(|e| {
            ActionError::validation(
                "url_verification.response",
                ValidationReason::WrongType,
                Some(e.to_string()),
            )
        })?;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        Ok(PreHandleOutcome::RespondNow(
            WebhookHttpResponse::new(StatusCode::OK, Bytes::from(body)).with_headers(headers),
        ))
    }

    fn config(&self) -> WebhookConfig {
        let secret = Arc::clone(&self.secret);
        let replay_window = self.replay_window;
        let verifier = move |req: &WebhookRequest| -> SignatureOutcome {
            verify_hmac_sha256_with_timestamp(
                req,
                &secret,
                SLACK_SIG_HEADER,
                SLACK_TS_HEADER,
                replay_window,
                |ts, body| {
                    let mut canonical = Vec::with_capacity(4 + ts.len() + 1 + body.len());
                    canonical.extend_from_slice(b"v0:");
                    canonical.extend_from_slice(ts.as_bytes());
                    canonical.push(b':');
                    canonical.extend_from_slice(body);
                    canonical
                },
            )
            .unwrap_or(SignatureOutcome::Invalid)
        };
        WebhookConfig::new()
            .with_signature_policy(SignaturePolicy::custom(verifier))
            .with_provider(WebhookProvider::Slack)
    }
}

#[derive(Deserialize)]
struct UrlVerification<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    challenge: &'a str,
}

// ── Factory ──────────────────────────────────────────────────────────────

/// [`WebhookActionFactory`] for [`SlackWebhookAction`]. Registered
/// by the engine runtime under the `"slack"` provider kind.
#[derive(Debug, Default)]
pub struct SlackWebhookActionFactory;

impl SlackWebhookActionFactory {
    /// Construct an instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl WebhookActionFactory for SlackWebhookActionFactory {
    fn kind(&self) -> &'static str {
        "slack"
    }

    fn build(&self, spec: &WebhookActivationSpec) -> Result<BuiltWebhookHandler, FactoryError> {
        let mut action = SlackWebhookAction::new(spec.secret.clone());
        if let Some(secs) = spec.replay_window_secs {
            action = action.with_replay_window(Duration::from_secs(secs));
        }
        let config = action.config();
        Ok(BuiltWebhookHandler {
            handler: Arc::new(WebhookTriggerAdapter::new(action)),
            config,
        })
    }
}
