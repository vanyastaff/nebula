//! Production `EmailPort` implementations wired by the composition root.
//!
//! The trait, dev `EchoSink`, and `EmailMessage` / `EmailError` types live
//! in `nebula-api`; the concrete transports (today: SMTP via `lettre`,
//! tomorrow: SES / SendGrid / Postmark) live here so that workers and
//! library consumers of `nebula-api` do not transitively pull a network
//! transport crate.
//!
//! The composition root branches on [`nebula_api::ApiConfig::smtp`]:
//!
//! - `Some(SmtpEmailConfig { .. })` \u2192 [`SmtpEmailPort`] (fails CLOSED on
//!   any malformed value rather than silently falling back to `EchoSink`);
//! - `None` \u2192 the dev [`nebula_api::ports::email::EchoSink`].
//!
//! See [`smtp`] for the lettre-backed transport, its TLS posture, and the
//! `EmailError::Transport` redaction discipline that keeps the SMTP
//! password out of `tracing` lines and `problem-details` bodies.

pub mod smtp;

pub use smtp::{SmtpEmailPort, SmtpEmailPortBuildError};
