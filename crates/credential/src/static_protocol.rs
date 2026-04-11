//! Reusable protocol pattern for static (non-interactive) credentials.
//!
//! [`StaticProtocol`] captures the common pattern: extract parameters →
//! build auth material. Used by `#[derive(Credential)]` to auto-generate
//! the [`Credential::resolve()`](crate::Credential::resolve) method.
//!
//! # Examples
//!
//! ```ignore
//! use nebula_credential::StaticProtocol;
//! use nebula_credential::scheme::ConnectionUri;
//!
//! struct PostgresProtocol;
//!
//! impl StaticProtocol for PostgresProtocol {
//!     type Scheme = ConnectionUri;
//!
//!     fn parameters() -> ParameterCollection {
//!         ParameterCollection::new()
//!             .add(Parameter::string("host").required())
//!             .add(Parameter::integer("port").default(json!(5432)))
//!     }
//!
//!     fn build(values: &ParameterValues) -> Result<ConnectionUri, CredentialError> {
//!         let host = values.get_string("host").unwrap_or_default();
//!         Ok(ConnectionUri::new(SecretString::new(
//!             format!("postgres://user:pass@{host}:5432/db")
//!         )))
//!     }
//! }
//! ```

use nebula_core::AuthScheme;
use nebula_parameter::{ParameterCollection, values::ParameterValues};

use crate::error::CredentialError;

/// Reusable protocol for static (non-interactive) credentials.
///
/// Defines [`parameters()`](StaticProtocol::parameters) (form schema) and
/// [`build()`](StaticProtocol::build) (parameter extraction + auth material
/// construction). The `#[derive(Credential)]` macro uses this to generate a
/// full [`Credential`](crate::Credential) impl.
///
/// For interactive or refreshable credentials, implement
/// [`Credential`](crate::Credential) directly.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::StaticProtocol;
/// use nebula_credential::scheme::SecretToken;
/// use nebula_credential::SecretString;
/// use nebula_credential::CredentialError;
/// use nebula_parameter::ParameterCollection;
/// use nebula_parameter::values::ParameterValues;
///
/// struct ApiKeyProtocol;
///
/// impl StaticProtocol for ApiKeyProtocol {
///     type Scheme = SecretToken;
///
///     fn parameters() -> ParameterCollection {
///         ParameterCollection::new()
///     }
///
///     fn build(values: &ParameterValues) -> Result<SecretToken, CredentialError> {
///         let token = values
///             .get_string("token")
///             .ok_or_else(|| CredentialError::InvalidInput("missing token".into()))?;
///         Ok(SecretToken::new(SecretString::new(token.to_owned())))
///     }
/// }
/// ```
pub trait StaticProtocol: Send + Sync + 'static {
    /// The auth scheme this protocol produces.
    type Scheme: AuthScheme;

    /// Parameter schema for the credential setup form.
    ///
    /// Returned as JSON to the frontend for form rendering.
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Extract parameters and build auth material.
    ///
    /// Called by the framework during [`Credential::resolve()`](crate::Credential::resolve).
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidInput`] for missing or
    /// malformed parameters.
    fn build(values: &ParameterValues) -> Result<Self::Scheme, CredentialError>
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SecretString, scheme::SecretToken};

    struct TestProtocol;

    impl StaticProtocol for TestProtocol {
        type Scheme = SecretToken;

        fn parameters() -> ParameterCollection {
            ParameterCollection::new()
        }

        fn build(values: &ParameterValues) -> Result<SecretToken, CredentialError> {
            let token = values
                .get_string("token")
                .ok_or_else(|| CredentialError::InvalidInput("missing token".into()))?;
            Ok(SecretToken::new(SecretString::new(token.to_owned())))
        }
    }

    #[test]
    fn build_returns_error_on_missing_parameter() {
        let values = ParameterValues::new();
        let result = TestProtocol::build(&values);
        assert!(result.is_err());
    }

    #[test]
    fn build_produces_scheme_from_valid_parameters() {
        let mut values = ParameterValues::new();
        values.set("token", serde_json::json!("test-token"));
        let token = TestProtocol::build(&values).unwrap();
        let value = token.token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "test-token");
    }

    #[test]
    fn parameters_returns_empty_collection() {
        let params = TestProtocol::parameters();
        assert_eq!(params.len(), 0);
    }
}
