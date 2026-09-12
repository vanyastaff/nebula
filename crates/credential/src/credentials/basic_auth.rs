//! HTTP Basic Auth credential -- static, non-interactive.
//!
//! Resolves a username + password pair into [`IdentityPassword`]. State and
//! Scheme are the same type via [`identity_state!`](crate::identity_state).

use nebula_schema::Schema;
use serde::Deserialize;

use crate::{
    CredentialContext, SecretString, error::CredentialError, metadata::CredentialMetadataDraft,
    resolve::StaticResolveResult, scheme::IdentityPassword,
};

/// Typed shape of the `basic_auth` credential setup form (Phase 5 — replaces
/// the legacy `BasicAuthInput`).
///
/// `#[derive(Schema)]` provides the `HasSchema` impl read via
/// `nebula_schema::schema_of::<Self::Properties>()` (schema-of properties). The
/// plaintext password lives
/// in a `String` here for schema derivation and is wrapped into
/// [`SecretString`] inside [`Credential::resolve`](crate::Credential::resolve)
/// before it leaves the resolver.
#[derive(Schema, Deserialize)]
pub struct BasicAuthProperties {
    /// Username for HTTP Basic authentication.
    #[field(label = "Username")]
    #[validate(required)]
    pub username: String,
    /// Password for HTTP Basic authentication.
    #[field(secret, label = "Password")]
    #[validate(required)]
    pub password: SecretString,
}

/// HTTP Basic Auth credential -- resolves username + password into
/// [`IdentityPassword`].
///
/// - **Non-interactive:** resolves in one step from user input. Per §15.4 sub-trait split, this
///   credential does *not* implement [`Interactive`](crate::Interactive).
/// - **Non-refreshable:** static credentials have no expiry. Does not implement
///   [`Refreshable`](crate::Refreshable).
/// - **Identity projection:** stored state is the scheme itself.
pub struct BasicAuthCredential;

// ADR-0088 D1: one `impl` block declares the whole credential. `#[credential]`
// sees only `project` + `resolve` (no capability methods) and emits the
// `Credential` impl, five all-`false` capability-report consts, and a static
// `CredentialLifecycle` policy — matching the absent capability sub-traits.
#[nebula_credential::credential(key = "basic_auth")]
impl BasicAuthCredential {
    type Properties = BasicAuthProperties;
    type Scheme = IdentityPassword;
    type State = IdentityPassword;

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("basic_auth"),
            crate::metadata_name!("Basic Auth"),
            "HTTP Basic authentication (username + password).",
        )
        .with_icon(nebula_metadata::Icon::inline("lock"))
    }

    fn project(state: &IdentityPassword) -> IdentityPassword {
        state.clone()
    }

    async fn resolve(
        properties: &BasicAuthProperties,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<IdentityPassword>, CredentialError> {
        Ok(StaticResolveResult::Complete(IdentityPassword::new(
            properties.username.clone(),
            properties.password.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    // `Credential` (for `KEY`) and `CredentialLifecycle` (for `policy`) are
    // only referenced by the tests now that `#[credential]` generates the
    // trait impls via absolute paths.
    use crate::{Credential, CredentialLifecycle, credentials::resolve_properties};

    use super::*;

    #[test]
    fn key_is_basic_auth() {
        assert_eq!(BasicAuthCredential::KEY, "basic_auth");
    }

    #[test]
    fn lifecycle_policy_is_static() {
        let auth = IdentityPassword::new("u", SecretString::new("p"));
        let p = BasicAuthCredential::policy(&auth);
        assert!(!p.is_expiring());
        assert!(!p.is_auto_renewable());
    }

    // Capability membership checks moved to compile-time: the absence
    // of `impl Interactive | Refreshable | Revocable | Testable | Dynamic`
    // for `BasicAuthCredential` is the type-level statement that this
    // credential is static. Probe 4 (compile_fail_engine_dispatch_capability)
    // pins this guarantee at the engine dispatch site.

    #[test]
    fn project_returns_clone_of_state() {
        let auth = IdentityPassword::new("admin", SecretString::new("s3cret"));
        let projected = BasicAuthCredential::project(&auth);
        assert_eq!(projected.identity(), "admin");
        let original = auth.password().expose_secret().to_owned();
        let cloned = projected.password().expose_secret().to_owned();
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_username_and_password() {
        let properties = resolve_properties::<BasicAuthCredential>(serde_json::json!({
            "username": "alice", "password": "p@ssw0rd"
        }))
        .unwrap();
        assert_eq!(properties.password.expose_secret(), "p@ssw0rd");
        let ctx = CredentialContext::for_owner("test-user");
        let result = BasicAuthCredential::resolve(&properties, &ctx)
            .await
            .unwrap();
        match result {
            StaticResolveResult::Complete(auth) => {
                assert_eq!(auth.identity(), "alice");
                let pw = auth.password().expose_secret().to_owned();
                assert_eq!(pw, "p@ssw0rd");
            },
            _ => panic!("expected Complete variant"),
        }
    }

    #[test]
    fn schema_rejects_missing_username_before_resolve() {
        let Err(report) = resolve_properties::<BasicAuthCredential>(serde_json::json!({
            "password": "secret"
        })) else {
            panic!("missing username must fail schema validation");
        };
        assert!(report.errors().any(|error| {
            error.code() == "required" && error.path().to_string() == "/username"
        }));
    }

    #[test]
    fn schema_rejects_missing_password_before_resolve() {
        let Err(report) = resolve_properties::<BasicAuthCredential>(serde_json::json!({
            "username": "alice"
        })) else {
            panic!("missing password must fail schema validation");
        };
        assert!(report.errors().any(|error| {
            error.code() == "required" && error.path().to_string() == "/password"
        }));
    }
}
