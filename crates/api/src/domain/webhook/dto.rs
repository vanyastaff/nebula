//! DTOs for the webhook registration endpoint.
//!
//! # Security invariants
//!
//! - The **request** DTO carries no server authority (`scope`, `mode`,
//!   `token_hash`, `nonce`, `secret`, or `secret_id`). Provider configuration
//!   is untrusted caller input. Its shape is bounded here, while the selected
//!   trusted factory must validate its complete non-secret schema before any
//!   durable write. Its OpenAPI and `Debug` representations remain
//!   secret-aware as defense in depth.
//! - The **response** DTO exposes `signing_secret` exactly once (201 body).
//!   No read-back path returns it.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use zeroize::Zeroize;

fn write_only_object_schema() -> utoipa::openapi::schema::Object {
    utoipa::openapi::schema::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::Object)
        .write_only(Some(true))
        .description(Some(
            "Provider-specific non-secret configuration accepted only during registration; the selected trusted factory validates it before persistence.",
        ))
        .build()
}

// ── Request ───────────────────────────────────────────────────────────────────

/// Request body for `POST /orgs/{org}/workspaces/{ws}/webhooks`.
///
/// The scope is server-derived from the authenticated principal — it MUST NOT
/// be supplied here.  `mode` is always `Prod` for this endpoint; there is no
/// caller override.
#[derive(Deserialize, ToSchema)]
pub struct RegisterWebhookRequest {
    /// The workflow this trigger belongs to.
    pub workflow_id: String,
    /// The stable `NodeKey` id of the trigger binding within the workflow
    /// definition's `trigger_bindings` array.  Must match a binding entry;
    /// cross-scope or absent ids return 404.
    pub trigger_id: String,
    /// Factory provider tag — selects which `WebhookActionFactory` builds the
    /// handler.  Examples: `"generic"`, `"slack"`, `"stripe"`.
    pub provider: String,
    /// Optional replay-window override in seconds.  `None` keeps the
    /// action's compiled-in default.
    #[serde(default)]
    pub replay_window_secs: Option<u64>,
    /// Optional timestamp header override (lower-cased canonical form).
    #[serde(default)]
    pub timestamp_header: Option<String>,
    /// Optional provider-specific, non-secret configuration blob. The selected
    /// trusted factory validates it before any write. Built-in providers accept
    /// only an omitted or empty object; authority belongs in an explicit
    /// credential seam, never this blob.
    #[serde(default)]
    #[schema(schema_with = write_only_object_schema)]
    pub provider_config: Option<serde_json::Value>,
    /// Optional per-key rate-limit budget, in requests per minute.
    #[serde(default)]
    pub rate_limit_per_minute: Option<u64>,
}

impl RegisterWebhookRequest {
    /// Bound the transport-level shape before delegating its complete schema to
    /// the selected trusted provider factory.
    ///
    /// The walk is iterative so adversarial JSON cannot recurse through the
    /// process stack. `serde_json` and the router body limit already bound the
    /// encoded input; the node cap also bounds validation work and future
    /// deserializers with different recursion policies.
    pub(crate) fn validate_provider_config_shape(&self) -> Result<(), &'static str> {
        const MAX_CONFIG_NODES: usize = 4_096;

        let Some(config) = self.provider_config.as_ref() else {
            return Ok(());
        };
        if !config.is_object() {
            return Err("provider_config must be a JSON object");
        }

        let mut pending = vec![config];
        let mut visited = 0_usize;
        while let Some(value) = pending.pop() {
            visited = visited.saturating_add(1);
            if visited > MAX_CONFIG_NODES {
                return Err("provider_config is too complex");
            }

            match value {
                serde_json::Value::Object(map) => pending.extend(map.values()),
                serde_json::Value::Array(values) => pending.extend(values),
                _ => {},
            }
        }

        Ok(())
    }
}

impl std::fmt::Debug for RegisterWebhookRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisterWebhookRequest")
            .field("workflow_id", &self.workflow_id)
            .field("trigger_id", &self.trigger_id)
            .field("provider", &self.provider)
            .field("replay_window_secs", &self.replay_window_secs)
            .field("timestamp_header", &self.timestamp_header)
            .field(
                "provider_config",
                &self.provider_config.as_ref().map(|_| "[redacted]"),
            )
            .field("rate_limit_per_minute", &self.rate_limit_per_minute)
            .finish()
    }
}

// ── Response ──────────────────────────────────────────────────────────────────

/// Response body for a successful webhook registration (HTTP 201).
///
/// `signing_secret` is returned **exactly once** in this response.
/// No subsequent GET returns it — store it immediately or re-register to rotate.
#[derive(Serialize, ToSchema)]
pub struct RegisterWebhookResponse {
    /// The fully-qualified HTTPS URL the external provider should POST to.
    /// The embedded nonce makes the full URL a bearer capability; never log it.
    #[schema(format = "uri", read_only = true)]
    pub webhook_url: String,
    /// The `whsec_<base64>` HMAC signing secret.  Returned **once only**;
    /// not persisted in plaintext anywhere in Nebula.
    #[schema(format = "password", read_only = true)]
    pub signing_secret: String,
    /// Opaque activation identity (the trigger UUID as a string).  Use this
    /// to correlate log entries and to identify this activation in future
    /// management operations.
    pub activation_id: String,
}

impl std::fmt::Debug for RegisterWebhookResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisterWebhookResponse")
            .field("webhook_url", &"[redacted]")
            .field("signing_secret", &"[redacted]")
            .field("activation_id", &self.activation_id)
            .finish()
    }
}

impl Drop for RegisterWebhookResponse {
    fn drop(&mut self) {
        self.webhook_url.zeroize();
        self.signing_secret.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::{RegisterWebhookRequest, RegisterWebhookResponse};

    static_assertions::assert_not_impl_any!(RegisterWebhookResponse: Clone);

    #[test]
    fn register_response_debug_redacts_signing_secret() {
        const CANARY: &str = "whsec_WEBHOOK_AUTHORITY_CANARY-391b";
        let response = RegisterWebhookResponse {
            webhook_url: format!("https://nebula.example/hooks/{CANARY}"),
            signing_secret: CANARY.to_owned(),
            activation_id: "activation-example".to_owned(),
        };

        let debug = format!("{response:?}");
        assert!(debug.contains("RegisterWebhookResponse"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(CANARY));
    }

    #[test]
    fn register_request_debug_redacts_provider_config() {
        const CANARY: &str = "WEBHOOK_PROVIDER_CONFIG_CANARY-a57c";
        let request = RegisterWebhookRequest {
            workflow_id: "workflow-example".to_owned(),
            trigger_id: "trigger-example".to_owned(),
            provider: "generic".to_owned(),
            replay_window_secs: None,
            timestamp_header: None,
            provider_config: Some(serde_json::json!({ "challenge_token": CANARY })),
            rate_limit_per_minute: None,
        };

        let debug = format!("{request:?}");
        assert!(debug.contains("RegisterWebhookRequest"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(CANARY));
    }

    #[test]
    fn provider_config_shape_accepts_bounded_opaque_objects_for_factory_validation() {
        let request = RegisterWebhookRequest {
            workflow_id: "workflow-example".to_owned(),
            trigger_id: "trigger-example".to_owned(),
            provider: "generic".to_owned(),
            replay_window_secs: None,
            timestamp_header: None,
            provider_config: Some(serde_json::json!({
                "provider_specific": [{ "nested": "value" }]
            })),
            rate_limit_per_minute: None,
        };

        assert_eq!(request.validate_provider_config_shape(), Ok(()));
    }

    #[test]
    fn provider_config_shape_rejects_non_object_roots() {
        let request = RegisterWebhookRequest {
            workflow_id: "workflow-example".to_owned(),
            trigger_id: "trigger-example".to_owned(),
            provider: "example".to_owned(),
            replay_window_secs: None,
            timestamp_header: None,
            provider_config: Some(serde_json::json!(["not", "an", "object"])),
            rate_limit_per_minute: None,
        };

        assert_eq!(
            request.validate_provider_config_shape(),
            Err("provider_config must be a JSON object")
        );
    }
}
