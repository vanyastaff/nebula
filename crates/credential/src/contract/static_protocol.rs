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
//!     fn parameters() -> ValidSchema {
//!         Schema::builder()
//!             .add(Field::string("host").required())
//!             .add(Field::integer("port").default(json!(5432)))
//!             .build()
//!             .expect("static schema is valid")
//!     }
//!
//!     fn build(values: &FieldValues) -> Result<ConnectionUri, CredentialError> {
//!         let host = values.get_string_by_str("host").unwrap_or_default();
//!         Ok(ConnectionUri::new(SecretString::new(
//!             format!("postgres://user:pass@{host}:5432/db")
//!         )))
//!     }
//! }
//! ```

use nebula_schema::{FieldValues, ValidSchema};

use crate::{AuthScheme, error::CredentialError};

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
/// use nebula_schema::{FieldValues, Schema, ValidSchema};
///
/// struct ApiKeyProtocol;
///
/// impl StaticProtocol for ApiKeyProtocol {
///     type Scheme = SecretToken;
///
///     fn parameters() -> ValidSchema {
///         Schema::builder().build().expect("empty schema is valid")
///     }
///
///     fn build(values: &FieldValues) -> Result<SecretToken, CredentialError> {
///         let token = values
///             .get_string_by_str("token")
///             .ok_or_else(|| CredentialError::InvalidInput("missing token".into()))?;
///         Ok(SecretToken::new(SecretString::new(token.to_owned())))
///     }
/// }
/// ```
pub trait StaticProtocol: Send + Sync + 'static {
    /// Typed shape of the setup-form fields — same role as
    /// [`Credential::Input`](crate::Credential::Input).
    ///
    /// The default [`parameters()`](StaticProtocol::parameters) impl derives
    /// the schema from this type. Use [`FieldValues`] for legacy protocols
    /// that do not declare a typed input.
    type Input: nebula_schema::HasSchema + Send + Sync + 'static;

    /// The auth scheme this protocol produces.
    type Scheme: AuthScheme;

    /// Parameter schema for the credential setup form.
    ///
    /// Returned as JSON to the frontend for form rendering. Default impl
    /// delegates to [`<Self::Input as HasSchema>::schema()`](nebula_schema::HasSchema::schema).
    fn parameters() -> ValidSchema
    where
        Self: Sized,
    {
        <Self::Input as nebula_schema::HasSchema>::schema()
    }

    /// Extract parameters and build auth material.
    ///
    /// Called by the framework during [`Credential::resolve()`](crate::Credential::resolve).
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::InvalidInput`] for missing or
    /// malformed parameters.
    fn build(values: &FieldValues) -> Result<Self::Scheme, CredentialError>
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use nebula_schema::FieldValues;

    use super::*;
    use crate::{SecretString, scheme::SecretToken};

    struct TestProtocol;

    impl StaticProtocol for TestProtocol {
        type Input = FieldValues;
        type Scheme = SecretToken;

        fn build(values: &FieldValues) -> Result<SecretToken, CredentialError> {
            let token = values
                .get_string_by_str("token")
                .ok_or_else(|| CredentialError::InvalidInput("missing token".into()))?;
            Ok(SecretToken::new(SecretString::new(token.to_owned())))
        }
    }

    #[test]
    fn build_returns_error_on_missing_parameter() {
        let values = FieldValues::new();
        let result = TestProtocol::build(&values);
        assert!(result.is_err());
    }

    #[test]
    fn build_produces_scheme_from_valid_parameters() {
        let mut values = FieldValues::new();
        values.set_raw("token", serde_json::json!("test-token"));
        let token = TestProtocol::build(&values).unwrap();
        let value = token.token().expose_secret(ToOwned::to_owned);
        assert_eq!(value, "test-token");
    }

    #[test]
    fn parameters_returns_empty_collection() {
        let params = TestProtocol::parameters();
        assert_eq!(params.fields().len(), 0);
    }
}
