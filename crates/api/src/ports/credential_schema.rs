//! API-owned credential-schema port (ADR-0052 P4).
//!
//! `nebula-api` never imports `nebula-schema`/`nebula-validator` types into
//! its DTOs; this object-safe port carries only api-safe values. The
//! concrete impl lives in the composition root (which legally depends on
//! `nebula-credential`/`nebula-schema`) and runs `ValidSchema::validate` /
//! `json_schema()` — authority sits with the validator
//! (INTEGRATION_MODEL §29/§33 proof-token custody unchanged). When no port
//! is wired the credential write path and credential-type catalog return an
//! honest 503, mirroring `AppState::action_registry` (canon §4.5).

use serde::Serialize;
use utoipa::ToSchema;

/// One field-level validation failure — secret-safe by construction:
/// an RFC-6901 path, a validator code, and a static message. **Never**
/// the submitted value (ADR-0034 redaction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialFieldError {
    /// RFC-6901 JSON Pointer to the offending field (e.g. `/api_key`).
    pub path: String,
    /// Validator vocabulary code (e.g. `required`, `min_length`).
    pub code: String,
    /// Static, value-free human message.
    pub message: String,
}

/// Catalog descriptor for one credential type. `schema_json` is the
/// public-projected JSON Schema (`x-nebula-root-rules` and predicate
/// operands already stripped by the api-owned projection).
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
    /// Validate `data` against the credential type's resolved schema
    /// **before persist**. `Err` is a secret-safe field-error list.
    fn validate_data(
        &self,
        credential_key: &str,
        data: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>>;

    /// All known credential types (raw `json_schema()` in `schema_json`;
    /// the api applies the public projection before serializing).
    fn list_types(&self) -> Vec<CredentialTypeDescriptor>;

    /// One credential type by key, if known.
    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor>;
}
