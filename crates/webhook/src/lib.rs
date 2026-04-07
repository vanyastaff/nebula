//! # Nebula Webhook Infrastructure
//!
//! Provides a unified HTTP webhook server for the Nebula workflow engine.
//! This crate enables external services (Telegram, GitHub, Stripe, etc.) to send
//! events to workflow triggers via unique, isolated webhook endpoints.
//!
//! ## Architecture
//!
//! - **Single Server**: One HTTP server per runtime, listening on a single port
//! - **UUID Isolation**: Each trigger gets a unique UUID path for security and routing
//! - **Environment Separation**: Test and Production traffic never cross paths
//! - **RAII Lifecycle**: Automatic cleanup when triggers are dropped
//! - **Framework Abstraction**: Developers only implement business logic
//!
//! ## Example
//!
//! ```no_run
//! use nebula_webhook::prelude::*;
//! use nebula_resource::Context;
//! use async_trait::async_trait;
//!
//! struct MyTrigger;
//!
//! #[async_trait]
//! impl WebhookAction for MyTrigger {
//!     type Event = String;
//!
//!     async fn on_subscribe(&self, ctx: &TriggerCtx) -> Result<()> {
//!         // Register webhook with external provider
//!         println!("Register webhook: {}", ctx.webhook_url());
//!         Ok(())
//!     }
//!
//!     async fn on_webhook(
//!         &self,
//!         ctx: &TriggerCtx,
//!         payload: WebhookPayload,
//!     ) -> Result<Option<Self::Event>> {
//!         // Verify and parse incoming request
//!         let event = String::from_utf8(payload.body.to_vec()).unwrap();
//!         Ok(Some(event))
//!     }
//!
//!     async fn on_unsubscribe(&self, ctx: &TriggerCtx) -> Result<()> {
//!         // Clean up webhook registration
//!         Ok(())
//!     }
//!
//!     async fn test(&self, ctx: &TriggerCtx) -> Result<TestResult> {
//!         Ok(TestResult::success("Connection test passed"))
//!     }
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

mod context;
mod environment;
mod error;
mod handle;
pub mod metrics;
mod payload;
pub mod rate_limit;
mod route_map;
mod server;
mod state;
mod store;
mod traits;
pub mod verifier;

pub use context::TriggerCtx;
pub use environment::Environment;
pub use error::{Error, Result};
pub use handle::TriggerHandle;
pub use payload::WebhookPayload;
pub use rate_limit::WebhookRateLimiter;
pub use server::{WebhookServer, WebhookServerConfig};
pub use state::TriggerState;
pub use store::{MemoryStateStore, StateStore};
pub use traits::{TestResult, WebhookAction};
pub use verifier::{HmacSha256Verifier, WebhookVerifier};

/// Convenience re-exports
pub mod prelude {
    pub use crate::{
        Environment, Error, HmacSha256Verifier, MemoryStateStore, Result, StateStore, TestResult,
        TriggerCtx, TriggerHandle, TriggerState, WebhookAction, WebhookPayload, WebhookRateLimiter,
        WebhookServer, WebhookServerConfig, WebhookVerifier,
    };
}
