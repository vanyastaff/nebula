//! Slug-routed webhook dispatch (M3.3).
//!
//! Bridges the human-facing
//! `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}` route to the engine
//! layer. See `crates/api/src/handlers/webhook.rs` for the HTTP
//! handler and `crates/api/src/routes/webhook.rs` for route wiring.
//!
//! # Module layout
//!
//! - [`error`] — typed `WebhookAuthError`, `WebhookEnqueueError`, `WebhookDispatchError`, plus
//!   `TriggerCoordinates` (the slug tuple).
//! - [`auth`] — `WebhookAuthConfig` enum and signature/bearer validation, sharing the HMAC
//!   primitive with [`crate::services::webhook`].
//! - [`sink`] — `WebhookTriggerSink` trait + `NoopSink` / `MpscSink` implementations.
//! - [`dispatcher`] — the in-memory `WebhookDispatcher` keyed by `TriggerCoordinates`. Looks up
//!   registrations, validates auth, and hands typed events to the configured sink.
//!
//! # Why a sibling to `services::webhook`?
//!
//! The mature `services::webhook::WebhookTransport` is keyed on
//! `(uuid, nonce)` and built around the typed
//! `nebula_action::WebhookAction` surface. That layout is right for
//! programmatic activation by the `nebula-action` runtime but not for
//! human-facing slug URLs that map to operator-configured triggers in
//! the storage layer. Building the slug-routed dispatcher as a
//! sibling type keeps the typed-action transport contract untouched
//! while sharing the same HMAC primitive.

pub mod auth;
pub mod dispatcher;
pub mod error;
pub mod sink;

pub use auth::{DEFAULT_SIGNATURE_HEADER, WebhookAuthConfig};
pub use dispatcher::{RegisterError, TriggerRegistration, WebhookDispatcher};
pub use error::{TriggerCoordinates, WebhookAuthError, WebhookDispatchError, WebhookEnqueueError};
pub use sink::{DispatchedEvent, MpscSink, NoopSink, WebhookTriggerSink};
