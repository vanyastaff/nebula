//! Generic provider-agnostic webhook action.
//!
//! Verifies a vanilla HMAC over the request body (Nebula default
//! header `X-Nebula-Signature`) and optionally answers a configured
//! `?challenge=<token>` GET handshake (Microsoft Teams,
//! SharePoint-style validation). On `POST`, emits the body as a JSON
//! event into the workflow engine.

use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http::{HeaderName, Method, StatusCode};
use nebula_core::{Dependencies, action_key};
use nebula_schema::{HasSchema, ValidSchema};
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::{
    action::Action,
    context::TriggerContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    trigger::TriggerEventOutcome,
    webhook::{
        BuiltWebhookHandler, FactoryError, PreHandleOutcome, RequiredPolicy, SignaturePolicy,
        WebhookAction, WebhookActionFactory, WebhookActivationSpec, WebhookConfig,
        WebhookHttpResponse, WebhookProvider, WebhookRequest, WebhookResponse,
        WebhookTriggerAdapter,
    },
};

/// Provider-agnostic webhook action.
///
/// Defaults: HMAC SHA-256 hex over the body, header
/// `X-Nebula-Signature`, no timestamp/replay protection unless opted
/// in via [`Self::with_timestamp_header`].
#[derive(Clone)]
pub struct GenericWebhookAction {
    secret: Arc<[u8]>,
    signature_header: HeaderName,
    timestamp_header: Option<HeaderName>,
    replay_window: std::time::Duration,
    challenge_token: Option<String>,
}

impl GenericWebhookAction {
    /// Construct a generic webhook with the given HMAC secret.
    /// Empty secrets are accepted at construction time so unit tests
    /// can exercise the fail-closed default; the transport returns
    /// 500 for an actual request.
    #[must_use]
    pub fn new(secret: impl Into<Arc<[u8]>>) -> Self {
        Self {
            secret: secret.into(),
            signature_header: HeaderName::from_static("x-nebula-signature"),
            timestamp_header: None,
            replay_window: std::time::Duration::from_mins(5),
            challenge_token: None,
        }
    }

    /// Replace the signature header. Some providers ship custom
    /// names (`X-Hub-Signature-256` for GitHub-flavoured generics).
    #[must_use]
    pub fn with_signature_header(mut self, header: HeaderName) -> Self {
        self.signature_header = header;
        self
    }

    /// Opt into replay-window enforcement by providing a timestamp
    /// header to consult.
    #[must_use]
    pub fn with_timestamp_header(mut self, header: HeaderName) -> Self {
        self.timestamp_header = Some(header);
        self
    }

    /// Replace the replay window. Default 5 min.
    #[must_use]
    pub fn with_replay_window(mut self, window: std::time::Duration) -> Self {
        self.replay_window = window;
        self
    }

    /// Configure a `?challenge=<token>` GET handshake. The transport
    /// answers a matching GET with `200 <token>`; non-matching GETs
    /// return 404 from `pre_handle`.
    #[must_use]
    pub fn with_challenge_token(mut self, token: impl Into<String>) -> Self {
        self.challenge_token = Some(token.into());
        self
    }
}

impl Action for GenericWebhookAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("nebula.webhook.generic"),
                "Generic Webhook",
                "Provider-agnostic HMAC-signed webhook trigger.",
            )
        })
    }

    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }

    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl WebhookAction for GenericWebhookAction {
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
        Ok(WebhookResponse::respond(
            StatusCode::ACCEPTED,
            Bytes::new(),
            TriggerEventOutcome::emit(payload),
        ))
    }

    async fn pre_handle(
        &self,
        request: &WebhookRequest,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<PreHandleOutcome, ActionError> {
        if request.method() != Method::GET {
            return Ok(PreHandleOutcome::Continue);
        }
        let Some(token) = self.challenge_token.as_ref() else {
            // No challenge configured — GETs should be 405. Let the
            // transport's method gating turn POST-only routes into
            // 405; for our pre-handle path, return RespondNow(404).
            return Ok(PreHandleOutcome::RespondNow(WebhookHttpResponse::new(
                StatusCode::NOT_FOUND,
                Bytes::new(),
            )));
        };
        let challenge_value = request
            .query()
            .and_then(|q| extract_query_param(q, "challenge"));
        match challenge_value {
            Some(v) if v.as_bytes().ct_eq(token.as_bytes()).into() => {
                debug!(provider = "generic", "webhook challenge matched");
                Ok(PreHandleOutcome::RespondNow(WebhookHttpResponse::new(
                    StatusCode::OK,
                    Bytes::copy_from_slice(token.as_bytes()),
                )))
            },
            _ => {
                debug!(provider = "generic", "webhook challenge mismatch");
                Ok(PreHandleOutcome::RespondNow(WebhookHttpResponse::new(
                    StatusCode::NOT_FOUND,
                    Bytes::new(),
                )))
            },
        }
    }

    fn config(&self) -> WebhookConfig {
        let mut policy = RequiredPolicy::new()
            .with_secret(Arc::clone(&self.secret))
            .with_header(self.signature_header.clone())
            .with_replay_window(self.replay_window);
        if let Some(ts) = self.timestamp_header.clone() {
            policy = policy.with_timestamp_header(ts);
        }
        WebhookConfig::new()
            .with_signature_policy(SignaturePolicy::Required(policy))
            .with_provider(WebhookProvider::Generic {
                challenge_token: self.challenge_token.clone(),
            })
    }
}

/// Extract the value of `name=<value>` from a URL-encoded query
/// string. Pure helper so the action does not pull in `url::Url`
/// just to read one parameter.
fn extract_query_param(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut split = pair.splitn(2, '=');
        let k = split.next()?;
        let v = split.next().unwrap_or("");
        if k == name {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            let hi = hex_nibble(bytes[i + 1]);
            let lo = hex_nibble(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        } else if b == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── Factory ──────────────────────────────────────────────────────────────

/// [`WebhookActionFactory`] for [`GenericWebhookAction`]. Registered
/// by the engine runtime under the `"generic"` provider kind.
#[derive(Debug, Default)]
pub struct GenericWebhookActionFactory;

impl GenericWebhookActionFactory {
    /// Construct an instance. The struct is zero-sized — wrap once
    /// at startup in `Arc::new(GenericWebhookActionFactory)`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl WebhookActionFactory for GenericWebhookActionFactory {
    fn kind(&self) -> &'static str {
        "generic"
    }

    fn build(&self, spec: &WebhookActivationSpec) -> Result<BuiltWebhookHandler, FactoryError> {
        let mut action = GenericWebhookAction::new(spec.secret.clone());
        if let Some(secs) = spec.replay_window_secs {
            action = action.with_replay_window(std::time::Duration::from_secs(secs));
        }
        if let Some(header_str) = spec.timestamp_header.as_deref() {
            let parsed = HeaderName::from_bytes(header_str.as_bytes()).map_err(|_| {
                FactoryError::InvalidSpec {
                    kind: "generic",
                    reason: format!("invalid timestamp_header: {header_str:?}"),
                }
            })?;
            action = action.with_timestamp_header(parsed);
        }
        if let Some(serde_json::Value::Object(map)) = spec.provider_config.as_ref()
            && let Some(serde_json::Value::String(token)) = map.get("challenge_token")
        {
            action = action.with_challenge_token(token.clone());
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
    fn extract_query_param_basic() {
        assert_eq!(
            extract_query_param("challenge=abc", "challenge").as_deref(),
            Some("abc"),
        );
        assert_eq!(
            extract_query_param("foo=1&challenge=abc&bar=2", "challenge").as_deref(),
            Some("abc"),
        );
        assert_eq!(extract_query_param("foo=1", "challenge"), None);
    }

    #[test]
    fn extract_query_param_percent_decoded() {
        assert_eq!(
            extract_query_param("challenge=hello%20world", "challenge").as_deref(),
            Some("hello world"),
        );
        assert_eq!(
            extract_query_param("challenge=a+b", "challenge").as_deref(),
            Some("a b"),
        );
    }
}
