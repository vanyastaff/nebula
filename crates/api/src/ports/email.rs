//! API-owned email port — decouples verification / password-reset email
//! delivery from the [`AuthBackend`] implementation.
//!
//! `nebula-api` ships a dev-only [`EchoSink`] impl that buffers messages
//! in-process so tests and the local-first `simple_server` binary can
//! exercise sign-up / password-reset flows without an SMTP transport. A
//! production composition root wires a real transport (SMTP, SES, …) as
//! `Arc<dyn EmailPort>` into [`crate::AppState::email_port`] and into the
//! storage-backed `AuthBackend` so a `PgAuthBackend` never silently drops
//! verification / reset emails.
//!
//! The port carries only api-safe types ([`EmailMessage`]) and a typed
//! [`EmailError`] — concrete impls live in the composition root, mirroring
//! the rest of [`crate::ports`].
//!
//! [`AuthBackend`]: crate::domain::auth::backend::AuthBackend
//! [`AppState`]: crate::AppState

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use thiserror::Error;

/// Category of an outbound email — drives templating in real transports and
/// gives tests a stable label to assert on without parsing bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmailKind {
    /// Email-address verification link (sign-up flow).
    Verification,
    /// Password reset link (forgot-password flow).
    PasswordReset,
    /// Catch-all for transactional / system notifications that do not
    /// belong to a specific identity flow.
    Generic,
}

impl EmailKind {
    /// Stable string label. Used in tracing fields and by the legacy
    /// `EmailEnvelope.kind` back-compat shim on the in-memory backend.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verification => "EmailVerify",
            Self::PasswordReset => "PasswordReset",
            Self::Generic => "Generic",
        }
    }
}

impl std::fmt::Display for EmailKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One outbound email — what a transport sees.
///
/// The dev [`EchoSink`] currently puts the verification / reset *token*
/// directly into [`Self::body`] so tests can pull the token out without
/// parsing rendered HTML; production transports replace this with a
/// rendered template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    /// Recipient address. The transport is responsible for any final
    /// address validation; the [`EchoSink`] applies a minimal `@` check.
    pub to: String,
    /// Short subject line.
    pub subject: String,
    /// Message body. For the dev [`EchoSink`] this is the raw token;
    /// production transports render a template.
    pub body: String,
    /// Category — drives templating and gives tests a label to assert on.
    pub kind: EmailKind,
}

/// Failure modes for an [`EmailPort::send`] call.
///
/// Deliberately small: every transport-side fault collapses into
/// [`Self::Transport`], and any address rejected before transit becomes
/// [`Self::InvalidAddress`]. Callers should treat both as terminal for
/// the current attempt (no implicit retry).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EmailError {
    /// The transport failed to accept / deliver the message (network
    /// error, SMTP 5xx, queue full, …). Detail string is operator-facing.
    #[error("email transport failure: {0}")]
    Transport(String),

    /// The recipient address was rejected before transit (bad format,
    /// disallowed domain). Detail string carries the offending value so
    /// operator logs can pinpoint the call site.
    #[error("invalid email address: {0}")]
    InvalidAddress(String),
}

/// Outbound email seam — the API never imports a concrete transport.
///
/// Implemented in dev / tests by [`EchoSink`]; production composition
/// roots wire an SMTP / SES / SendGrid impl as `Arc<dyn EmailPort>`. The
/// trait uses `#[async_trait]` so a single `Arc<dyn EmailPort>` works
/// across every handler and storage-backed `AuthBackend` impl (matches
/// the `AuthBackend` pattern).
#[async_trait]
pub trait EmailPort: Send + Sync {
    /// Deliver `msg` to the recipient. Returns [`EmailError::Transport`]
    /// for transit-layer failures and [`EmailError::InvalidAddress`] for
    /// rejected recipients.
    async fn send(&self, msg: EmailMessage) -> Result<(), EmailError>;
}

/// Dev / test [`EmailPort`] that buffers every delivered message in an
/// in-process inbox.
///
/// Mirrors the previous `InMemoryAuthBackend::email_sink` behaviour so
/// the in-memory backend's `emails()` introspection method keeps working
/// against the default port. Production composition roots replace this
/// with a real transport via [`crate::AppState::with_email_port`].
#[derive(Default, Clone)]
pub struct EchoSink {
    inbox: Arc<RwLock<Vec<EmailMessage>>>,
}

impl EchoSink {
    /// Construct an empty echo sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the inbox without clearing it.
    #[must_use]
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn peek(&self) -> Vec<EmailMessage> {
        self.inbox.read().clone()
    }

    /// Drain and return every buffered message.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn drain(&self) -> Vec<EmailMessage> {
        std::mem::take(&mut *self.inbox.write())
    }
}

#[async_trait]
impl EmailPort for EchoSink {
    #[tracing::instrument(
        level = "debug",
        skip(self, msg),
        fields(email.kind = %msg.kind),
    )]
    async fn send(&self, msg: EmailMessage) -> Result<(), EmailError> {
        let trimmed = msg.to.trim();
        if trimmed.is_empty() || !trimmed.contains('@') {
            return Err(EmailError::InvalidAddress(msg.to));
        }
        self.inbox.write().push(msg);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: EmailKind) -> EmailMessage {
        EmailMessage {
            to: "user@nebula.dev".to_owned(),
            subject: "hello".to_owned(),
            body: "token-abc".to_owned(),
            kind,
        }
    }

    #[tokio::test]
    async fn echo_sink_buffers_sent_message() {
        let sink = EchoSink::new();
        sink.send(sample(EmailKind::Verification))
            .await
            .expect("send must succeed");

        let inbox = sink.peek();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].to, "user@nebula.dev");
        assert_eq!(inbox[0].kind, EmailKind::Verification);
        assert_eq!(inbox[0].body, "token-abc");
    }

    #[tokio::test]
    async fn echo_sink_drain_clears_inbox() {
        let sink = EchoSink::new();
        sink.send(sample(EmailKind::PasswordReset)).await.unwrap();
        sink.send(sample(EmailKind::Generic)).await.unwrap();

        let drained = sink.drain();
        assert_eq!(drained.len(), 2);
        assert!(sink.peek().is_empty(), "drain must leave an empty inbox");
    }

    #[tokio::test]
    async fn echo_sink_rejects_invalid_address() {
        let sink = EchoSink::new();
        let mut msg = sample(EmailKind::Verification);
        msg.to = "not-an-email".to_owned();
        let err = sink
            .send(msg)
            .await
            .expect_err("missing `@` must reject before buffer");
        assert!(matches!(err, EmailError::InvalidAddress(_)));
        assert!(sink.peek().is_empty(), "rejected message must not buffer");
    }

    #[test]
    fn email_kind_as_str_matches_legacy_labels() {
        // The legacy `EmailEnvelope.kind` strings — keep stable across
        // the trait refactor so the `InMemoryAuthBackend::emails()`
        // back-compat shim and downstream test assertions still match.
        assert_eq!(EmailKind::Verification.as_str(), "EmailVerify");
        assert_eq!(EmailKind::PasswordReset.as_str(), "PasswordReset");
        assert_eq!(EmailKind::Generic.as_str(), "Generic");
    }
}
