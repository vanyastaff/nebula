//! LDAP protocol — FlowProtocol implementation.
//!
//! Stores bind credentials as [`LdapState`] without performing a network
//! bind at initialization time. Full network verification (using the `ldap3`
//! crate) is planned for Phase 6.
//!
//! Use via `#[credential(extends = LdapProtocol)]` + `#[ldap(...)]`.

pub mod config;

pub use config::{LdapConfig, TlsMode};

use serde::{Deserialize, Serialize};

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;

use crate::core::result::InitializeResult;
use crate::core::{CredentialContext, CredentialError, CredentialState, ValidationError};
use crate::traits::FlowProtocol;

/// Persisted state after a successful LDAP bind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapState {
    pub host: String,
    pub port: u16,
    pub bind_dn: String,
    pub bind_password: String,
    pub tls: TlsMode,
}

impl CredentialState for LdapState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "ldap";
    fn scrub_ephemeral(&mut self) {}
}

/// LDAP bind protocol.
///
/// Collects host, port, bind DN, and password from the user, storing them
/// as [`LdapState`] for later use by an LDAP resource client.
///
/// # Note
/// Network connectivity is not verified at initialization time.
/// Phase 6 will add an optional test-bind step.
pub struct LdapProtocol;

impl FlowProtocol for LdapProtocol {
    type Config = LdapConfig;
    type State = LdapState;

    fn parameters() -> ParameterCollection {
        let mut host = TextParameter::new("host", "LDAP Host");
        host.metadata.required = true;
        host.metadata.placeholder = Some("ldap.example.com".into());

        let mut port = TextParameter::new("port", "Port");
        port.metadata.placeholder = Some("389".into());

        let mut bind_dn = TextParameter::new("bind_dn", "Bind DN");
        bind_dn.metadata.required = true;
        bind_dn.metadata.placeholder = Some("cn=admin,dc=example,dc=com".into());

        let mut bind_password = SecretParameter::new("bind_password", "Bind Password");
        bind_password.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(host))
            .with(ParameterDef::Text(port))
            .with(ParameterDef::Text(bind_dn))
            .with(ParameterDef::Secret(bind_password))
    }

    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let host = values
            .get_string("host")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: host".into()),
            })?
            .to_owned();

        let port = values
            .get_string("port")
            .unwrap_or("389")
            .parse::<u16>()
            .unwrap_or(389);

        let bind_dn = values
            .get_string("bind_dn")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: bind_dn".into()),
            })?
            .to_owned();

        let bind_password = values
            .get_string("bind_password")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat(
                    "missing required field: bind_password".into(),
                ),
            })?
            .to_owned();

        Ok(InitializeResult::Complete(LdapState {
            host,
            port,
            bind_dn,
            bind_password,
            tls: config.tls,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CredentialContext;

    #[test]
    fn parameters_has_required_fields() {
        let params = LdapProtocol::parameters();
        assert!(params.contains("host"));
        assert!(params.contains("port"));
        assert!(params.contains("bind_dn"));
        assert!(params.contains("bind_password"));
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn ldap_config_defaults() {
        let config = LdapConfig::default();
        assert_eq!(config.tls, TlsMode::None);
        assert_eq!(config.timeout.as_secs(), 30);
        assert!(config.ca_cert.is_none());
    }

    #[tokio::test]
    async fn initialize_stores_credentials() {
        let config = LdapConfig::default();
        let mut values = ParameterValues::new();
        values.set("host", serde_json::json!("ldap.example.com"));
        values.set("port", serde_json::json!("389"));
        values.set("bind_dn", serde_json::json!("cn=admin,dc=example,dc=com"));
        values.set("bind_password", serde_json::json!("secret"));

        let mut ctx = CredentialContext::new("test");
        let result = LdapProtocol::initialize(&config, &values, &mut ctx)
            .await
            .unwrap();

        match result {
            InitializeResult::Complete(state) => {
                assert_eq!(state.host, "ldap.example.com");
                assert_eq!(state.port, 389);
                assert_eq!(state.bind_dn, "cn=admin,dc=example,dc=com");
                assert_eq!(state.bind_password, "secret");
                assert_eq!(state.tls, TlsMode::None);
            }
            other => panic!("expected Complete, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_host_returns_error() {
        let config = LdapConfig::default();
        let values = ParameterValues::new();
        let mut ctx = CredentialContext::new("test");
        let result = LdapProtocol::initialize(&config, &values, &mut ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn port_defaults_to_389_when_missing() {
        let config = LdapConfig::default();
        let mut values = ParameterValues::new();
        values.set("host", serde_json::json!("ldap.example.com"));
        values.set("bind_dn", serde_json::json!("cn=admin,dc=example,dc=com"));
        values.set("bind_password", serde_json::json!("secret"));

        let mut ctx = CredentialContext::new("test");
        let result = LdapProtocol::initialize(&config, &values, &mut ctx)
            .await
            .unwrap();

        match result {
            InitializeResult::Complete(state) => assert_eq!(state.port, 389),
            other => panic!("expected Complete, got: {other:?}"),
        }
    }
}
