//! Production [`EmailPort`] backed by the `lettre` SMTP transport.
//!
//! The trait is defined in `nebula-api` (api-safe shape, no `lettre` dep).
//! This module owns the lettre-side glue: parsing a [`SmtpEmailConfig`]
//! into an `AsyncSmtpTransport<Tokio1Executor>` with the requested TLS
//! posture, translating an [`EmailMessage`] into a `lettre::Message`,
//! and mapping `lettre`'s rich transport error tree onto the small
//! [`EmailError::Transport`] / [`EmailError::InvalidAddress`] surface
//! the rest of the API understands.
//!
//! ## Security
//!
//! The SMTP password lives in a `secrecy::SecretString` on the config
//! side: `SmtpEmailConfig::password` is `Option<SecretString>`
//! (auto-redacting `Debug`, zeroized on drop). At
//! [`SmtpEmailPort::new`] construction time the secret is exposed once
//! via `ExposeSecret` and consumed by
//! `lettre::transport::smtp::authentication::Credentials::new`, which
//! internally owns a `String` for the **lifetime of the transport** (it
//! must survive every `send` call). That `String` is private to
//! lettre, never re-read by this module, and never logged. The
//! `SecretString` on the config drops + zeroizes when the `ApiConfig`
//! value drops at process exit; from that moment forward the only
//! remaining copy is inside the lettre transport, which is also
//! dropped at process exit.
//!
//! The error mapping path NEVER includes the credentials. lettre's own
//! `Error::Display` does not embed the password (verified against
//! `lettre-0.11.22/src/transport/smtp/error.rs`), but we still wrap the
//! mapped string in a tracing-safe `EmailError::Transport(reason)` and
//! never log the `SmtpEmailConfig` value, so a future lettre revision
//! that started printing more context could not silently regress us.
//!
//! ## Composition
//!
//! [`SmtpEmailPort::new`] is what `apps/server::compose` calls when
//! `ApiConfig::smtp` is `Some`. It fails CLOSED on TLS-parameter
//! construction errors so an operator who set `API_SMTP_HOST` without a
//! reachable certificate authority does not silently fall back to
//! plaintext or to `EchoSink`.
//!
//! [`EmailPort`]: nebula_api::ports::email::EmailPort
//! [`EmailMessage`]: nebula_api::ports::email::EmailMessage
//! [`EmailError::Transport`]: nebula_api::ports::email::EmailError::Transport
//! [`EmailError::InvalidAddress`]: nebula_api::ports::email::EmailError::InvalidAddress

use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use nebula_api::{
    config::{SmtpEmailConfig, SmtpTlsMode},
    ports::email::{EmailError, EmailMessage, EmailPort},
};
use secrecy::ExposeSecret;
use thiserror::Error;

/// Failure modes for [`SmtpEmailPort::new`] — separate from runtime
/// `EmailError` so the composition root can fail CLOSED at startup with
/// a typed error instead of a string-formatted runtime path.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SmtpEmailPortBuildError {
    /// `from_address` failed to parse as an RFC 5321 mailbox. The
    /// `from_env` validator already enforces the `@` test but does
    /// not parse the full local-part / domain grammar; lettre's
    /// `Mailbox::FromStr` is the canonical gate.
    #[error("SMTP from_address is not a valid mailbox: {0}")]
    InvalidFromAddress(String),

    /// `lettre::AsyncSmtpTransport::relay` / `starttls_relay` rejected
    /// the host string (bad DNS name, malformed TLS parameters, etc.).
    /// The wrapped detail comes from lettre and intentionally does NOT
    /// include the SMTP password (we never pass it into this path).
    #[error("SMTP transport construction failed: {0}")]
    Transport(String),
}

/// `EmailPort` impl backed by a single-connection
/// `AsyncSmtpTransport<Tokio1Executor>`.
///
/// The `pool` feature of `lettre` is intentionally NOT enabled, so this
/// is NOT a pooled transport — lettre opens / closes the connection
/// per `send` and reconnects on transient errors. Pooling can be added
/// later by enabling the `pool` feature + plumbing a `pool_config`
/// knob through `SmtpEmailConfig`.
///
/// Holds the validated `From` mailbox alongside the transport so every
/// outbound message uses the operator-configured sender regardless of
/// what an [`EmailMessage`] carries — a misconfigured handler cannot
/// smuggle a different sender through this port. The auto-derived
/// `Debug` is safe: the inner `AsyncSmtpTransport` only exposes server
/// metadata (no credentials), and lettre's stub transport carries only
/// the test-recorded message log.
#[derive(Debug)]
pub struct SmtpEmailPort {
    transport: TransportImpl,
    from_address: Mailbox,
}

/// Inner transport dispatch.
///
/// Production builds use `Smtp` exclusively; the `#[cfg(test)]` `Stub`
/// arm lets the unit tests assert SMTP envelope shape and error mapping
/// against `lettre::transport::stub::AsyncStubTransport` without a real
/// SMTP server. The enum dispatch keeps `EmailPort` `dyn`-compatible
/// (no generic on the public type) while staying allocation-free in
/// the hot path.
#[derive(Debug)]
enum TransportImpl {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    #[cfg(test)]
    Stub(lettre::transport::stub::AsyncStubTransport),
}

impl SmtpEmailPort {
    /// Build an `SmtpEmailPort` from a validated [`SmtpEmailConfig`].
    ///
    /// Returns [`SmtpEmailPortBuildError`] when the `from_address`
    /// cannot parse as a mailbox or when lettre rejects the host /
    /// TLS parameters. The composition root treats both as
    /// startup-fatal (per the fail-closed contract documented on
    /// [`nebula_api::ApiConfig::smtp`]).
    pub fn new(config: &SmtpEmailConfig) -> Result<Self, SmtpEmailPortBuildError> {
        let from_address = config
            .from_address
            .parse::<Mailbox>()
            .map_err(|err| SmtpEmailPortBuildError::InvalidFromAddress(err.to_string()))?;

        // TLS posture: lettre exposes three constructors, one per mode.
        // `relay` (implicit TLS, default port 465) and `starttls_relay`
        // (STARTTLS upgrade, default port 587) build TlsParameters from
        // the host string and can fail at parameter-construction time;
        // `builder_dangerous` is plaintext.
        let mut builder = match config.tls {
            SmtpTlsMode::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|err| SmtpEmailPortBuildError::Transport(err.to_string()))?,
            SmtpTlsMode::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .map_err(|err| SmtpEmailPortBuildError::Transport(err.to_string()))?
            },
            SmtpTlsMode::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(config.host.clone())
            },
        };

        builder = builder.port(config.port);

        // Both halves are validated to be Some-Some or None-None by
        // `ApiConfig::smtp_from_env` (`SmtpAuthIncomplete`), so the
        // `Some(_), Some(_)` arm is the only auth path. The `_, _`
        // catch-all preserves the unauthenticated-relay contract
        // without re-validating here.
        if let (Some(user), Some(password)) = (config.username.as_ref(), config.password.as_ref()) {
            // `ExposeSecret` is the only place the password leaves
            // `SecretString`. The String we hand to lettre is owned by
            // `Credentials` for the lifetime of the transport; the
            // original `SecretString` zeroizes on drop when the config
            // value drops.
            let creds = Credentials::new(user.clone(), password.expose_secret().to_owned());
            builder = builder.credentials(creds);
        }

        let transport = builder.build::<Tokio1Executor>();

        Ok(Self {
            transport: TransportImpl::Smtp(transport),
            from_address,
        })
    }

    /// Test-only constructor that wires a `lettre::AsyncStubTransport`.
    ///
    /// Kept `pub(crate)` so production code cannot accidentally build an
    /// `SmtpEmailPort` against the stub. The composition root only ever
    /// calls [`Self::new`].
    #[cfg(test)]
    pub(crate) fn with_stub_transport(
        from_address: &str,
        transport: lettre::transport::stub::AsyncStubTransport,
    ) -> Result<Self, SmtpEmailPortBuildError> {
        let from_address = from_address
            .parse::<Mailbox>()
            .map_err(|err| SmtpEmailPortBuildError::InvalidFromAddress(err.to_string()))?;
        Ok(Self {
            transport: TransportImpl::Stub(transport),
            from_address,
        })
    }

    /// Build a `lettre::Message` from an api-side [`EmailMessage`].
    ///
    /// Extracted so the production path and the unit tests share one
    /// envelope-construction implementation (the strict-TDD refactor
    /// step).
    fn build_lettre_message(&self, msg: &EmailMessage) -> Result<Message, EmailError> {
        let to: Mailbox = msg
            .to
            .parse()
            .map_err(|_| EmailError::InvalidAddress(msg.to.clone()))?;
        Message::builder()
            .from(self.from_address.clone())
            .to(to)
            .subject(msg.subject.clone())
            .body(msg.body.clone())
            // `lettre::error::Error` here is a builder-side failure
            // (invalid header, body encoding). It does NOT touch the
            // SMTP password — keep the mapping symmetrical with the
            // transport-side mapping below.
            .map_err(|err| EmailError::Transport(format!("envelope build failed: {err}")))
    }
}

#[async_trait]
impl EmailPort for SmtpEmailPort {
    #[tracing::instrument(
        level = "debug",
        skip(self, msg),
        fields(email.kind = %msg.kind, smtp.from = %self.from_address)
    )]
    async fn send(&self, msg: EmailMessage) -> Result<(), EmailError> {
        let letter = self.build_lettre_message(&msg)?;
        match &self.transport {
            TransportImpl::Smtp(transport) => {
                transport.send(letter).await.map_err(|err| {
                    // lettre's smtp::Error Display formats the SMTP
                    // status / response without the password (verified
                    // against 0.11.22 source). We still wrap rather
                    // than propagate so a future revision adding more
                    // context cannot regress us.
                    EmailError::Transport(format!("smtp send failed: {err}"))
                })?;
                Ok(())
            },
            #[cfg(test)]
            TransportImpl::Stub(transport) => {
                transport
                    .send(letter)
                    .await
                    .map_err(|err| EmailError::Transport(format!("stub send failed: {err}")))?;
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use lettre::transport::stub::AsyncStubTransport;
    use nebula_api::ports::email::{EmailKind, EmailMessage};
    use secrecy::SecretString;

    use super::*;

    const FROM: &str = "noreply@nebula.dev";
    const SECRET_PASSWORD: &str = "supersecret-must-never-appear";

    fn message(kind: EmailKind) -> EmailMessage {
        EmailMessage {
            to: "user@example.com".to_owned(),
            subject: "Verify your account".to_owned(),
            body: "token-abc-123".to_owned(),
            kind,
        }
    }

    fn cfg() -> SmtpEmailConfig {
        SmtpEmailConfig {
            host: "smtp.example.com".to_owned(),
            port: 587,
            username: Some("noreply@nebula.dev".to_owned()),
            password: Some(SecretString::from(SECRET_PASSWORD.to_owned())),
            from_address: FROM.to_owned(),
            tls: SmtpTlsMode::StartTls,
        }
    }

    /// RED \u2192 GREEN: every `EmailKind` produces a deliverable
    /// `lettre::Message` whose envelope carries the operator-configured
    /// `from_address` and the per-message recipient / subject / body.
    #[tokio::test]
    async fn smtp_port_renders_verification_message_with_correct_envelope() {
        let stub = AsyncStubTransport::new_ok();
        let port = SmtpEmailPort::with_stub_transport(FROM, stub.clone())
            .expect("from_address must parse");

        for kind in [
            EmailKind::Verification,
            EmailKind::PasswordReset,
            EmailKind::Generic,
        ] {
            port.send(message(kind)).await.expect("send must succeed");
        }

        let recorded = stub.messages().await;
        assert_eq!(recorded.len(), 3, "all three kinds must be delivered");

        for (envelope, raw) in &recorded {
            // Envelope-level sender check (RFC 5321 MAIL FROM).
            assert_eq!(
                envelope.from().map(ToString::to_string).as_deref(),
                Some(FROM),
                "every message must declare the operator-configured sender at the envelope level"
            );
            let recipients: Vec<String> = envelope.to().iter().map(ToString::to_string).collect();
            assert_eq!(recipients, vec!["user@example.com".to_owned()]);

            // Body fields exposed via the raw RFC 5322 frame.
            assert!(raw.contains("Subject: Verify your account"), "raw: {raw}");
            assert!(raw.contains("token-abc-123"), "raw: {raw}");
            assert!(
                raw.contains(&format!("From: {FROM}")),
                "from header missing in raw frame: {raw}"
            );
        }
    }

    /// TRIANGULATE: the `from_address` from the config wins even if
    /// `EmailMessage` ever carried a different sender hint in a future
    /// shape revision — the port is the single source of truth for
    /// the canonical sender.
    #[tokio::test]
    async fn smtp_port_uses_configured_from_address() {
        let stub = AsyncStubTransport::new_ok();
        let port = SmtpEmailPort::with_stub_transport(FROM, stub.clone()).expect("must construct");

        port.send(message(EmailKind::Verification))
            .await
            .expect("send must succeed");

        let (envelope, raw) = stub.messages().await.into_iter().next().expect("recorded");
        assert_eq!(
            envelope.from().map(ToString::to_string).as_deref(),
            Some(FROM)
        );
        assert!(
            raw.contains(&format!("From: {FROM}")),
            "raw frame must carry the configured From header: {raw}"
        );
    }

    /// TRIANGULATE: transport failure maps to `EmailError::Transport`
    /// AND the resulting error string does NOT contain the SMTP
    /// password — the password never enters any error context.
    #[tokio::test]
    async fn smtp_port_maps_transport_error_to_email_error_transport() {
        let stub = AsyncStubTransport::new_error();
        let port = SmtpEmailPort::with_stub_transport(FROM, stub).expect("must construct");

        let err = port
            .send(message(EmailKind::Verification))
            .await
            .expect_err("stub configured to fail must surface the error");

        let formatted = err.to_string();
        assert!(
            matches!(err, EmailError::Transport(_)),
            "expected EmailError::Transport, got: {err:?}"
        );
        assert!(
            !formatted.contains(SECRET_PASSWORD),
            "transport error must NEVER leak the SMTP password — got: {formatted}"
        );
    }

    /// REDACTION GATE: `SmtpEmailConfig::Debug` must never print the
    /// SMTP password. `SecretString` handles this via its derived
    /// `Debug` (`"[REDACTED alloc::string::String]"`), and this test
    /// makes the contract executable so a future field addition that
    /// switched to a plain `String` would fail CI immediately.
    #[test]
    fn smtp_email_config_redacts_password_in_debug() {
        let formatted = format!("{:?}", cfg());
        assert!(
            !formatted.contains(SECRET_PASSWORD),
            "SmtpEmailConfig::Debug must not leak the password — got: {formatted}"
        );
    }

    /// Construction-time validation: a malformed `from_address` must
    /// fail at compose time, never at first-send time.
    #[test]
    fn smtp_port_rejects_invalid_from_address_at_construction() {
        let err = SmtpEmailPort::with_stub_transport("not-an-email", AsyncStubTransport::new_ok())
            .expect_err("invalid from_address must fail closed");
        assert!(matches!(
            err,
            SmtpEmailPortBuildError::InvalidFromAddress(_)
        ));
    }

    /// Runtime validation: a malformed RECIPIENT address must surface
    /// as `EmailError::InvalidAddress(_)`, NOT `EmailError::Transport(_)`.
    ///
    /// `build_lettre_message` (the envelope-construction helper) maps
    /// `Mailbox::FromStr` rejection on `msg.to` to `InvalidAddress`
    /// before the transport is ever consulted, so this is a
    /// rejected-before-transit contract. Review follow-up on PR
    /// #754 — the path existed but was untested; this lock-down test
    /// prevents a future refactor from silently collapsing the variant
    /// into the catch-all `Transport` arm.
    #[tokio::test]
    async fn smtp_port_rejects_malformed_recipient_with_invalid_address_error() {
        let stub = AsyncStubTransport::new_ok();
        let port = SmtpEmailPort::with_stub_transport(FROM, stub.clone()).expect("must construct");

        let mut msg = message(EmailKind::Generic);
        msg.to = "not-an-email".to_owned();

        let err = port
            .send(msg)
            .await
            .expect_err("malformed recipient must fail before transit");

        assert!(
            matches!(err, EmailError::InvalidAddress(_)),
            "expected EmailError::InvalidAddress, got: {err:?}"
        );
        // Stub must NOT have observed the message — the address gate
        // runs before the transport, proving the variant is preserved
        // and we are not leaking malformed envelopes to lettre.
        assert!(
            stub.messages().await.is_empty(),
            "transport must not see envelopes that fail address validation"
        );
    }
}
