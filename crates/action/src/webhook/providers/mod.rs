//! Provider-typed [`WebhookAction`](super::WebhookAction) catalog.
//!
//! Three reference implementations supplied out of the box:
//!
//! - [`generic::GenericWebhookAction`] — provider-agnostic HMAC over the
//!   request body, optional `?challenge=<token>` GET handshake.
//! - [`slack::SlackWebhookAction`] — Slack `v0` HMAC over
//!   `v0:{ts}:{body}` plus `X-Slack-Request-Timestamp`,
//!   `url_verification` interception.
//! - [`stripe::StripeWebhookAction`] — Stripe `t=…,v1=…` parsing,
//!   `pending_webhook` ping interception.
//!
//! Each provider returns a [`WebhookConfig`](super::WebhookConfig)
//! tagged with the corresponding [`WebhookProvider`](super::WebhookProvider)
//! variant so the transport can label telemetry without inspecting
//! action types.
//!
//! Operator-supplied webhook activations carry an `action_kind`
//! ("slack" | "stripe" | "generic") in storage. The runtime registry
//! looks up a [`crate::webhook::factory::WebhookActionFactory`] for
//! that kind and constructs the right provider with the stored
//! secret, replay window, and (for Generic) optional challenge token.

pub mod generic;
pub mod slack;
pub mod stripe;

use std::sync::Arc;

pub use generic::{GenericWebhookAction, GenericWebhookActionFactory};
pub use slack::{SlackWebhookAction, SlackWebhookActionFactory};
pub use stripe::{StripeWebhookAction, StripeWebhookActionFactory};

use super::factory::WebhookActionFactory;

/// Default factory bundle: `slack`, `stripe`, `generic`. Engine
/// startup typically iterates this list and calls
/// `ActionRegistry::register_webhook_provider` for each.
#[must_use]
pub fn default_factories() -> Vec<Arc<dyn WebhookActionFactory>> {
    vec![
        Arc::new(GenericWebhookActionFactory::new()),
        Arc::new(SlackWebhookActionFactory::new()),
        Arc::new(StripeWebhookActionFactory::new()),
    ]
}
