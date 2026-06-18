//! `WebhookActivationSpec` — operator-configured webhook trigger spec
//! persisted under `triggers.config.webhook_activation`.
//!
//! # JSONB namespace
//!
//! `triggers.config` is a single `JSONB` column shared across every
//! `triggers.kind` (`manual`, `cron`, `webhook`, `event`, `polling`).
//! To prevent accidental field collisions across kinds, each kind owns
//! a top-level namespace key inside the JSON object. For
//! `kind = 'webhook'`, the spec lives under the `webhook_activation`
//! key:
//!
//! ```json
//! {
//!   "webhook_activation": {
//!     "provider": "slack",
//!     "secret_id":   "cred_01J0...",
//!     "replay_window_secs": 300,
//!     "timestamp_header": "x-slack-request-timestamp",
//!     "timestamp_format": "unix_seconds",
//!     "provider_config": { "challenge_token": "abc" },
//!     "rate_limit_per_minute": 600
//!   }
//! }
//! ```
//!
//! Cron triggers keep their `schedule` / `timezone` keys at the top
//! level of the same JSON object; the `webhook_activation` key is
//! ignored by every consumer that is not the webhook bootstrap
//! pathway. PG migration `0025_triggers_config_namespace_comment.sql`
//! attaches a `COMMENT ON COLUMN triggers.config` so DBA tooling sees
//! the contract.
//!
//! # Why a sibling row, not a column
//!
//! Adding a typed column-per-field would balloon the `triggers` table
//! across kinds; the JSONB lane keeps schema drift inside the
//! application without losing operator readability (every spec key is
//! human-grepable in DB dumps).
//!
//! # Cross-layer contract
//!
//! `secret_id` is the **storage-layer identifier** of a credential
//! registered with `nebula-credential`. The webhook factory resolves
//! it to the actual HMAC secret at registration time; storage never
//! sees the raw bytes.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Top-level JSONB key used to namespace webhook activation specs
/// inside `triggers.config`.
///
/// Constants for namespace keys live next to the row type so encoder
/// and decoder agree on the same string without re-typing it.
pub const WEBHOOK_ACTIVATION_KEY: &str = "webhook_activation";

/// Activation parameters for an operator-configured webhook trigger.
///
/// Encoded under [`WEBHOOK_ACTIVATION_KEY`] inside `triggers.config`.
/// See module docs for the on-disk JSON shape and namespace
/// conventions.
///
/// # Forwards compatibility
///
/// `#[non_exhaustive]` so future provider knobs (per-action TLS pin,
/// audit-only mode) can land without breaking external row decoders.
/// Construct via [`WebhookActivationSpec::new`] and chain `with_*`
/// builders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WebhookActivationSpec {
    /// Provider tag selecting which `WebhookActionFactory` instantiates
    /// the trigger handler at registration time. Matches the
    /// `WebhookActionFactory::kind` returned by the factory
    /// (`"slack"` / `"stripe"` / `"generic"`).
    ///
    /// Renamed from `action_kind` (ADR-0101): the old name collided with
    /// the `ActionKind` enum being introduced in ADR-0098.
    /// `#[serde(alias = "action_kind")]` preserves read-compat with any
    /// rows serialised under the old name.
    #[serde(alias = "action_kind")]
    pub provider: String,

    /// Storage identifier of the HMAC secret credential. Resolved
    /// against the credential registry by the webhook factory at
    /// activation time. Storage layer **must not** inline raw secret
    /// bytes — credential registry holds secrets at rest.
    pub secret_id: String,

    /// Optional override for the signature replay window. `None`
    /// keeps the action's compiled-in default (Slack/Stripe both ship
    /// 5 min). Stored in seconds for the same reason every other
    /// duration in the schema is — JSON numbers and DB integer columns
    /// agree on whole seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_window_secs: Option<u64>,

    /// Optional override for the timestamp header name. `None`
    /// disables replay-window enforcement on the wire (the action's
    /// default takes over). Lower-cased canonical form is recommended
    /// to match `http::HeaderName` rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_header: Option<String>,

    /// Optional override for timestamp encoding. `None` falls through
    /// to the action's default ([`WebhookTimestampFormat::UnixSeconds`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_format: Option<WebhookTimestampFormat>,

    /// Provider-specific configuration blob — the factory decodes it
    /// according to `action_kind`. Keeps provider knobs (Slack
    /// channel filter, Stripe API version pin) out of the top-level
    /// schema until they earn a typed home.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config: Option<serde_json::Value>,

    /// Optional per-key rate-limit budget, in requests per minute.
    /// `None` keeps the transport's compiled-in default (configured
    /// at the API layer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u64>,
}

impl WebhookActivationSpec {
    /// Construct a spec with the required fields populated. Optional
    /// knobs default to `None`; chain `with_*` to set them.
    ///
    /// `provider` is the factory-kind tag (e.g. `"generic"`, `"slack"`).
    #[must_use]
    pub fn new(provider: impl Into<String>, secret_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            secret_id: secret_id.into(),
            replay_window_secs: None,
            timestamp_header: None,
            timestamp_format: None,
            provider_config: None,
            rate_limit_per_minute: None,
        }
    }

    /// Set the replay window (in seconds).
    #[must_use]
    pub fn with_replay_window_secs(mut self, secs: u64) -> Self {
        self.replay_window_secs = Some(secs);
        self
    }

    /// Set the timestamp header name (lower-cased canonical form
    /// recommended).
    #[must_use]
    pub fn with_timestamp_header(mut self, header: impl Into<String>) -> Self {
        self.timestamp_header = Some(header.into());
        self
    }

    /// Set the timestamp encoding.
    #[must_use]
    pub fn with_timestamp_format(mut self, format: WebhookTimestampFormat) -> Self {
        self.timestamp_format = Some(format);
        self
    }

    /// Attach a provider-specific configuration blob.
    #[must_use]
    pub fn with_provider_config(mut self, config: serde_json::Value) -> Self {
        self.provider_config = Some(config);
        self
    }

    /// Set the per-key rate-limit budget (requests per minute).
    #[must_use]
    pub fn with_rate_limit_per_minute(mut self, rpm: u64) -> Self {
        self.rate_limit_per_minute = Some(rpm);
        self
    }

    /// Decode a `triggers.config` JSON value into a spec.
    ///
    /// Looks up the [`WEBHOOK_ACTIVATION_KEY`] sub-object, then
    /// `serde_json::from_value`'s it. Returns `Ok(None)` when the key
    /// is absent (e.g. cron triggers) so callers can distinguish
    /// "wrong kind" from "decode failed".
    ///
    /// # Errors
    ///
    /// Returns [`WebhookActivationSpecError`] when the namespace key
    /// is present but the body fails to decode (missing required
    /// fields, type mismatches, unknown timestamp formats).
    pub fn from_trigger_config(
        config: &serde_json::Value,
    ) -> Result<Option<Self>, WebhookActivationSpecError> {
        let Some(body) = config.get(WEBHOOK_ACTIVATION_KEY) else {
            return Ok(None);
        };
        serde_json::from_value(body.clone())
            .map(Some)
            .map_err(|err| WebhookActivationSpecError::Decode {
                reason: err.to_string(),
            })
    }

    /// Encode the spec into a `triggers.config` JSON value, preserving
    /// any sibling kind-namespaced keys already present in `existing`.
    ///
    /// Use this when patching the trigger's config in place — the
    /// returned object overwrites only the [`WEBHOOK_ACTIVATION_KEY`]
    /// branch.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookActivationSpecError::Encode`] if the spec
    /// itself fails to serialize.
    pub fn write_into_trigger_config(
        &self,
        existing: serde_json::Value,
    ) -> Result<serde_json::Value, WebhookActivationSpecError> {
        let mut object = match existing {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            other => {
                return Err(WebhookActivationSpecError::Encode {
                    reason: format!(
                        "triggers.config must be a JSON object, got: {}",
                        other_type_name(&other),
                    ),
                });
            },
        };
        let body =
            serde_json::to_value(self).map_err(|err| WebhookActivationSpecError::Encode {
                reason: err.to_string(),
            })?;
        object.insert(WEBHOOK_ACTIVATION_KEY.to_string(), body);
        Ok(serde_json::Value::Object(object))
    }
}

/// Wire encoding of `nebula_action::webhook::TimestampFormat` for
/// the storage layer.
///
/// Storage cannot depend on `nebula-action` (sibling layer); the
/// factory at the API layer converts between the two. `serde` uses
/// snake-case so the JSON is operator-readable
/// (`"unix_seconds"` / `"unix_millis"` / `"rfc3339"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WebhookTimestampFormat {
    /// Unix epoch seconds. Slack and Stripe default.
    #[default]
    UnixSeconds,
    /// Unix epoch milliseconds. Used by some bespoke providers.
    UnixMillis,
    /// RFC 3339 / ISO 8601 timestamps.
    Rfc3339,
}

/// Failure modes for [`WebhookActivationSpec::from_trigger_config`]
/// and [`WebhookActivationSpec::write_into_trigger_config`].
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum WebhookActivationSpecError {
    /// `triggers.config.webhook_activation` was present but failed to
    /// decode. `reason` carries the `serde_json` error message; safe
    /// to surface in operator-facing logs (no secret material).
    #[error("webhook activation spec decode failed: {reason}")]
    Decode {
        /// One-line cause from `serde_json`.
        reason: String,
    },
    /// `triggers.config` was malformed (not an object) or the spec
    /// failed to encode.
    #[error("webhook activation spec encode failed: {reason}")]
    Encode {
        /// One-line cause.
        reason: String,
    },
}

fn other_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_minimal_spec() {
        let spec = WebhookActivationSpec::new("slack", "cred_01J0").with_replay_window_secs(300);
        let config = spec
            .write_into_trigger_config(serde_json::Value::Null)
            .expect("encode");
        let decoded = WebhookActivationSpec::from_trigger_config(&config)
            .expect("decode")
            .expect("present");
        assert_eq!(decoded.provider, "slack");
        assert_eq!(decoded.secret_id, "cred_01J0");
        assert_eq!(decoded.replay_window_secs, Some(300));
    }

    #[test]
    fn preserves_sibling_kind_keys() {
        let existing = serde_json::json!({
            "schedule": "*/5 * * * *",
            "timezone": "UTC",
        });
        let spec = WebhookActivationSpec::new("generic", "cred_x");
        let merged = spec.write_into_trigger_config(existing).expect("encode");
        // Sibling cron keys must survive the patch.
        assert_eq!(merged["schedule"].as_str(), Some("*/5 * * * *"));
        assert_eq!(merged["timezone"].as_str(), Some("UTC"));
        // Spec lands under the namespace key.
        assert_eq!(merged[WEBHOOK_ACTIVATION_KEY]["provider"], "generic");
    }

    #[test]
    fn missing_namespace_returns_none() {
        let config = serde_json::json!({ "schedule": "@hourly" });
        let decoded = WebhookActivationSpec::from_trigger_config(&config).expect("ok");
        assert!(decoded.is_none());
    }

    #[test]
    fn malformed_namespace_surfaces_typed_error() {
        let config = serde_json::json!({
            WEBHOOK_ACTIVATION_KEY: {
                "provider": 42,
                "secret_id": "x",
            }
        });
        let err = WebhookActivationSpec::from_trigger_config(&config).expect_err("must fail");
        let WebhookActivationSpecError::Decode { reason } = err else {
            panic!("expected Decode, got {err:?}");
        };
        // serde_json reports the type-mismatch message; we only assert
        // the failure surfaced as a `Decode` variant with a non-empty
        // reason — the wording itself is serde's, not ours.
        assert!(!reason.is_empty(), "reason must carry the serde error");
    }

    #[test]
    fn rejects_non_object_config() {
        let spec = WebhookActivationSpec::new("slack", "x");
        let err = spec
            .write_into_trigger_config(serde_json::json!([]))
            .expect_err("must fail");
        let WebhookActivationSpecError::Encode { reason } = err else {
            panic!("expected Encode, got {err:?}");
        };
        assert!(reason.contains("array"), "reason: {reason}");
    }

    #[test]
    fn timestamp_format_serde_snake_case() {
        let value = serde_json::to_value(WebhookTimestampFormat::UnixSeconds).unwrap();
        assert_eq!(value, serde_json::Value::String("unix_seconds".into()));
        let parsed: WebhookTimestampFormat =
            serde_json::from_value(serde_json::json!("rfc3339")).unwrap();
        assert_eq!(parsed, WebhookTimestampFormat::Rfc3339);
    }
}
