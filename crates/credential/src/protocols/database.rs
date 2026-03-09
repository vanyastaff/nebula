//! Database protocol — host, port, database, username, password, ssl_mode.

use serde::{Deserialize, Serialize};

use nebula_parameter::schema::{Field, Schema};
use nebula_parameter::values::ParameterValues;

use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::StaticProtocol;

/// State produced by [`DatabaseProtocol`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseState {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: String,
}

impl CredentialState for DatabaseState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "database";

    fn scrub_ephemeral(&mut self) {}
}

/// Protocol that contributes database connection fields.
///
/// Used as a base for any database-backed credential (Postgres, MySQL, etc.).
pub struct DatabaseProtocol;

impl StaticProtocol for DatabaseProtocol {
    type State = DatabaseState;

    fn parameters() -> Schema {
        Schema::new()
            .field(Field::text("host").with_label("Host").with_placeholder("localhost").required())
            .field(Field::text("port").with_label("Port").with_placeholder("5432"))
            .field(Field::text("database").with_label("Database").required())
            .field(Field::text("username").with_label("Username").required())
            .field(Field::text("password").with_label("Password").required().secret())
            .field(Field::text("ssl_mode").with_label("SSL Mode").with_placeholder("disable"))
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let host = values
            .get_string("host")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: host".into()),
            })?
            .to_owned();

        let port_str = values.get_string("port").unwrap_or("5432");
        let port = port_str
            .parse::<u16>()
            .map_err(|_| CredentialError::Validation {
                source: ValidationError::InvalidFormat(format!("invalid port: {port_str}")),
            })?;

        let database = values
            .get_string("database")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: database".into()),
            })?
            .to_owned();

        let username = values
            .get_string("username")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: username".into()),
            })?
            .to_owned();

        let password = values
            .get_string("password")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: password".into()),
            })?
            .to_owned();

        let ssl_mode = values
            .get_string("ssl_mode")
            .unwrap_or("disable")
            .to_owned();

        Ok(DatabaseState {
            host,
            port,
            database,
            username,
            password,
            ssl_mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parameters_are_complete() {
        let params = DatabaseProtocol::parameters();
        assert!(params.contains("host"));
        assert!(params.contains("port"));
        assert!(params.contains("database"));
        assert!(params.contains("username"));
        assert!(params.contains("password"));
        assert!(params.contains("ssl_mode"));
        assert_eq!(params.len(), 6);
    }

    #[test]
    fn build_state_with_defaults() {
        let mut values = ParameterValues::new();
        values.set("host", json!("localhost"));
        values.set("port", json!("5432"));
        values.set("database", json!("mydb"));
        values.set("username", json!("admin"));
        values.set("password", json!("pass"));
        let state = DatabaseProtocol::build_state(&values).unwrap();
        assert_eq!(state.host, "localhost");
        assert_eq!(state.port, 5432);
        assert_eq!(state.database, "mydb");
        assert_eq!(state.ssl_mode, "disable");
    }

    #[test]
    fn missing_host_returns_error() {
        let mut values = ParameterValues::new();
        values.set("database", json!("mydb"));
        values.set("username", json!("admin"));
        values.set("password", json!("pass"));
        assert!(DatabaseProtocol::build_state(&values).is_err());
    }
}
