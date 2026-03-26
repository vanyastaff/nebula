//! Database connection credential -- static, non-interactive.
//!
//! Resolves host, port, database name, username, and password into
//! [`DatabaseAuth`]. State and Scheme are the same type via
//! [`identity_state!`](crate::identity_state).

use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Parameter, ParameterCollection};

use crate::SecretString;
use crate::core::{CredentialContext, CredentialDescription, CredentialError};
use crate::credential_trait::Credential;
use crate::pending::NoPendingState;
use crate::resolve::StaticResolveResult;
use crate::scheme::DatabaseAuth;

/// Database connection credential -- resolves connection parameters into
/// [`DatabaseAuth`].
///
/// - **Non-interactive:** resolves in one step from user input.
/// - **Non-refreshable:** static credentials have no expiry.
/// - **Identity projection:** stored state is the scheme itself.
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
            description: "Database connection credentials (host, port, user, password).".to_owned(),
            icon: Some("database".to_owned()),
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(Parameter::string("host").label("Host").required())
            .add(Parameter::number("port").label("Port").required())
            .add(Parameter::string("database").label("Database").required())
            .add(Parameter::string("username").label("Username").required())
            .add(
                Parameter::string("password")
                    .label("Password")
                    .required()
                    .secret(),
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

        let port_val = values
            .get("port")
            .ok_or_else(|| CredentialError::Provider("missing required field 'port'".to_owned()))?;
        let port = port_val
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .or_else(|| {
                port_val.as_f64().and_then(|f| {
                    let truncated = f as u64;
                    u16::try_from(truncated).ok()
                })
            })
            .ok_or_else(|| {
                CredentialError::Provider(
                    "field 'port' must be a valid port number (0-65535)".to_owned(),
                )
            })?;

        let database = values.get_string("database").ok_or_else(|| {
            CredentialError::Provider("missing required field 'database'".to_owned())
        })?;

        let username = values.get_string("username").ok_or_else(|| {
            CredentialError::Provider("missing required field 'username'".to_owned())
        })?;

        let password = values.get_string("password").ok_or_else(|| {
            CredentialError::Provider("missing required field 'password'".to_owned())
        })?;

        let secret = SecretString::new(password.to_owned());
        Ok(StaticResolveResult::Complete(DatabaseAuth::new(
            host, port, database, username, secret,
        )))
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
                let pw = auth.password().expose_secret(|s| s.to_owned());
                assert_eq!(pw, "super-secret");
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
    async fn resolve_returns_error_on_invalid_port() {
        let mut values = ParameterValues::new();
        values.set(
            "host".to_owned(),
            serde_json::Value::String("localhost".into()),
        );
        values.set(
            "port".to_owned(),
            serde_json::Value::String("not-a-number".into()),
        );
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
}
