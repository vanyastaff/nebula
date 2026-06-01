//! Concrete [`CredentialSchemaPort`] over a
//! `nebula_credential::CredentialRegistry` (credential-schema validation).
//!
//! Per the user's adjudication of the deny.toml-vs-#671 conflict, the
//! concrete impl lives in `nebula-api` (already an allow-listed
//! `nebula-credential` consumer; `nebula-schema` is Core — **no**
//! `deny.toml` change) rather than in the `nebula-server` composition
//! crate (which would have required a wrapper-allowlist edit). `nebula-api`
//! takes a `nebula-schema` production dep + `schemars`, but **no
//! `ValidSchema` type enters any DTO** — the port returns only
//! `serde_json::Value` / api-owned structs, so stub-endpoint policy's DTO-purity rule
//! is intact (only the informal "api never imports nebula-schema" prose is
//! relaxed; recorded in the credential-schema validation / stub-endpoint policy amendments).
//!
//! Authority sits with the validator: `ValidSchema::validate` is invoked
//! here; credential `data` is **never** `.resolve()`-d (credential secrecy —
//! secrets must not depend on workflow state). Errors are secret-safe by
//! construction (RFC-6901 path + validator code + static message; never a
//! submitted value — credential redaction).

use std::sync::Arc;

use nebula_credential::{
    AnyCredential, ApiKeyCredential, BasicAuthCredential, Capabilities, CredentialRegistry,
    OAuth2Credential,
};
use nebula_schema::FieldValues;

use crate::ports::credential_schema::{
    CredentialCapabilityFlags, CredentialFieldError, CredentialSchemaPort, CredentialTypeDescriptor,
};

/// Map a validator/schema vocabulary `code` to a **static, value-free**
/// message. The seam NEVER forwards a lower-layer `ValidationError.message`
/// (some validators embed the submitted input — e.g. `'{input}' is not a
/// valid IPv4 address` — and `validate_json_keys` embeds the raw object
/// key). Deriving the message from the code makes the credential redaction "never the
/// submitted value" invariant hold *by construction* at this seam,
/// independent of any upstream validator's message hygiene.
fn safe_field_message(code: &str) -> &'static str {
    match code {
        "required" => "value is required",
        "min_length" => "value is too short",
        "max_length" => "value is too long",
        "min" => "value is below the allowed minimum",
        "max" => "value is above the allowed maximum",
        "invalid_format" => "value does not match the required format",
        "type_mismatch" => "value has the wrong type",
        "invalid_key" => "object contains an invalid field key",
        "recursion_limit" => "value is nested too deeply",
        "unknown_credential_type" => "no such credential type",
        // Vocabulary codes are value-free tokens; echoing the code (not
        // the submitted value) is safe and stays informative.
        _ => "field failed schema validation",
    }
}

/// Build a secret-safe field error. `path` is the schema/RFC-6901
/// structural location (declared-field/parent path — value-free);
/// `message` is derived from `code`, never from the submitted value.
fn field_error(path: String, code: &str) -> CredentialFieldError {
    CredentialFieldError {
        path,
        code: code.to_owned(),
        message: safe_field_message(code).to_owned(),
    }
}

/// `CredentialSchemaPort` backed by a registered credential set.
pub struct RegistryCredentialSchema {
    registry: Arc<CredentialRegistry>,
}

impl RegistryCredentialSchema {
    /// Wrap a populated registry.
    #[must_use]
    pub fn new(registry: Arc<CredentialRegistry>) -> Self {
        Self { registry }
    }

    fn flags(caps: Capabilities) -> CredentialCapabilityFlags {
        CredentialCapabilityFlags {
            interactive: caps.contains(Capabilities::INTERACTIVE),
            refreshable: caps.contains(Capabilities::REFRESHABLE),
            testable: caps.contains(Capabilities::TESTABLE),
            revocable: caps.contains(Capabilities::REVOCABLE),
        }
    }

    fn descriptor(&self, any: &dyn AnyCredential) -> CredentialTypeDescriptor {
        let meta = any.metadata();
        let key = any.credential_key().to_owned();
        // Structural JSON-Schema export. On failure, an empty object
        // schema (never a panic); the api-owned public projection still
        // strips predicate operands at the wire (#6).
        let schema_json = meta
            .base
            .schema
            .json_schema()
            .ok()
            .and_then(|s| serde_json::to_value(&s).ok())
            .unwrap_or_else(|| serde_json::json!({ "type": "object" }));
        let caps = self
            .registry
            .capabilities_of(&key)
            .unwrap_or_else(Capabilities::empty);
        CredentialTypeDescriptor {
            key,
            name: meta.base.name.clone(),
            description: meta.base.description.clone(),
            auth_pattern: format!("{:?}", meta.pattern),
            capabilities: Self::flags(caps),
            icon: meta.base.icon.as_inline().map(str::to_owned),
            documentation_url: meta.base.documentation_url,
            schema_json,
        }
    }
}

impl CredentialSchemaPort for RegistryCredentialSchema {
    #[tracing::instrument(
        skip_all,
        fields(cred.key = %credential_key, outcome = tracing::field::Empty)
    )]
    fn validate_data(
        &self,
        credential_key: &str,
        data: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>> {
        let Some(any) = self.registry.resolve_any(credential_key) else {
            tracing::Span::current().record("outcome", "unknown_type");
            return Err(vec![field_error(
                "/credential_key".to_owned(),
                "unknown_credential_type",
            )]);
        };
        let schema = any.metadata().base.schema;
        // Ingest only (NEVER `.resolve()` — credential secrecy: credential data
        // must not be expression-resolved against workflow state).
        let values = FieldValues::from_json(data.clone()).map_err(|e| {
            vec![field_error(
                e.field.as_deref().unwrap_or("").to_owned(),
                e.code.as_ref(),
            )]
        })?;
        match schema.validate(&values) {
            Ok(_valid) => {
                tracing::Span::current().record("outcome", "ok");
                Ok(())
            },
            Err(report) => {
                let errors: Vec<CredentialFieldError> = report
                    .errors()
                    .map(|e| {
                        field_error(e.field.as_deref().unwrap_or("").to_owned(), e.code.as_ref())
                    })
                    .collect();
                tracing::Span::current().record("outcome", "rejected");
                debug_assert!(
                    !errors.is_empty(),
                    "validate() Err must yield at least one hard error"
                );
                Err(errors)
            },
        }
    }

    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        // `iter_compatible(empty)` enumerates every registered type
        // (registry.rs:212 — empty is a subset of any capability set).
        let keys: Vec<String> = self
            .registry
            .iter_compatible(Capabilities::empty())
            .map(|(k, _caps)| k.to_owned())
            .collect();
        keys.into_iter()
            .filter_map(|k| {
                self.registry
                    .resolve_any(&k)
                    .map(|any| self.descriptor(any))
            })
            .collect()
    }

    fn get_type(&self, credential_key: &str) -> Option<CredentialTypeDescriptor> {
        self.registry
            .resolve_any(credential_key)
            .map(|any| self.descriptor(any))
    }
}

/// Build the default port with the first-party credential types
/// registered (credential-schema validation). Used by the composition root so
/// `apps/server` needs no `nebula-credential`/`nebula-schema` dep.
///
/// # Errors
///
/// Returns [`nebula_credential::RegisterError`] if a credential KEY is
/// already registered (a composition bug — distinct first-party const
/// KEYs make this unreachable in practice, but the library returns the
/// typed error rather than panicking; the caller decides how to surface
/// it — AGENTS.md "no `expect()` in library code").
pub fn try_default_registry_port()
-> Result<Arc<dyn CredentialSchemaPort>, nebula_credential::RegisterError> {
    let mut registry = CredentialRegistry::new();
    registry.register(ApiKeyCredential, "nebula-credential")?;
    registry.register(BasicAuthCredential, "nebula-credential")?;
    registry.register(OAuth2Credential, "nebula-credential")?;
    Ok(Arc::new(RegistryCredentialSchema::new(Arc::new(registry))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port() -> RegistryCredentialSchema {
        let mut reg = CredentialRegistry::new();
        reg.register(ApiKeyCredential, "nebula-credential")
            .expect("api_key registers (statically unique key)");
        RegistryCredentialSchema::new(Arc::new(reg))
    }

    #[test]
    fn validate_rejects_missing_required_field_secret_safe() {
        let p = port();
        let data = serde_json::json!({ "server": "https://x", "leak": "NEVER-ECHO-9f" });
        let err = p
            .validate_data("api_key", &data)
            .expect_err("missing required api_key must reject");
        assert!(
            err.iter().any(|e| e.code == "required"),
            "expected a `required` code, got {err:?}"
        );
        assert!(
            !format!("{err:?}").contains("NEVER-ECHO-9f"),
            "field errors must not echo submitted values"
        );
    }

    #[test]
    fn validate_accepts_well_formed_data() {
        assert!(
            port()
                .validate_data("api_key", &serde_json::json!({ "api_key": "k-123" }))
                .is_ok()
        );
    }

    #[test]
    fn unknown_type_is_a_field_error_not_panic() {
        let err = port()
            .validate_data("does-not-exist", &serde_json::json!({}))
            .expect_err("unknown type rejects");
        assert_eq!(err[0].code, "unknown_credential_type");
        assert_eq!(err[0].path, "/credential_key");
    }

    #[test]
    fn get_type_exports_capable_descriptor_and_default_port_lists_first_party() {
        let p = port();
        let d = p.get_type("api_key").expect("api_key present");
        assert_eq!(d.key, "api_key");
        assert!(
            d.schema_json.get("properties").is_some(),
            "json_schema() export must carry properties: {:?}",
            d.schema_json
        );
        assert!(p.get_type("nope").is_none());

        // The composition default registers all three first-party types.
        let default = try_default_registry_port().expect("first-party set registers (unique KEYs)");
        let listed = default.list_types();
        for k in ["api_key", "basic_auth", "oauth2"] {
            assert!(
                listed.iter().any(|t| t.key == k),
                "default port must register {k}; got {:?}",
                listed.iter().map(|t| &t.key).collect::<Vec<_>>()
            );
        }
    }
}
