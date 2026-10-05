//! Server compatibility imports and transport-to-domain mappings.

pub use nebula_api_contract::v1::webhook::*;

/// Bound the transport-level shape before delegating its complete schema to
/// the selected trusted provider factory.
///
/// The walk is iterative so adversarial JSON cannot recurse through the
/// process stack. `serde_json` and the router body limit already bound the
/// encoded input; the node cap also bounds validation work and future
/// deserializers with different recursion policies.
pub(crate) fn validate_provider_config_shape(
    request: &RegisterWebhookRequest,
) -> Result<(), &'static str> {
    const MAX_CONFIG_NODES: usize = 4_096;

    let Some(config) = request.provider_config.as_ref() else {
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

#[cfg(test)]
mod tests {
    use super::{RegisterWebhookRequest, RegisterWebhookResponse, validate_provider_config_shape};

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

        assert_eq!(validate_provider_config_shape(&request), Ok(()));
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
            validate_provider_config_shape(&request),
            Err("provider_config must be a JSON object")
        );
    }
}
