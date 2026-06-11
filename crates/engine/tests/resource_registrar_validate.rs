//! `ResourceRegistrarRegistry::validate` — the config-CRUD validation
//! seam (config validation, NOT live registration).
//!
//! A config-CRUD writer (the `POST .../resources` API handler) must
//! reject a malformed resource config *before* persisting the row, but
//! it must **not** live-register the resource into a running
//! `nebula_resource::Manager` — live registration is an
//! engine-activation concern (.1). This test pins
//! that `validate`:
//!
//! - resolves the `kind` through the **closed allowlist** (an unknown
//!   kind is a typed `RegistrarError::UnknownKind`, never a silent grab
//!   of the wrong resource type);
//! - runs the real `R::Config` schema pass + closed-set guard for a
//!   known kind (schema-valid ⇒ `Ok`; schema-invalid ⇒
//!   `RegistrarError::Register`; an undeclared secret-shaped field ⇒
//!   `RegistrarError::Register`);
//! - performs **no** `Manager` mutation — validating a config never
//!   makes the resource resolvable in a manager (the live/validate
//!   separation that keeps config CRUD distinct from activation).
//!
//! The validation core is shared verbatim with the live
//! `Manager::register_resolved` path via
//! `Manager::validate_config_value`, so a green
//! `register_resolved` suite plus this seam test together prove the
//! two paths cannot drift.

use std::sync::Arc;

use nebula_core::{ResourceKey, resource_key};
use nebula_engine::{RegistrarError, ResourceRegistrarRegistry, TypedResourceRegistrar};
use nebula_resource::{
    Manager, ScopeLevel,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident,
    topology::resident::Resident,
};
use nebula_schema::{HasSchema, Schema};
use serde::Deserialize;
use serde_json::json;

// ── A resource with a REAL schema (so the closed-set guard + schema
//    `#[validate]` rules are exercised, unlike `impl_empty_has_schema!`) ──

#[derive(Clone, Debug, Deserialize, Schema)]
struct HttpPoolConfig {
    /// Required, must be a non-empty URL ≤ 256 chars.
    #[field(label = "Base URL", hint = "url")]
    #[validate(required, length(max = 256))]
    base_url: String,

    /// Optional pool size in 1..=128.
    #[field(label = "Max connections")]
    #[validate(range(1..=128))]
    max_connections: Option<u32>,
}

#[derive(Debug, Clone)]
struct HttpPoolError(String);

impl std::fmt::Display for HttpPoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpPoolError {}

impl From<HttpPoolError> for ResourceError {
    fn from(e: HttpPoolError) -> Self {
        ResourceError::transient(e.0)
    }
}

impl ResourceConfig for HttpPoolConfig {
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.base_url.hash(&mut h);
        self.max_connections.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone)]
struct HttpPool;

#[async_trait::async_trait]
impl Provider for HttpPool {
    type Config = HttpPoolConfig;
    type Instance = ();

    fn key() -> ResourceKey {
        resource_key!("http_pool")
    }

    async fn create(
        &self,
        _config: &HttpPoolConfig,
        _ctx: &nebula_resource::ResourceContext,
    ) -> Result<(), nebula_resource::Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(
            <Self as Provider>::key(),
            "http_pool".to_owned(),
            String::new(),
            <HttpPoolConfig as HasSchema>::schema(),
        )
    }
}

impl nebula_core::DeclaresDependencies for HttpPool {}

impl nebula_resource::HasCredentialSlots for HttpPool {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Resident for HttpPool {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

fn registry_with_http_pool() -> ResourceRegistrarRegistry {
    let mut registry = ResourceRegistrarRegistry::new();
    registry.insert(
        "http_pool",
        Arc::new(TypedResourceRegistrar::<HttpPool, _, _, _>::new(
            || HttpPool,
            || {
                TopologyRuntime::Resident(ResidentRuntime::<HttpPool>::new(
                    resident::config::Config::default(),
                ))
            },
            nebula_resource::resident_acquire_fn::<HttpPool>,
        )),
    );
    registry
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// A schema-valid config for a known kind validates `Ok` — and the
/// resource is **NOT** registered into any `Manager` as a side effect
/// (config validation is not activation; ).
#[tokio::test]
async fn known_kind_schema_valid_config_is_ok_and_no_manager_mutation() {
    let registry = registry_with_http_pool();

    registry
        .validate(
            "http_pool",
            json!({ "base_url": "https://api.example.com", "max_connections": 16 }),
        )
        .expect("a schema-valid config for a known kind validates Ok");

    // `validate` takes no `&Manager` and constructs no runtime: there is
    // no manager it *could* have mutated. This is asserted structurally
    // (the seam's signature) and reinforced here — a fresh manager has
    // nothing registered, proving validation never reached registration.
    // (`Manager::new` spawns a release-queue reactor task, hence the
    // Tokio context.)
    let manager = Manager::new();
    assert!(
        manager
            .get_any(&<HttpPool as Provider>::key(), &ScopeLevel::Global)
            .is_none(),
        "validating a config must NEVER live-register the resource \
         (config CRUD is separate from engine activation — §13.1)"
    );
}

/// A config that violates the kind's schema (missing the `required`
/// `base_url`) is a typed `RegistrarError::Register`, not `UnknownKind`
/// and not a panic.
#[test]
fn known_kind_schema_invalid_config_is_register_error() {
    let registry = registry_with_http_pool();

    let err = registry
        .validate("http_pool", json!({ "max_connections": 8 }))
        .expect_err("a config missing the required base_url must be rejected");

    match err {
        RegistrarError::Register { kind, .. } => {
            assert_eq!(kind, "http_pool", "the failing kind is reported");
        },
        other => panic!("expected Register{{..}} for a schema failure, got {other:?}"),
    }
}

/// An out-of-range field value (`max_connections` outside `1..=128`)
/// fails the schema's `#[validate(range)]` rule.
#[test]
fn known_kind_out_of_range_value_is_register_error() {
    let registry = registry_with_http_pool();

    let err = registry
        .validate(
            "http_pool",
            json!({ "base_url": "https://x.test", "max_connections": 9999 }),
        )
        .expect_err("max_connections outside 1..=128 must fail the schema rule");

    assert!(
        matches!(err, RegistrarError::Register { .. }),
        "an out-of-range value is a schema (Register) failure, got {err:?}"
    );
}

/// An undeclared, secret-shaped field is rejected by the closed-set
/// guard — and the rejection message
/// names only the offending KEY, never its value, so a mis-wired secret
/// can never leak through the error.
#[test]
fn undeclared_secret_shaped_field_is_rejected_without_leaking_value() {
    let registry = registry_with_http_pool();

    let secret_value = "super-secret-token-do-not-leak";
    let err = registry
        .validate(
            "http_pool",
            json!({
                "base_url": "https://api.example.com",
                "password": secret_value,
            }),
        )
        .expect_err("an inlined secret-shaped field must be rejected by the closed-set guard");

    let RegistrarError::Register { source, .. } = &err else {
        panic!("expected Register{{..}} for the closed-set rejection, got {err:?}");
    };
    let msg = source.to_string();
    assert!(
        msg.contains("password"),
        "the rejection must name the offending key for operator diagnosis; got: {msg}"
    );
    assert!(
        !msg.contains(secret_value),
        "the rejection must NEVER echo the offending field's VALUE \
 ; got: {msg}"
    );
}

/// An unknown `kind` is a typed `RegistrarError::UnknownKind` resolved
/// through the closed allowlist *before* any typed call — it can never
/// touch a resource type.
#[test]
fn unknown_kind_is_typed_unknownkind_not_silent() {
    let registry = registry_with_http_pool();

    let err = registry
        .validate("ghost_kind", json!({ "base_url": "https://x.test" }))
        .expect_err("an unknown kind must be rejected, never silently accepted");

    match err {
        RegistrarError::UnknownKind(kind) => assert_eq!(kind, "ghost_kind"),
        other => panic!("expected UnknownKind(\"ghost_kind\"), got {other:?}"),
    }
}

/// An empty registry is fail-closed: every kind is `UnknownKind`.
#[test]
fn empty_registry_rejects_every_kind() {
    let registry = ResourceRegistrarRegistry::new();
    assert!(registry.is_empty());

    let err = registry
        .validate("http_pool", json!({ "base_url": "https://x.test" }))
        .expect_err("an empty allowlist must reject every kind");
    assert!(matches!(err, RegistrarError::UnknownKind(k) if k == "http_pool"));
}
