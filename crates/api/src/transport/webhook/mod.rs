//! HTTP transport layer for `nebula-action` webhook triggers.
//!
//! See the maintainers' private design vault section 4.4
//! for the full design.
//!
//! ## Module layout
//!
//! - [`transport`] — public [`WebhookTransport`] struct with lifecycle API
//!   ([`activate`](WebhookTransport::activate) /
//!   [`deactivate`](WebhookTransport::deactivate) /
//!   [`router`](WebhookTransport::router)) and `TransportInner` internals.
//! - `dispatch` — shared `dispatch_inner` pipeline (routing lookup →
//!   rate-limit → signature → oneshot dispatch) plus the two axum handler fns.
//! - `signature` —  signature-policy enforcement (`enforce_signature`,
//!   problem+json response builders, metric recording).
//! - `replay` — replay-window rejection reason mapping
//!   (`replay_reason_for`): maps signature failure codes to the dedicated
//!   replay-counter label set.
//! - [`provider`] — [`EndpointProviderImpl`] implementing `nebula_action::WebhookEndpointProvider`
//!   so action code can read `ctx.webhook.endpoint_url()`.
//! - `routing` — private `RoutingMap` (DashMap under the hood) keyed by `(trigger_uuid, nonce)`.
//! - Rate limiting lives in [`ratelimit`] (moved out of
//!   `middleware/webhook_ratelimit` in webhook activation phase F1; the
//!   middleware/ placement was a misnomer — there was never a Tower
//!   `Layer` wrapping the limiter, it is consumed directly by the
//!   transport).

pub mod bootstrap;
pub(super) mod dispatch;
pub mod events;
pub mod key;
pub mod provider;
pub mod ratelimit;
pub(super) mod replay;
pub(crate) mod routing;
pub(super) mod signature;
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
