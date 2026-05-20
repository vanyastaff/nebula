//! `Manager::register_from_value` JSON-driven registration with `{{ }}`
//! template resolution + schema validation.
//!
//! The flow:
//!
//!   1. Resolve every `{{ }}` template inside the JSON tree via
//!      `ExpressionEngine::render_template`.
//!   2. Deserialize `R::Config` from the resolved JSON.
//!   3. Schema-validate the resolved JSON against `<R::Config as HasSchema>::schema()`.
//!   4. Validate `slot_bindings` keys against `R::dependencies()` (rejects configs whose credential
//!      surface diverged from the workflow JSON).
//!   5. Dispatch into the typed `Manager::register<R>(...)`.

use std::{collections::HashMap, sync::Arc};

use nebula_core::{
    CredentialKey, DeclaresDependencies, Dependencies, ResourceKey, ScopeLevel, resource_key,
};
use nebula_expression::ExpressionEngine;
use nebula_resource::{
    Manager, ResidentConfig, ResourceContext,
    error::Error,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use serde::Deserialize;
use serde_json::json;

// ── Test resource ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "fields exercised through ResourceConfig::validate + serde::Deserialize, not direct read"
)]
struct PgConfig {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
}

fn default_port() -> u16 {
    5432
}

nebula_schema::impl_empty_has_schema!(PgConfig);

impl ResourceConfig for PgConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            Err(Error::permanent("host must not be empty"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
struct PgError(String);

impl std::fmt::Display for PgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PgError {}

impl From<PgError> for Error {
    fn from(e: PgError) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct Postgres;

impl Resource for Postgres {
    type Config = PgConfig;
    type Runtime = Arc<()>;
    type Lease = Arc<()>;
    type Error = PgError;

    fn key() -> ResourceKey {
        resource_key!("phase9-pg")
    }

    async fn create(&self, _config: &PgConfig, _ctx: &ResourceContext) -> Result<Arc<()>, PgError> {
        Ok(Arc::new(()))
    }

    async fn destroy(&self, _runtime: Arc<()>) -> Result<(), PgError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for Postgres {
    fn is_alive_sync(&self, _runtime: &Arc<()>) -> bool {
        true
    }
}

impl DeclaresDependencies for Postgres {
    fn dependencies() -> Dependencies {
        // Default: no slot fields. Tests that exercise slot binding
        // declare the slot via a dedicated fixture below.
        Dependencies::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn register_from_value_resolves_template_and_registers() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // JSON config with a `{{ }}` template — a literal expression evaluates
    // to its string value at render time. The point is to exercise the
    // template render path, not to test the expression language itself.
    let config_json = json!({
        "host": "db-{{ \"example.com\" }}",
        "port": 5432,
    });

    manager
        .register_resolved::<Postgres>(
            config_json,
            &engine,
            HashMap::new(),
            Postgres,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<Postgres>::new(ResidentConfig::default())),
            Manager::erased_acquire_resident_for::<Postgres>(),
            None,
        )
        .await
        .expect("register_from_value must succeed");

    assert!(
        manager.contains(&Postgres::key()),
        "resource must be registered after register_from_value"
    );

    // The rendered config must be *observable*, not merely "registered":
    // assert the `{{ "example.com" }}` template was actually resolved into
    // the stored `PgConfig` (not passed through verbatim or dropped). This
    // pins that `register_from_value` threads the resolved config all the
    // way into the installed registry row through the collapsed
    // `RegistrationSpec` funnel.
    let managed = manager
        .lookup::<Postgres>(&ScopeLevel::Global)
        .expect("registered resident row must be resolvable");
    let stored = managed.config();
    assert_eq!(
        stored.host, "db-example.com",
        "the `{{{{ \"example.com\" }}}}` template must have rendered into the stored host"
    );
    assert_eq!(stored.port, 5432, "non-templated field must round-trip");
}

#[tokio::test]
async fn register_from_value_validates_schema_failure() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // `host` is a String per PgConfig — supplying a number must trip
    // serde::Deserialize. (The HasSchema impl is empty here so schema
    // validation is permissive; serde deserialize is the gate.)
    let config_json = json!({
        "host": 12345,
        "port": 5432,
    });

    let err = manager
        .register_resolved::<Postgres>(
            config_json,
            &engine,
            HashMap::new(),
            Postgres,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<Postgres>::new(ResidentConfig::default())),
            Manager::erased_acquire_resident_for::<Postgres>(),
            None,
        )
        .await
        .expect_err("must reject ill-typed config");

    let msg = err.to_string();
    assert!(
        msg.contains("deserialize"),
        "expected deserialize-related error, got: {msg}"
    );
}

#[tokio::test]
async fn register_from_value_resourceconfig_validate_fires() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // Empty `host` deserializes fine but fails ResourceConfig::validate().
    let config_json = json!({
        "host": "",
        "port": 5432,
    });

    let err = manager
        .register_resolved::<Postgres>(
            config_json,
            &engine,
            HashMap::new(),
            Postgres,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<Postgres>::new(ResidentConfig::default())),
            Manager::erased_acquire_resident_for::<Postgres>(),
            None,
        )
        .await
        .expect_err("must reject empty host");

    assert!(
        err.to_string().contains("host must not be empty"),
        "expected ResourceConfig::validate to surface, got: {err}"
    );
}

#[tokio::test]
async fn register_from_value_unknown_slot_binding_rejected() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // `Postgres` declares no credential slots; supplying a binding
    // for `auth` is a misconfiguration that must be caught at register
    // time rather than as a confusing rotation no-op later.
    let mut bindings = HashMap::new();
    bindings.insert("auth".to_owned(), CredentialKey::new("db_auth").unwrap());

    let err = manager
        .register_resolved::<Postgres>(
            json!({"host": "example.com",
            "port": 5432}),
            &engine,
            bindings,
            Postgres,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<Postgres>::new(ResidentConfig::default())),
            Manager::erased_acquire_resident_for::<Postgres>(),
            None,
        )
        .await
        .expect_err("unknown slot must be rejected");

    let msg = err.to_string();
    assert!(
        msg.contains("auth") && msg.contains("does not match any declared credential slot"),
        "expected slot-binding rejection message, got: {msg}"
    );
}

#[tokio::test]
async fn register_from_value_passthrough_no_templates() {
    // Plain JSON with no `{{ }}` markers — the engine fast-path is
    // exercised; resolution is a no-op walk.
    let manager = Manager::new();
    let engine = ExpressionEngine::new();
    let config_json = json!({
        "host": "static.example.com",
        "port": 5432,
    });

    manager
        .register_resolved::<Postgres>(
            config_json,
            &engine,
            HashMap::new(),
            Postgres,
            ScopeLevel::Global,
            TopologyRuntime::Resident(ResidentRuntime::<Postgres>::new(ResidentConfig::default())),
            Manager::erased_acquire_resident_for::<Postgres>(),
            None,
        )
        .await
        .expect("register_from_value must succeed for plain JSON");

    assert!(manager.contains(&Postgres::key()));
}
