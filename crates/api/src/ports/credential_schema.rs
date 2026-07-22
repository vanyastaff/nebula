//! API-owned credential catalog/form read-model port.
//!
//! `nebula-api` never imports `nebula-schema`/`nebula-validator` types into
//! its DTOs; this object-safe port carries only api-safe values. The
//! concrete impl projects the credential registry's `json_schema()` output.
//! It is deliberately not a mutation validator: authenticated commands are
//! authorized and validated once inside the credential controller/service.
//! When this port is absent, only catalog/form discovery returns an honest
//! 503; credential mutations remain governed by their command gateway.

use serde::Serialize;
use utoipa::ToSchema;

/// Coarse, API-owned location for a credential validation failure.
///
/// A free-form JSON Pointer is deliberately not accepted at this trust seam:
/// validator adapters are allowed to inspect secret input, so even a
/// syntactically valid path must be treated as untrusted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialValidationLocation {
    /// The registered credential type key.
    CredentialKey,
    /// The credential data object. Individual secret-bearing field names are
    /// intentionally not exposed across the port.
    Data,
    /// Interactive continuation input.
    UserInput,
}

impl CredentialValidationLocation {
    /// Stable RFC 6901 pointer owned by the API.
    #[must_use]
    pub const fn pointer(self) -> &'static str {
        match self {
            Self::CredentialKey => "/credential_key",
            Self::Data => "/data",
            Self::UserInput => "/user_input",
        }
    }
}

/// Closed, value-free validation vocabulary crossing credential API ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialValidationCode {
    /// A required value is absent.
    Required,
    /// Text is shorter than allowed.
    MinLength,
    /// Text is longer than allowed.
    MaxLength,
    /// A number is below its lower bound.
    Min,
    /// A number is above its upper bound.
    Max,
    /// A value has an invalid format.
    InvalidFormat,
    /// A value has the wrong type.
    TypeMismatch,
    /// An object key is invalid.
    InvalidKey,
    /// A value exceeds the nesting limit.
    RecursionLimit,
    /// The credential type is not registered.
    UnknownCredentialType,
    /// Interactive input cannot be decoded.
    UserInputInvalid,
    /// A lower-layer code is not part of the public vocabulary.
    Invalid,
}

impl CredentialValidationCode {
    /// Collapse an untrusted lower-layer code into the closed public
    /// vocabulary without retaining the source string.
    #[must_use]
    pub fn from_untrusted(code: &str) -> Self {
        match code {
            "required" => Self::Required,
            "min_length" => Self::MinLength,
            "max_length" => Self::MaxLength,
            "min" => Self::Min,
            "max" => Self::Max,
            "invalid_format" => Self::InvalidFormat,
            "type_mismatch" => Self::TypeMismatch,
            "invalid_key" => Self::InvalidKey,
            "recursion_limit" => Self::RecursionLimit,
            "unknown_credential_type" => Self::UnknownCredentialType,
            "credential.user_input_invalid" => Self::UserInputInvalid,
            _ => Self::Invalid,
        }
    }

    /// Stable machine-readable API code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::MinLength => "min_length",
            Self::MaxLength => "max_length",
            Self::Min => "min",
            Self::Max => "max",
            Self::InvalidFormat => "invalid_format",
            Self::TypeMismatch => "type_mismatch",
            Self::InvalidKey => "invalid_key",
            Self::RecursionLimit => "recursion_limit",
            Self::UnknownCredentialType => "unknown_credential_type",
            Self::UserInputInvalid => "credential.user_input_invalid",
            Self::Invalid => "credential.invalid",
        }
    }

    /// API-owned, value-free human message.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Required => "value is required",
            Self::MinLength => "value is too short",
            Self::MaxLength => "value is too long",
            Self::Min => "value is below the allowed minimum",
            Self::Max => "value is above the allowed maximum",
            Self::InvalidFormat => "value does not match the required format",
            Self::TypeMismatch => "value has the wrong type",
            Self::InvalidKey => "object contains an invalid field key",
            Self::RecursionLimit => "value is nested too deeply",
            Self::UnknownCredentialType => "no such credential type",
            Self::UserInputInvalid => "interactive input is invalid",
            Self::Invalid => "credential data failed schema validation",
        }
    }
}

/// API-safe capability flags for a credential type (no lower-layer
/// `Capabilities` type crosses the seam).
#[derive(Debug, Clone, Copy, Default, Serialize, ToSchema)]
pub struct CredentialCapabilityFlags {
    /// Multi-step user interaction (OAuth redirect, device code).
    pub interactive: bool,
    /// Token refresh (OAuth2 `refresh_token`).
    pub refreshable: bool,
    /// Connection testing.
    pub testable: bool,
    /// Explicit revocation.
    pub revocable: bool,
}

/// Catalog descriptor for one credential type. `schema_json` is the raw
/// `ValidSchema::json_schema()` export; the api applies the public
/// projection (strips `x-nebula-root-rules` + predicate operands) before
/// it reaches the wire.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialTypeDescriptor {
    /// Stable type key (e.g. `api_key`, `oauth2`).
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Authentication-pattern classification (stringified).
    pub auth_pattern: String,
    /// Capability flags.
    pub capabilities: CredentialCapabilityFlags,
    /// Optional icon identifier or URL.
    pub icon: Option<String>,
    /// Optional documentation link.
    pub documentation_url: Option<String>,
    /// JSON Schema describing the credential's input fields. Raw export
    /// from `ValidSchema::json_schema()`; the api projects it before the
    /// wire (strips `x-nebula-root-rules` + predicate operands).
    pub schema_json: serde_json::Value,
}

/// Resolve a credential type's schema for the write-path gate (V2) and the
/// catalog read-model (V3). Implemented in the composition root over a
/// `nebula_credential::CredentialRegistry`.
pub trait CredentialSchemaPort: Send + Sync + 'static {
    /// All known credential types (raw `json_schema()` in `schema_json`;
    /// the api applies the public projection before serializing).
    fn list_types(&self) -> Vec<CredentialTypeDescriptor>;

    /// One credential type by key, if known.
    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_validation_text_is_not_retained() {
        const CANARY: &str = "submitted-secret-in-validator-code";
        let code = CredentialValidationCode::from_untrusted(CANARY);

        assert_eq!(code, CredentialValidationCode::Invalid);
        assert_eq!(CredentialValidationLocation::Data.pointer(), "/data");
        assert!(!format!("{code:?}").contains(CANARY));
        assert!(!code.message().contains(CANARY));
    }
}
