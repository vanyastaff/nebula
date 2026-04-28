//! [`WebhookSource`] — `TriggerSource` for HTTP webhook trigger family.

use crate::{trigger::TriggerSource, webhook::WebhookRequest};

/// Trigger event source for HTTP webhooks.
///
/// Implementations of `WebhookAction` must use
/// `type Source = WebhookSource;` — the `<Self::Source as TriggerSource>::Event`
/// projection then resolves to [`WebhookRequest`], which carries the
/// transport-specific body, headers, signature outcome, and method.
///
/// # In-tree consumers (П1 status)
///
/// `WebhookTriggerAdapter` currently implements `TriggerHandler` directly
/// and routes `WebhookRequest` via `TriggerEvent::downcast`. The typed
/// `type Source = WebhookSource` projection is a public-API surface for
/// community plugins that implement `TriggerAction` directly; engine
/// cascade work later wires the adapter to use the typed surface.
#[derive(Debug, Clone, Copy)]
pub struct WebhookSource;

impl TriggerSource for WebhookSource {
    type Event = WebhookRequest;
}
