//! Server compatibility imports and transport-to-domain mappings.

pub use nebula_api_contract::v1::resource::*;

#[cfg(test)]
mod tests {
    use super::{CreateResourceRequest, UpdateResourceRequest};

    #[test]
    fn create_request_defaults_credential_bindings_and_redacts_debug() {
        let request: CreateResourceRequest = serde_json::from_value(serde_json::json!({
            "slug": "primary-http",
            "display_name": "Primary HTTP",
            "kind": "http_pool",
            "config": { "api_token": "config-secret" }
        }))
        .expect("create request without bindings must deserialize");

        assert!(request.credential_bindings.is_empty());
        let debug = format!("{request:?}");
        assert!(!debug.contains("config-secret"));
        assert!(!debug.contains("config"));
        assert!(!debug.contains("credential_bindings"));
    }

    #[test]
    fn update_request_debug_does_not_expose_binding_selectors() {
        let request: UpdateResourceRequest = serde_json::from_value(serde_json::json!({
            "display_name": "Primary HTTP",
            "kind": "http_pool",
            "config": { "api_token": "config-secret" },
            "credential_bindings": { "api_token": "credential-selector" },
            "expected_version": 4
        }))
        .expect("update request with bindings must deserialize");

        assert_eq!(
            request
                .credential_bindings
                .get("api_token")
                .map(String::as_str),
            Some("credential-selector")
        );
        let debug = format!("{request:?}");
        assert!(!debug.contains("config-secret"));
        assert!(!debug.contains("credential-selector"));
        assert!(!debug.contains("config"));
        assert!(!debug.contains("credential_bindings"));
    }
}
