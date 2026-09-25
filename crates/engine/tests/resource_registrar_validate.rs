//! `ResourceActivatorRegistry::validate` — the config-CRUD validation
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
//! `ResourceFactory::register` path through the same admitted schema, so a green
//! `register_resolved` suite plus this seam test together prove the
//! two paths cannot drift.

use std::{collections::BTreeMap, sync::Arc};

use nebula_core::{
    CredentialId, Dependencies, ResourceKey, SlotField, credential_key, dependencies::SlotKind,
    resource_key,
};
use nebula_engine::{KindActivator, RegistrarError, ResourceActivatorRegistry};
use nebula_resource::Resident;
use nebula_resource::{
    HasCredentialSlots, Manager, ScopeLevel,
    error::Error as ResourceError,
    rate_limit::{Rate, ResiliencePolicy},
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::resident,
    topology::resident::ResidentProvider,
};
use nebula_schema::Schema;
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
    type Topology = Resident<Self>;

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

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            <Self as Provider>::key(),
            nebula_resource::metadata_name!("http_pool"),
            String::new(),
        )
    }

    /// The provider allows 30 requests per second, 30 back to back.
    fn resilience() -> ResiliencePolicy {
        let thirty = std::num::NonZeroU32::new(30).expect("non-zero");
        ResiliencePolicy::new().rate(
            Rate::per_second(thirty)
                .with_burst(thirty)
                .expect("valid burst"),
        )
    }
}

impl nebula_core::DeclaresDependencies for HttpPool {}

nebula_resource::no_credential_slots!(HttpPool);

#[async_trait::async_trait]
impl ResidentProvider for HttpPool {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

fn registry_with_http_pool() -> ResourceActivatorRegistry {
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "http_pool",
            Arc::new(KindActivator::<HttpPool, _, _>::new(
                || HttpPool,
                nebula_resource::topology::fixed(|| {
                    Resident::<HttpPool>::new(resident::config::Config::default())
                }),
            )),
        )
        .expect("test resource metadata admits");
    registry
}

// ── A kind with one required credential slot, so the binding rules beyond
//    "declares no slot" are exercised ─────────────────────────────────────────

#[derive(Clone)]
struct TokenPool;

#[async_trait::async_trait]
impl Provider for TokenPool {
    type Config = HttpPoolConfig;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("token_pool")
    }

    async fn create(
        &self,
        _config: &HttpPoolConfig,
        _ctx: &nebula_resource::ResourceContext,
    ) -> Result<(), nebula_resource::Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            <Self as Provider>::key(),
            nebula_resource::metadata_name!("token_pool"),
            String::new(),
        )
    }
}

impl nebula_core::DeclaresDependencies for TokenPool {
    fn dependencies() -> Dependencies {
        Dependencies::new().slot_field(SlotField {
            slot_key: "api_token",
            default_id: "api_token",
            kind: SlotKind::Credential {
                type_id: std::any::TypeId::of::<()>(),
                type_name: std::any::type_name::<()>(),
                key: credential_key!("token_pool.api_token"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

impl HasCredentialSlots for TokenPool {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["api_token"]
    }
}

#[async_trait::async_trait]
impl ResidentProvider for TokenPool {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

fn registry_with_token_pool() -> ResourceActivatorRegistry {
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "token_pool",
            Arc::new(KindActivator::<TokenPool, _, _>::new(
                || TokenPool,
                nebula_resource::topology::fixed(|| {
                    Resident::<TokenPool>::new(resident::config::Config::default())
                }),
            )),
        )
        .expect("test resource metadata admits");
    registry
}

fn bindings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(slot, selector)| ((*slot).to_owned(), (*selector).to_owned()))
        .collect()
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

/// The operator may slow a kind down but not speed it past what its author
/// declared; the refusal names the field and the rule, never the values, so
/// the API can return it verbatim.
#[test]
fn resilience_override_is_bounded_by_the_kind_policy() {
    let registry = registry_with_http_pool();

    registry
        .validate_resilience_override("http_pool", None)
        .expect("no override enforces the declared policy");
    registry
        .validate_resilience_override(
            "http_pool",
            Some(&json!({ "rate": { "requests": 5, "period_ms": 1000 } })),
        )
        .expect("a slower rate is a tightening");

    for (document, field) in [
        (
            json!({ "rate": { "requests": 4242, "period_ms": 1000 } }),
            "resilience_override.rate:",
        ),
        (
            json!({ "rate": { "requests": 5, "period_ms": 1000, "burst": 4242 } }),
            "resilience_override.rate:",
        ),
        (json!({ "rate": "4242 per second" }), "resilience_override:"),
        (json!({ "requests": 4242 }), "resilience_override:"),
    ] {
        let RegistrarError::Register { source, .. } = registry
            .validate_resilience_override("http_pool", Some(&document))
            .expect_err("the policy refuses it")
        else {
            panic!("expected a Register error for {document}");
        };
        let message = source.to_string();
        assert!(message.contains(field), "{message}");
        assert!(!message.contains("4242"), "values never echo: {message}");
    }
}

/// A row's credential bindings are checked against the kind's declared slots
/// before the row is stored, so a binding every activation would refuse never
/// reaches the catalog. The refusal names the slot and the rule, never the
/// selector.
#[test]
fn credential_bindings_are_checked_against_the_declared_slots() {
    let credential = CredentialId::new().to_string();
    let registry = registry_with_token_pool();
    registry
        .validate_credential_bindings("token_pool", &bindings(&[("api_token", &credential)]))
        .expect("the declared slot is bound to a credential id");

    for (row, rule) in [
        (bindings(&[]), "required slot `api_token` is not bound"),
        (
            bindings(&[("api_token", "not-a-credential-id")]),
            "slot `api_token` must name a credential id",
        ),
        (
            bindings(&[("api_token", &credential), ("audit", &credential)]),
            "slot `audit` is not declared",
        ),
    ] {
        let RegistrarError::Register { kind, source } = registry
            .validate_credential_bindings("token_pool", &row)
            .expect_err(rule)
        else {
            panic!("expected a Register error: {rule}");
        };
        assert_eq!(kind, "token_pool");
        let message = source.to_string();
        assert!(message.contains(rule), "{message}");
        assert!(
            !message.contains(&credential) && !message.contains("not-a-credential-id"),
            "selectors never echo: {message}"
        );
    }
}

/// A kind without credential slots takes no bindings at all, and an unknown
/// kind is refused before its bindings are looked at.
#[test]
fn credential_bindings_need_a_known_kind_that_declares_the_slot() {
    let registry = registry_with_http_pool();
    registry
        .validate_credential_bindings("http_pool", &BTreeMap::new())
        .expect("no slots, no bindings");
    let error = registry
        .validate_credential_bindings("http_pool", &bindings(&[("api_token", "cred_x")]))
        .expect_err("http_pool declares no slot");
    assert!(
        matches!(error, RegistrarError::Register { .. }),
        "{error:?}"
    );
    assert!(matches!(
        registry.validate_credential_bindings("ghost_kind", &BTreeMap::new()),
        Err(RegistrarError::UnknownKind(kind)) if kind == "ghost_kind"
    ));
}

/// A kind with a fixed topology takes no operator topology settings.
#[test]
fn fixed_topology_rejects_operator_settings() {
    let registry = registry_with_http_pool();
    registry
        .validate_topology("http_pool", None)
        .expect("defaults are always fine");
    assert!(
        registry
            .validate_topology("http_pool", Some(&json!({ "max_size": 4 })))
            .is_err()
    );
}

/// An empty registry is fail-closed: every kind is `UnknownKind`.
#[test]
fn empty_registry_rejects_every_kind() {
    let registry = ResourceActivatorRegistry::new();
    assert!(registry.is_empty());

    let err = registry
        .validate("http_pool", json!({ "base_url": "https://x.test" }))
        .expect_err("an empty allowlist must reject every kind");
    assert!(matches!(err, RegistrarError::UnknownKind(k) if k == "http_pool"));
}
