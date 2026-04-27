//! [`WebhookSource`] — `TriggerSource` for HTTP webhook trigger family.

use crate::{trigger::TriggerSource, webhook::WebhookRequest};

/// Trigger event source for HTTP webhooks.
///
/// Implementations of `WebhookAction` must use
/// `type Source = WebhookSource;` — the `<Self::Source as TriggerSource>::Event`
/// projection then resolves to [`WebhookRequest`], which carries the
/// transport-specific body, headers, signature outcome, and method.
#[derive(Debug, Clone, Copy)]
pub struct WebhookSource;

impl TriggerSource for WebhookSource {
    type Event = WebhookRequest;
}
