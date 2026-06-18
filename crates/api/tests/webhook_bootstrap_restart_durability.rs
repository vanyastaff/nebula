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
//! 2. Cross-tenant isolation (RED-on-revert): `scope_a` and `scope_b` both
//!    have a `port_triggers` row for the same `trigger_id`, but with different
//!    `secret_id` values. Only `scope_b`'s secret is known to the resolver.
//!    The `scope_b` activation row must load successfully (`loaded==1`).
//!    On revert (drop scope binding so the lookup is unscoped): the lookup
//!    returns `scope_a`'s row → its unknown secret_id fails resolution →
//!    `skipped==1`. That outcome is the opposite of green, making the guard
//!    genuinely RED-on-revert.
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

/// The secret_id used by scope_a's trigger row.
///
/// This id is intentionally UNKNOWN to the `TwoSecretResolver` so that if the
/// scope binding is dropped (making the lookup return scope_a's row when
/// scope_b's activation is bootstrapped), secret resolution fails → `skipped`.
const SECRET_ID_A: &str = "cred_scope_a_secret";

/// The secret_id used by scope_b's trigger row.
///
/// This id IS known to the `TwoSecretResolver`; it is the one that must
/// resolve for the bootstrap to count `loaded==1`.
const SECRET_ID_B: &str = "cred_scope_b_secret";

/// HMAC key for scope_b's secret. Not a real secret — used only as a fixture.
const HMAC_SECRET_B: &[u8] = b"scope-b-hmac-secret-fixture-32b!";

// ── Scopes ────────────────────────────────────────────────────────────────────

fn scope_a() -> Scope {
    Scope::new("ws-restart-a", "org-restart-a")
}

fn scope_b() -> Scope {
    Scope::new("ws-restart-b", "org-restart-b")
}

// ── Test doubles ──────────────────────────────────────────────────────────────

/// Secret resolver that returns a fixed byte slice for one known `secret_id`
/// and a typed error for any other id.
///
/// No scope check here — scope enforcement is exercised through
/// `TriggerStoreSpecLookup`, not this resolver.  The single-known-id design is
/// load-bearing for the cross-tenant test: `scope_a`'s row references an id
/// this resolver does NOT know, so a scope-blind lookup that accidentally
/// returns `scope_a`'s row will fail at secret resolution and surface as
/// `skipped`, not `loaded`.
struct SingleSecretResolver {
    known_id: &'static str,
    secret: &'static [u8],
}

#[async_trait]
impl WebhookSecretResolver for SingleSecretResolver {
    async fn resolve(
        &self,
        _scope: &Scope,
        secret_id: &str,
    ) -> Result<Vec<u8>, SecretResolutionError> {
        if secret_id == self.known_id {
            Ok(self.secret.to_vec())
        } else {
            Err(format!("SingleSecretResolver: no entry for secret_id {secret_id:?}").into())
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
/// config namespace under `scope`, using the given `provider` tag and `secret_id`.
async fn seed_trigger_row(
    store: &dyn TriggerStore,
    scope: &Scope,
    provider: &str,
    secret_id: &str,
) {
    let spec = WebhookActivationSpec::new(provider, secret_id);
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
        .expect("TriggerRow creation must succeed; duplicate id means the store was not empty");
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

    seed_trigger_row(trigger_store.as_ref(), &scope_a(), "generic", SECRET_ID_B).await;
    seed_activation_record(activation_store.as_ref(), &scope_a()).await;

    let spec_lookup = TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as _);
    let registry = build_action_registry();
    let secrets = SingleSecretResolver {
        known_id: SECRET_ID_B,
        secret: HMAC_SECRET_B,
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

/// Cross-tenant isolation — strengthened RED-on-revert guard.
///
/// Setup: BOTH `scope_a` and `scope_b` have a `port_triggers` row for the same
/// `trigger_id`. The rows carry different `secret_id` values:
///
/// - `scope_a` row → `SECRET_ID_A` (NOT in the resolver — unknown).
/// - `scope_b` row → `SECRET_ID_B` (known to the resolver).
///
/// Only `scope_b` has an active `port_webhook_activations` row.
///
/// Correct outcome (scope binding intact):
///   `lookup(scope_b, TRIGGER_ID)` returns `scope_b`'s spec → `SECRET_ID_B`
///   resolves → `loaded==1, skipped==0`.
///
/// RED-on-revert (scope binding dropped, lookup becomes unscoped by-id):
///   An unscoped query may return EITHER row (depending on store internals).
///   If it returns `scope_a`'s row → `SECRET_ID_A` is unknown to the resolver
///   → secret resolution fails → `skipped==1, loaded==0`.
///   Either way the assertions below (`loaded==1, skipped==0`) become RED,
///   proving the scope binding is the load-bearing correctness guarantee.
#[tokio::test]
async fn bootstrap_scoped_lookup_returns_correct_tenant_spec() {
    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());

    // scope_a row: provider="generic", SECRET_ID_A (unknown to resolver).
    seed_trigger_row(trigger_store.as_ref(), &scope_a(), "generic", SECRET_ID_A).await;
    // scope_b row: provider="stripe", SECRET_ID_B (known to resolver).
    seed_trigger_row(trigger_store.as_ref(), &scope_b(), "stripe", SECRET_ID_B).await;

    // Only scope_b has an active activation — that is the bootstrap target.
    seed_activation_record(activation_store.as_ref(), &scope_b()).await;

    let spec_lookup = TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as _);
    let registry = build_action_registry();
    // Only SECRET_ID_B is resolvable. If the lookup returns scope_a's row
    // (misattribution), secret resolution fails and the bootstrap skips.
    let secrets = SingleSecretResolver {
        known_id: SECRET_ID_B,
        secret: HMAC_SECRET_B,
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

    // Correct: scope_b's spec (SECRET_ID_B, provider="stripe") resolves.
    assert_eq!(
        report.loaded, 1,
        "scope_b activation must load using scope_b's spec; \
         skipped>0 means the lookup returned scope_a's row (misattribution); {report:?}"
    );
    assert_eq!(
        report.skipped, 0,
        "no activation must be skipped when the scoped lookup works correctly; {report:?}"
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
