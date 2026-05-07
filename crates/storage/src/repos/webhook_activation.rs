//! Webhook-activation repository (M3.3 / ADR-0049).
//!
//! Storage seam for the API's slug-routed webhook bootstrap. Sibling
//! to the generic [`crate::repos::TriggerRepo`] (whose contract is
//! deliberately kind-agnostic). Webhook-specific operations live here
//! to keep `TriggerRepo` from carrying provider concerns it never
//! cares about.
//!
//! # Why a sibling repo, not a method on `TriggerRepo`
//!
//! 1. Decoding [`WebhookActivationSpec`] is webhook-specific — adding
//!    `decode_webhook_spec(...)` to a kind-agnostic trait pulls
//!    `nebula-credential` resolution into every cron / event consumer
//!    that has no business with it.
//! 2. The bootstrap query joins `orgs ↔ workspaces ↔ triggers` and
//!    filters by `kind = 'webhook'` — the projection shape (slug
//!    triple + decoded spec) does not match `TriggerRepo`'s
//!    row-centric contract.
//! 3. Future webhook operations (`mark_seen_path`, audit pulls) land
//!    here without re-shuffling the trait surface.
//!
//! Two backings:
//!
//! - [`InMemoryWebhookActivationRepo`] — process-local, used by the
//!   API's transport tests and the `test_support` harness.
//! - [`crate::pg::PgWebhookActivationRepo`] (behind
//!   `feature = "postgres"`) — runs the canonical join.
//!
//! SQLite parity is tracked alongside the broader SQLite repo
//! adoption. Until the SQLite repo backend lands, the dev path uses
//! the in-memory backing.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::error::StorageError;
#[cfg(any(feature = "postgres", test))]
use crate::rows::WebhookActivationSpec;
#[cfg(feature = "postgres")]
use crate::rows::WebhookActivationSpecError;
use crate::rows::{WebhookActivationCoords, WebhookActivationRecord};

/// Storage-layer port for the webhook-bootstrap pathway.
///
/// Two production callers consume this trait:
///
/// 1. **Startup bootstrap** — `build_app` calls
///    [`Self::list_active`] once, hands each
///    [`WebhookActivationRecord`] to the registered factory, and
///    registers the resulting handler in the transport's slug map.
/// 2. **Admin reload** — `POST /internal/v1/webhooks/reload` re-runs
///    [`Self::list_active`] and atomically swaps the slug map.
///
/// `find_by_webhook_path` is the cache-miss fall-through: when the
/// transport receives a request for a slug that is not in its
/// in-memory map (post-create race, pre-reload), it consults storage
/// directly. Default implementations are not provided so backends
/// answer with their fastest available query path.
///
/// `#[async_trait]` is used (not RPITIT / `impl Future`) so this
/// trait is **dyn-compatible** — the API layer threads a
/// `&dyn WebhookActivationRepo` through bootstrap and reload
/// pathways without monomorphising every call site.
#[async_trait]
pub trait WebhookActivationRepo: Send + Sync {
    /// List every `(kind = 'webhook', state = 'active')` trigger that
    /// carries a non-null `webhook_path`, decoded into the runtime
    /// shape consumed by the bootstrap pathway.
    ///
    /// Records with malformed `webhook_activation` JSONB are skipped
    /// and logged at `warn` level — one bad row must not block the
    /// rest of the bootstrap. Implementations include the failing
    /// trigger's primary key in the log so an operator can find and
    /// repair the row.
    ///
    /// # Errors
    ///
    /// Returns a [`StorageError`] only when the underlying query
    /// fails (connection error, schema drift). Decode errors are
    /// surfaced as warnings inside the result, not as `Err`.
    async fn list_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError>;

    /// Fetch a single activation by its `(org_slug, ws_slug, trigger_slug)`
    /// composite path.
    ///
    /// Resolves through the indexed `webhook_path` column (migration
    /// 0018) for `O(1)` lookup. Returns `Ok(None)` for a true miss.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on transport/connection failure or
    /// when the matched row's `webhook_activation` JSON fails to
    /// decode (a hard `Err` here — the caller asked for this exact
    /// row).
    async fn find_by_webhook_path(
        &self,
        path: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError>;
}

/// Translate a [`WebhookActivationSpecError`] into a
/// [`StorageError::Serialization`]. Repo impls funnel decode failures
/// through here so callers see one error type regardless of backend.
#[cfg(feature = "postgres")]
pub(crate) fn decode_err(trigger_id: &[u8], err: WebhookActivationSpecError) -> StorageError {
    StorageError::Serialization(format!(
        "webhook_activation decode failed (trigger_id={}): {err}",
        hex_short(trigger_id),
    ))
}

#[cfg(feature = "postgres")]
fn hex_short(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// Process-local in-memory backing.
///
/// Drives the `nebula-api` transport tests and any composition root
/// that wires storage with the in-memory `ExecutionRepo`. Records
/// are stored verbatim — no decoding is performed because they are
/// already typed.
#[derive(Default)]
pub struct InMemoryWebhookActivationRepo {
    inner: Arc<RwLock<Vec<WebhookActivationRecord>>>,
}

impl InMemoryWebhookActivationRepo {
    /// Construct an empty repo.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a repo seeded with the given activations.
    #[must_use]
    pub fn with_records(records: Vec<WebhookActivationRecord>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(records)),
        }
    }

    /// Replace every record with a new set. Used by tests that want
    /// to mimic `replace_slug_map`-style admin reloads.
    pub fn replace(&self, records: Vec<WebhookActivationRecord>) {
        *self.inner.write() = records;
    }

    /// Insert or update an activation in place. Idempotent on
    /// `(coords.org_slug, coords.workspace_slug, coords.trigger_slug)`.
    pub fn upsert(&self, record: WebhookActivationRecord) {
        let mut guard = self.inner.write();
        if let Some(slot) = guard.iter_mut().find(|r| r.coords == record.coords) {
            *slot = record;
        } else {
            guard.push(record);
        }
    }

    /// Remove a record by its slug coordinates. Returns `true` when a
    /// matching record was removed.
    pub fn remove(&self, coords: &WebhookActivationCoords) -> bool {
        let mut guard = self.inner.write();
        let before = guard.len();
        guard.retain(|r| &r.coords != coords);
        guard.len() != before
    }

    /// Live count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Whether the repo is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

impl std::fmt::Debug for InMemoryWebhookActivationRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryWebhookActivationRepo")
            .field("len", &self.len())
            .finish()
    }
}

#[async_trait]
impl WebhookActivationRepo for InMemoryWebhookActivationRepo {
    async fn list_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        Ok(self.inner.read().clone())
    }

    async fn find_by_webhook_path(
        &self,
        path: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        let snapshot = self.inner.read().clone();
        Ok(snapshot.into_iter().find(|record| {
            let composed = format!(
                "{}/{}/{}",
                record.coords.org_slug, record.coords.workspace_slug, record.coords.trigger_slug,
            );
            composed == path
        }))
    }
}

/// Build a record from raw row data — handy for tests and for the PG
/// impl after it has run the canonical join.
#[cfg(any(feature = "postgres", test))]
pub(crate) fn record_from_parts(
    trigger_id: Vec<u8>,
    org_slug: String,
    workspace_slug: String,
    trigger_slug: String,
    spec: WebhookActivationSpec,
) -> WebhookActivationRecord {
    WebhookActivationRecord {
        trigger_id,
        coords: WebhookActivationCoords {
            org_slug,
            workspace_slug,
            trigger_slug,
        },
        spec,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(trigger_id: u8, slug: &str) -> WebhookActivationRecord {
        record_from_parts(
            vec![trigger_id; 16],
            "acme".into(),
            "ops".into(),
            slug.into(),
            WebhookActivationSpec::new("generic", "cred_x"),
        )
    }

    #[tokio::test]
    async fn list_active_round_trips_seeded_records() {
        let repo = InMemoryWebhookActivationRepo::with_records(vec![rec(1, "stripe-prod")]);
        let listed = repo.list_active().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].coords.trigger_slug, "stripe-prod");
    }

    #[tokio::test]
    async fn upsert_replaces_existing_coords() {
        let repo = InMemoryWebhookActivationRepo::new();
        repo.upsert(rec(1, "x"));
        repo.upsert(rec(2, "x"));
        let listed = repo.list_active().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].trigger_id, vec![2u8; 16]);
    }

    #[tokio::test]
    async fn remove_returns_false_for_unknown() {
        let repo = InMemoryWebhookActivationRepo::new();
        let coords = WebhookActivationCoords {
            org_slug: "acme".into(),
            workspace_slug: "ops".into(),
            trigger_slug: "missing".into(),
        };
        assert!(!repo.remove(&coords));
    }

    #[tokio::test]
    async fn find_by_webhook_path_round_trip() {
        let repo = InMemoryWebhookActivationRepo::with_records(vec![rec(1, "stripe-prod")]);
        let hit = repo
            .find_by_webhook_path("acme/ops/stripe-prod")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(hit.coords.trigger_slug, "stripe-prod");
        let miss = repo.find_by_webhook_path("missing").await.unwrap();
        assert!(miss.is_none());
    }
}
