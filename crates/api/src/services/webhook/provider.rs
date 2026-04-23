//! `EndpointProviderImpl` — implements the [`WebhookEndpointProvider`]
//! trait so webhook actions can read the public URL they should
//! register with the external provider in `on_activate`.

use nebula_action::WebhookEndpointProvider;
use url::Url;
use uuid::Uuid;

/// Concrete provider stored on [`TriggerContext.webhook`][nebula_action::TriggerContext]
/// at trigger activation time.
///
/// Holds the fully-resolved URL and path string so accessor calls
/// are infallible reads. The transport constructs one instance per
/// `(trigger_uuid, nonce)` activation and injects it via
/// [`TriggerContext::with_webhook_endpoint`][nebula_action::TriggerContext::with_webhook_endpoint].
#[derive(Debug, Clone)]
pub struct EndpointProviderImpl {
    url: Url,
    path: String,
}

impl EndpointProviderImpl {
    /// Build a provider from `base_url`, a path prefix, a trigger
    /// UUID, and a per-activation nonce.
    ///
    /// Final URL shape: `<base_url><path_prefix>/<uuid>/<nonce>`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the combined URL cannot be parsed — in
    /// practice this only happens if `base_url` is not a valid
    /// origin-only URL (no path, no query, no fragment).
    pub fn new(
        base_url: &Url,
        path_prefix: &str,
        trigger_uuid: Uuid,
        nonce: &str,
    ) -> Result<Self, url::ParseError> {
        let trimmed = path_prefix.trim_matches('/');
        let path = if trimmed.is_empty() {
            format!("/{trigger_uuid}/{nonce}")
        } else {
            format!("/{trimmed}/{trigger_uuid}/{nonce}")
        };
        let mut url = base_url.clone();
        url.set_path(&path);
        Ok(Self { url, path })
    }
}

impl WebhookEndpointProvider for EndpointProviderImpl {
    fn endpoint_url(&self) -> &Url {
        &self.url
    }

    fn endpoint_path(&self) -> &str {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_url_with_path_prefix() {
        let base = Url::parse("https://nebula.example.com").unwrap();
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let provider = EndpointProviderImpl::new(&base, "/webhooks", uuid, "abc123").unwrap();
        assert_eq!(
            provider.endpoint_url().as_str(),
            "https://nebula.example.com/webhooks/550e8400-e29b-41d4-a716-446655440000/abc123"
        );
        assert_eq!(
            provider.endpoint_path(),
            "/webhooks/550e8400-e29b-41d4-a716-446655440000/abc123"
        );
    }

    #[test]
    fn normalizes_missing_or_extra_slashes() {
        let base = Url::parse("https://x.example.com").unwrap();
        let uuid = Uuid::nil();
        let a = EndpointProviderImpl::new(&base, "webhooks", uuid, "n").unwrap();
        let b = EndpointProviderImpl::new(&base, "/webhooks/", uuid, "n").unwrap();
        let c = EndpointProviderImpl::new(&base, "//webhooks//", uuid, "n").unwrap();
        assert_eq!(a.endpoint_path(), b.endpoint_path());
        assert_eq!(a.endpoint_path(), c.endpoint_path());
    }

    #[test]
    fn empty_path_prefix_allowed() {
        let base = Url::parse("https://x.example.com").unwrap();
        let uuid = Uuid::nil();
        let provider = EndpointProviderImpl::new(&base, "", uuid, "nonce").unwrap();
        assert_eq!(
            provider.endpoint_path(),
            "/00000000-0000-0000-0000-000000000000/nonce"
        );
    }
}
