//! Database connection credential -- static, non-interactive.
//!
//! Resolves host, port, database name, username, and password into
//! [`DatabaseAuth`]. State and Scheme are the same type via
//! [`identity_state!`](crate::identity_state).

use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Parameter, ParameterCollection};
use serde_json::json;

use crate::SecretString;
use crate::core::{CredentialContext, CredentialDescription, CredentialError};
use crate::credential_trait::Credential;
use crate::pending::NoPendingState;
use crate::resolve::StaticResolveResult;
use crate::scheme::{DatabaseAuth, SslMode};

/// Default port for database connections (PostgreSQL).
const DEFAULT_PORT: u16 = 5432;

/// Default SSL mode string when not specified by the user.
const DEFAULT_SSL_MODE: &str = "prefer";

/// Parses a string SSL mode into the [`SslMode`] enum.
///
/// Recognized values (case-insensitive): `disable`, `prefer`, `require`,
/// `verify-ca` / `verify_ca`, `verify-full` / `verify_full`.
/// Falls back to [`SslMode::Prefer`] for unrecognized values.
fn parse_ssl_mode(value: &str) -> SslMode {
    match value.to_lowercase().as_str() {
        "disable" | "disabled" => SslMode::Disabled,
        "prefer" => SslMode::Prefer,
        "require" => SslMode::Require,
        "verify-ca" | "verify_ca" => SslMode::VerifyCa,
        "verify-full" | "verify_full" => SslMode::VerifyFull,
        _ => SslMode::Prefer,
    }
}

/// Database connection credential -- resolves connection parameters into
/// [`DatabaseAuth`].
///
/// - **Non-interactive:** resolves in one step from user input.
/// - **Non-refreshable:** static credentials have no expiry.
/// - **Identity projection:** stored state is the scheme itself.
///
/// # Defaults
///
/// - **Port:** 5432 (PostgreSQL standard)
/// - **SSL mode:** `prefer` (attempt SSL, fall back to plaintext)
pub struct DatabaseCredential;

impl Credential for DatabaseCredential {
    type Scheme = DatabaseAuth;
    type State = DatabaseAuth;
    type Pending = NoPendingState;

    const KEY: &'static str = "database";

    fn description() -> CredentialDescription {
        CredentialDescription {
            key: Self::KEY.to_owned(),
            name: "Database".to_owned(),
            description: "Database connection credentials (host, port, user, password, ssl_mode)."
                .to_owned(),
            icon: Some("database".to_owned()),
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(
                Parameter::string("host")
                    .label("Host")
                    .description("Database server hostname or IP address")
                    .placeholder("localhost")
                    .required(),
            )
            .add(
                Parameter::integer("port")
                    .label("Port")
                    .description("Database server port number")
                    .default(json!(5432)),
            )
            .add(
                Parameter::string("database")
                    .label("Database")
                    .description("Name of the database to connect to")
                    .required(),
            )
            .add(
                Parameter::string("username")
                    .label("Username")
                    .description("Username for database authentication")
                    .required(),
            )
            .add(
                Parameter::string("password")
                    .label("Password")
                    .description("Password for database authentication")
                    .required()
                    .secret(),
            )
            .add(
                Parameter::string("ssl_mode")
                    .label("SSL Mode")
                    .description(
                        "SSL/TLS connection mode: disable, prefer, require, verify-ca, verify-full",
                    )
                    .placeholder("prefer")
                    .default(json!("prefer")),
            )
    }

    fn project(state: &DatabaseAuth) -> DatabaseAuth {
        state.clone()
    }

    async fn resolve(
        values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<DatabaseAuth>, CredentialError> {
        let host = values
            .get_string("host")
            .ok_or_else(|| CredentialError::Provider("missing required field 'host'".to_owned()))?;

        let port: u16 = values.get_number("port").unwrap_or(DEFAULT_PORT);

        let database = values.get_string("database").ok_or_else(|| {
            CredentialError::Provider("missing required field 'database'".to_owned())
        })?;

        let username = values.get_string("username").ok_or_else(|| {
            CredentialError::Provider("missing required field 'username'".to_owned())
        })?;

        let password = values.get_string("password").ok_or_else(|| {
            CredentialError::Provider("missing required field 'password'".to_owned())
        })?;

        let ssl_mode = parse_ssl_mode(values.get_string("ssl_mode").unwrap_or(DEFAULT_SSL_MODE));

        let secret = SecretString::new(password.to_owned());
        Ok(StaticResolveResult::Complete(
            DatabaseAuth::new(host, port, database, username, secret).with_ssl_mode(ssl_mode),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_database() {
        assert_eq!(DatabaseCredential::KEY, "database");
    }

    #[test]
    fn capabilities_are_all_false() {
        assert!(!DatabaseCredential::INTERACTIVE);
        assert!(!DatabaseCredential::REFRESHABLE);
        assert!(!DatabaseCredential::REVOCABLE);
        assert!(!DatabaseCredential::TESTABLE);
    }

    #[test]
    fn project_returns_clone_of_state() {
        let auth = DatabaseAuth::new(
            "localhost",
            5432,
            "mydb",
            "admin",
            SecretString::new("dbpass"),
        );
        let projected = DatabaseCredential::project(&auth);
        assert_eq!(projected.host, "localhost");
        assert_eq!(projected.port, 5432);
        assert_eq!(projected.database, "mydb");
        assert_eq!(projected.username, "admin");
        let original = auth.password().expose_secret(|s| s.to_owned());
        let cloned = projected.password().expose_secret(|s| s.to_owned());
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_all_fields() {
        let mut values = ParameterValues::new();
        values.set(
            "host".to_owned(),
            serde_json::Value::String("db.example.com".into()),
        );
        values.set("port".to_owned(), serde_json::Value::Number(5432.into()));
        values.set(
            "database".to_owned(),
            serde_json::Value::String("production".into()),
        );
        values.set(
            "username".to_owned(),
            serde_json::Value::String("app_user".into()),
        );
        values.set(
            "password".to_owned(),
            serde_json::Value::String("super-secret".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = DatabaseCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(auth) => {
                assert_eq!(auth.host, "db.example.com");
                assert_eq!(auth.port, 5432);
                assert_eq!(auth.database, "production");
                assert_eq!(auth.username, "app_user");
                assert_eq!(auth.ssl_mode, SslMode::Prefer);
                let pw = auth.password().expose_secret(|s| s.to_owned());
                assert_eq!(pw, "super-secret");
            }
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_defaults_port_to_5432() {
        let mut values = ParameterValues::new();
        values.set("host", serde_json::Value::String("localhost".into()));
        values.set("database", serde_json::Value::String("mydb".into()));
        values.set("username", serde_json::Value::String("user".into()));
        values.set("password", serde_json::Value::String("pass".into()));
        let ctx = CredentialContext::new("test-user");
        let result = DatabaseCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(auth) => {
                assert_eq!(auth.port, 5432);
            }
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_parses_ssl_mode() {
        let mut values = ParameterValues::new();
        values.set("host", serde_json::Value::String("localhost".into()));
        values.set("port", serde_json::Value::Number(5432.into()));
        values.set("database", serde_json::Value::String("mydb".into()));
        values.set("username", serde_json::Value::String("user".into()));
        values.set("password", serde_json::Value::String("pass".into()));
        values.set("ssl_mode", serde_json::Value::String("require".into()));
        let ctx = CredentialContext::new("test-user");
        let result = DatabaseCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(auth) => {
                assert_eq!(auth.ssl_mode, SslMode::Require);
            }
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_host() {
        let mut values = ParameterValues::new();
        values.set("port".to_owned(), serde_json::Value::Number(5432.into()));
        values.set(
            "database".to_owned(),
            serde_json::Value::String("db".into()),
        );
        values.set(
            "username".to_owned(),
            serde_json::Value::String("user".into()),
        );
        values.set(
            "password".to_owned(),
            serde_json::Value::String("pass".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = DatabaseCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_falls_back_to_default_port_on_invalid_value() {
        let mut values = ParameterValues::new();
        values.set("host", serde_json::Value::String("localhost".into()));
        values.set("port", serde_json::Value::String("not-a-number".into()));
        values.set("database", serde_json::Value::String("db".into()));
        values.set("username", serde_json::Value::String("user".into()));
        values.set("password", serde_json::Value::String("pass".into()));
        let ctx = CredentialContext::new("test-user");
        let result = DatabaseCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(auth) => {
                assert_eq!(auth.port, DEFAULT_PORT);
            }
            _ => panic!("expected Complete variant"),
        }
    }

    #[test]
    fn parse_ssl_mode_handles_all_variants() {
        assert_eq!(parse_ssl_mode("disable"), SslMode::Disabled);
        assert_eq!(parse_ssl_mode("disabled"), SslMode::Disabled);
        assert_eq!(parse_ssl_mode("prefer"), SslMode::Prefer);
        assert_eq!(parse_ssl_mode("require"), SslMode::Require);
        assert_eq!(parse_ssl_mode("verify-ca"), SslMode::VerifyCa);
        assert_eq!(parse_ssl_mode("verify_ca"), SslMode::VerifyCa);
        assert_eq!(parse_ssl_mode("verify-full"), SslMode::VerifyFull);
        assert_eq!(parse_ssl_mode("verify_full"), SslMode::VerifyFull);
        assert_eq!(parse_ssl_mode("REQUIRE"), SslMode::Require);
        assert_eq!(parse_ssl_mode("unknown"), SslMode::Prefer);
    }
}
