//! Regression: `Manager::register_from_value` must reject a config that inlines
//! a secret-shaped field not declared by the typed `R::Config` schema.
//!
//! Invariant (product credential boundary; slot model; engine credential orchestration redaction;
//! credential isolation isolation): secrets reach a resource ONLY via typed credential
//! slots (credential *references*), NEVER inline in `ResourceEntry.config` —
//! `ResourceConfig` carries no secrets. `register_from_value` schema-validates
//! the JSON against `<R::Config as HasSchema>::schema()`; a JSON carrying an
//! inline secret-shaped field that is not part of that typed schema must be
//! rejected with a typed error, not silently accepted/ignored.
//!
//! Two cases prove the gate is *specific*, not a blanket failure:
//!   * negative — config with an extra `password` field is rejected, and the
//!     rejection error does NOT echo the secret value back (a validation error
//!     that leaks the inlined secret would itself be a disclosure);
//!   * positive control — the SAME resource with a clean config (only the
//!     schema's declared fields) registers OK, so the rejection is attributable
//!     to the extra secret-shaped field and the test is not vacuous.

use std::{collections::HashMap, sync::Arc};

use nebula_core::{DeclaresDependencies, Dependencies, ResourceKey, ScopeLevel, resource_key};
use nebula_expression::ExpressionEngine;
use nebula_resource::{
    Manager, ResidentConfig, ResourceContext,
    error::Error,
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use nebula_schema::{Field, HasSchema, Schema, ValidSchema, field_key};
use serde::Deserialize;
use serde_json::json;

// ── Test resource with a REAL (non-empty) typed schema ─────────────────────
//
// The schema declares exactly `{host, port}` and NO secret field. This is the
// closed set: any other top-level config key — including a secret-shaped
// `password` — is outside the typed `R::Config` surface and must be refused.

#[derive(Clone, Debug, Deserialize)]
#[allow(
    dead_code,
    reason = "fields exercised via the HasSchema closed-set guard + serde::Deserialize, not direct read"
)]
struct DbConfig {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
}

fn default_port() -> u16 {
    5432
}

impl HasSchema for DbConfig {
    fn schema() -> ValidSchema {
        Schema::builder()
            .add(Field::string(field_key!("host")).required())
            .add(Field::number(field_key!("port")).integer())
            .build()
            .expect("DbConfig schema is valid")
    }
}

impl ResourceConfig for DbConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            Err(Error::permanent("host must not be empty"))
        } else {
            Ok(())
        }
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.host.hash(&mut h);
        self.port.hash(&mut h);
        h.finish()
    }
}

// Custom error boilerplate removed — Resource lifecycle methods now return
// `crate::Error` directly (HasCredentialSlots redesign).

#[derive(Clone)]
struct Db;

impl Provider for Db {
    type Config = DbConfig;
    type Instance = Arc<()>;

    fn key() -> ResourceKey {
        resource_key!("secret-config-guard-db")
    }

    async fn create(&self, _config: &DbConfig, _ctx: &ResourceContext) -> Result<Arc<()>, Error> {
        Ok(Arc::new(()))
    }

    async fn destroy(&self, _runtime: Arc<()>) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for Db {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl Resident for Db {
    fn is_alive_sync(&self, _runtime: &Arc<()>) -> bool {
        true
    }
}

impl DeclaresDependencies for Db {
    fn dependencies() -> Dependencies {
        // No credential slots: the only legitimate way to wire a secret would
        // be a typed slot, and this resource declares none — so a secret in
        // `config` has no honest place to go and must be refused outright.
        Dependencies::new()
    }
}

fn topology() -> TopologyRuntime<Db> {
    TopologyRuntime::Resident(ResidentRuntime::<Db>::new(ResidentConfig::default()))
}

// ── Negative: secret-shaped field is rejected (the security assertion) ──────

/// A config that inlines a `password` field — not declared by `DbConfig`'s
/// typed schema — must be rejected with a typed error attributable to the
/// schema/closed-set surface. Critically the rejection must NOT echo the
/// secret value: a validation error that prints the inlined secret would
/// itself be a disclosure (redaction is part of the invariant, not optional).
#[tokio::test]
async fn register_from_value_rejects_inline_secret_field() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    let secret = "p@ss";
    let config_json = json!({
        "host": "h",
        "password": secret,
    });

    let err = manager
        .register_resolved::<Db>(
            config_json,
            &engine,
            HashMap::new(),
            Db,
            ScopeLevel::Global,
            topology(),
            Manager::erased_acquire_resident_for::<Db>(),
            None,
        )
        .await
        .expect_err("config carrying an inline secret-shaped field must be rejected");

    let msg = err.to_string();

    // (a) Rejection is attributable to the typed config surface and names the
    //     offending field so the operator learns they mis-wired a secret.
    assert!(
        msg.contains("password") && msg.contains("not declared"),
        "expected a closed-set rejection naming the undeclared `password` field, got: {msg}"
    );

    // (b) Redaction: the rejection error must NOT contain the secret value.
    //     An error that echoes the inlined secret would leak it.
    assert!(
        !msg.contains(secret),
        "rejection error leaked the secret value `{secret}`: {msg}"
    );

    // The resource must NOT have been registered.
    assert!(
        !manager.contains(&Db::key()),
        "a rejected register_from_value must not register the resource"
    );
}

// ── Positive control: clean config registers OK (proves specificity) ───────

/// The SAME resource with a config containing ONLY the schema's declared
/// fields registers successfully. This proves the rejection above is specific
/// to the extra secret-shaped field and not a blanket failure of the harness —
/// a redaction/abuse test that always fails is worthless.
#[tokio::test]
async fn register_from_value_accepts_clean_config_same_resource() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    let config_json = json!({
        "host": "h",
        "port": 5432,
    });

    manager
        .register_resolved::<Db>(
            config_json,
            &engine,
            HashMap::new(),
            Db,
            ScopeLevel::Global,
            topology(),
            Manager::erased_acquire_resident_for::<Db>(),
            None,
        )
        .await
        .expect("clean config with only declared fields must register");

    assert!(
        manager.contains(&Db::key()),
        "resource must be registered after a clean register_from_value"
    );
}
