//! Restart-durability of the webhook bootstrap READ path (ADR-0096, PR-3a).
//!
//! Proves that `bootstrap_webhook_activations` can reconstruct a webhook
//! handler after a process restart by reading `port_triggers.config.webhook_activation`
//! through `TriggerStoreSpecLookup`.
//!
//! # Acceptance criteria
//!
//! 1. Happy path: one active `port_webhook_activations` row + one matching
//!    `port_triggers` row with a valid `webhook_activation` spec → `loaded==1,
//!    skipped==0`.
//!
//! 2. Cross-tenant isolation (RED-on-revert): the same `trigger_id` seeded
//!    under `scope_a` is NOT visible to a `scope_b` activation row → the
//!    activation for `scope_b` is counted as `skipped==1` (MissingSpec).
//!    Removing the scope binding from `TriggerStoreSpecLookup` so it does an
//!    unscoped by-id query would cause this test to pass when it must fail,
//!    proving the cross-tenant guard is structural, not coincidental.
//!
//! 3. Serde backward-compat: the old `action_kind` field name (pre-rename)
//!    deserialises correctly via `#[serde(alias = "action_kind")]`.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::{TriggerRuntimeContext, webhook::providers::default_factories};
use nebula_api::transport::webhook::{
    SecretResolutionError, TriggerStoreSpecLookup, WebhookActivationContextFactory,
    WebhookSecretResolver, bootstrap_webhook_activations,
};
use nebula_engine::ActionRegistry;
use nebula_storage::inmem::{InMemoryTriggerStore, InMemoryWebhookActivationStore};
use nebula_storage::rows::WebhookActivationSpec;
use nebula_storage_port::{
    Scope,
    dto::{TriggerRow, WebhookActivationRecord},
    store::{TriggerStore, WebhookActivationStore},
};
use tokio_util::sync::CancellationToken;

// ── Constants ─────────────────────────────────────────────────────────────────

const TRIGGER_ID: &str = "trg_bootstrap_restart_001";
const SECRET_ID: &str = "cred_bootstrap_secret_001";
/// 32-byte HMAC test key. Not a real secret — used only as a fixture.
const HMAC_SECRET: &[u8] = b"test-bootstrap-hmac-secret-32by!";

// ── Scopes ────────────────────────────────────────────────────────────────────

fn scope_a() -> Scope {
    Scope::new("ws-restart-a", "org-restart-a")
}

fn scope_b() -> Scope {
    Scope::new("ws-restart-b", "org-restart-b")
}

// ── Test doubles ──────────────────────────────────────────────────────────────

/// Secret resolver that returns a fixed byte slice for one known `secret_id`
/// and an error for any other id.  No scope check — the test exercises scope
/// enforcement through `TriggerStoreSpecLookup`, not through this resolver.
struct FixedSecretResolver {
    secret_id: &'static str,
    secret: &'static [u8],
}

#[async_trait]
impl WebhookSecretResolver for FixedSecretResolver {
    async fn resolve(
        &self,
        _scope: &Scope,
        secret_id: &str,
    ) -> Result<Vec<u8>, SecretResolutionError> {
        if secret_id == self.secret_id {
            Ok(self.secret.to_vec())
        } else {
            Err(format!("FixedSecretResolver: unknown secret_id {secret_id:?}").into())
        }
    }
}

/// Minimal ctx-template factory — builds a placeholder context that carries
/// enough to satisfy the `WebhookActivationContextFactory` contract without
/// any real workflow state.
struct NoopCtxFactory;

impl WebhookActivationContextFactory for NoopCtxFactory {
    fn build(&self, _record: &WebhookActivationRecord) -> TriggerRuntimeContext {
        TriggerRuntimeContext::new(
            Arc::new(
                nebula_core::BaseContext::builder()
                    .cancellation(CancellationToken::new())
                    .build(),
            ),
            nebula_core::WorkflowId::new(),
            nebula_core::node_key!("bootstrap-restart-test"),
        )
    }
}

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn build_action_registry() -> ActionRegistry {
    let registry = ActionRegistry::new();
    for factory in default_factories() {
        registry.register_webhook_provider(factory);
    }
    registry
}

/// Seed a `port_triggers` row with `kind=webhook` and a `webhook_activation`
/// config namespace under `scope`. The config contains `provider="generic"` and
/// the test `SECRET_ID`.
async fn seed_trigger_row(store: &dyn TriggerStore, scope: &Scope) {
    let spec = WebhookActivationSpec::new("generic", SECRET_ID);
    let config = spec
        .write_into_trigger_config(serde_json::Value::Null)
        .expect(
            "WebhookActivationSpec::new always produces a serialisable value; \
             Null is a valid empty-object seed for write_into_trigger_config",
        );

    let row = TriggerRow {
        id: TRIGGER_ID.to_string(),
        workspace_id: scope.workspace_id.clone(),
        workflow_id: "wf-bootstrap-test-001".to_string(),
        slug: "bootstrap-restart-test".to_string(),
        display_name: "Bootstrap Restart Test".to_string(),
        kind: "webhook".to_string(),
        config,
        state: "active".to_string(),
        run_as: None,
        webhook_path: None,
        created_at: "2026-06-18T00:00:00Z".to_string(),
        created_by: "usr-test".to_string(),
        version: 1,
        deleted_at: None,
    };
    store
        .create(scope, row)
        .await
        .expect("TriggerRow creation must succeed on an empty InMemoryTriggerStore");
}

/// Seed an active `port_webhook_activations` row for `TRIGGER_ID` under `scope`.
async fn seed_activation_record(store: &dyn WebhookActivationStore, scope: &Scope) {
    let record = WebhookActivationRecord::new(TRIGGER_ID, scope.clone(), "bootstrap-slug", true);
    store
        .upsert(scope, record)
        .await
        .expect("WebhookActivationRecord upsert must succeed on an empty store");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Happy path: one active activation row + matching trigger config row → the
/// bootstrap reconstructs the handler and counts `loaded==1, skipped==0`.
///
/// This simulates a server restart: both rows exist (written at activation
/// time, PR-3b); the bootstrap reads them back and validates the factory chain.
#[tokio::test]
async fn bootstrap_reads_spec_from_trigger_store_after_restart() {
    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());

    seed_trigger_row(trigger_store.as_ref(), &scope_a()).await;
    seed_activation_record(activation_store.as_ref(), &scope_a()).await;

    let spec_lookup = TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as _);
    let registry = build_action_registry();
    let secrets = FixedSecretResolver {
        secret_id: SECRET_ID,
        secret: HMAC_SECRET,
    };

    let report = bootstrap_webhook_activations(
        activation_store.as_ref(),
        &registry,
        &secrets,
        &NoopCtxFactory,
        &spec_lookup,
        None,
    )
    .await
    .expect("bootstrap must not return Err — storage errors are the only fatal path");

    assert_eq!(
        report.loaded, 1,
        "one seeded activation must be loaded; {report:?}"
    );
    assert_eq!(
        report.skipped, 0,
        "no activations must be skipped; {report:?}"
    );
}

/// Cross-tenant isolation — RED-on-revert guard.
///
/// The `port_triggers` row belongs to `scope_a`; the `port_webhook_activations`
/// row claims `scope_b` (different tenant, same `trigger_id`).
/// `TriggerStoreSpecLookup::lookup` must return `Ok(None)` because
/// `ScopedTriggerStore` partitions by `(workspace_id, org_id)`.
///
/// Expected: `loaded==0, skipped==1` (BootstrapError::MissingSpec).
///
/// **RED-on-revert invariant**: if `TriggerStoreSpecLookup::lookup` were
/// changed to query the store without a scope binding, it would find the
/// `scope_a` row for `scope_b`'s activation and return `Some(spec)`. That
/// would flip the assertion (`loaded==1`) and make this test pass when the
/// cross-tenant boundary is broken — confirming the guard is structural.
#[tokio::test]
async fn bootstrap_cross_tenant_trigger_row_is_invisible_to_other_scope() {
    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());

    // Trigger config row exists under scope_a only.
    seed_trigger_row(trigger_store.as_ref(), &scope_a()).await;
    // Activation record claims scope_b — a different tenant.
    seed_activation_record(activation_store.as_ref(), &scope_b()).await;

    let spec_lookup = TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as _);
    let registry = build_action_registry();
    let secrets = FixedSecretResolver {
        secret_id: SECRET_ID,
        secret: HMAC_SECRET,
    };

    let report = bootstrap_webhook_activations(
        activation_store.as_ref(),
        &registry,
        &secrets,
        &NoopCtxFactory,
        &spec_lookup,
        None,
    )
    .await
    .expect("bootstrap must not return Err — storage errors are the only fatal path");

    assert_eq!(
        report.skipped, 1,
        "cross-tenant activation must be skipped (MissingSpec); {report:?}"
    );
    assert_eq!(
        report.loaded, 0,
        "a cross-tenant activation must never count as loaded; {report:?}"
    );
}

/// Serde backward-compat: rows serialised under the old field name `action_kind`
/// (before ADR-0101 renamed it to `provider`) must deserialise correctly via
/// `#[serde(alias = "action_kind")]`.
///
/// This is a pure decode test — no store, no bootstrap. It pins the serde alias
/// contract that protects any existing rows written before the rename.
#[test]
fn webhook_activation_spec_old_field_name_reads_via_alias() {
    let old_json = serde_json::json!({
        "webhook_activation": {
            "action_kind": "generic",
            "secret_id": "cred_legacy_001"
        }
    });
    let spec = WebhookActivationSpec::from_trigger_config(&old_json)
        .expect(
            "decode must succeed — `action_kind` is a registered serde alias for `provider`; \
             a failure here means the alias was accidentally removed",
        )
        .expect(
            "spec must be present — `webhook_activation` key exists in the test JSON; \
             Ok(None) means the namespace key lookup failed",
        );

    assert_eq!(
        spec.provider, "generic",
        "alias `action_kind` must round-trip into the `provider` field"
    );
    assert_eq!(
        spec.secret_id, "cred_legacy_001",
        "secret_id must survive the old-format decode unchanged"
    );
}
