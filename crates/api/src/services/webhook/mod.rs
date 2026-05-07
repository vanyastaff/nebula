//! HTTP transport layer for `nebula-action` webhook triggers.
//!
//! See `docs/plans/2026-04-13-webhook-subsystem-spec.md` section 4.4
//! for the full design.
//!
//! ## Module layout
//!
//! - [`transport`] — public [`WebhookTransport`] struct with
//!   [`activate`](WebhookTransport::activate) / [`deactivate`](WebhookTransport::deactivate) /
//!   [`router`](WebhookTransport::router) and the axum handler function.
//! - [`provider`] — [`EndpointProviderImpl`] implementing `nebula_action::WebhookEndpointProvider`
//!   so action code can read `ctx.webhook.endpoint_url()`.
//! - `routing` — private `RoutingMap` (DashMap under the hood) keyed by `(trigger_uuid, nonce)`.
//! - Rate limiting lives in [`crate::middleware::webhook_ratelimit`] and is re-exported here for
//!   public API compatibility.

pub mod bootstrap;
pub mod events;
pub mod key;
pub mod provider;
pub mod ratelimit;
pub(crate) mod routing;
pub mod transport;

pub use bootstrap::{
    BootstrapError, BootstrapReport, ResolvedActivation, SecretResolutionError,
    WebhookContextFactory, WebhookSecretResolver, bootstrap_webhook_activations,
    collect_webhook_activations,
};
pub use events::{
    LifecycleApplyError, TriggerLifecycleBus, TriggerLifecycleEvent, TriggerLifecycleEventBus,
    TriggerLifecycleSubscriber,
};
pub use key::{TriggerCoordinates, WebhookKey};
pub use provider::EndpointProviderImpl;
pub use ratelimit::{RateLimitExceeded, WebhookRateLimiter};
pub use transport::{ActivationError, ActivationHandle, WebhookTransport, WebhookTransportConfig};
