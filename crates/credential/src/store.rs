//! v2 credential store trait with layered composition.
//!
//! Provides a CRUD abstraction for credential persistence with optimistic
//! concurrency control via [`PutMode::CompareAndSwap`]. Encryption is handled
//! by the `EncryptionLayer` wrapper (in `nebula-storage`), not by store
//! implementations themselves.

use std::future::Future;

use serde_json::Value;

/// How to handle conflicts on put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PutMode {
    /// Fail if credential already exists.
    CreateOnly,
    /// Overwrite unconditionally.
    Overwrite,
    /// Compare-and-swap: only succeed if current version matches.
    CompareAndSwap {
        /// The version the caller last observed.
        expected_version: u64,
    },
}

/// Resolves the current caller's scope for credential access control.
///
/// The contract-level abstraction the multi-tenant credential scoping
/// **policy** (`nebula_tenancy::CredentialScopeLayer`) keys on: it
/// supplies the per-request owner identity that filters every
/// [`CredentialStore`] operation. The trait lives here in the credential
/// contract crate (not in the tenancy policy crate) so downward
/// consumers — the credential runtime, builtins — can name it without an
/// upward `→ nebula-tenancy` dependency (spec §3 data-vs-policy split:
/// the abstraction goes down, the concrete resolver + scoping layer stay
/// in `nebula-tenancy`).
///
/// Implementations typically extract the owner identity from a
/// request-scoped context (e.g. JWT claims, session state).
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::ScopeResolver;
///
/// struct StaticScope(Option<String>);
///
/// impl ScopeResolver for StaticScope {
///     fn current_owner(&self) -> Option<&str> {
///         self.0.as_deref()
///     }
/// }
/// ```
pub trait ScopeResolver: Send + Sync {
    /// Returns the owner ID for the current request context.
    ///
    /// Returns `None` for admin / global access, which bypasses
    /// all scope checks.
    fn current_owner(&self) -> Option<&str>;
}

/// Reserved `StoredCredential.metadata` key under which the owning tenant's
/// `owner_id` is stamped. Single source of truth for both the management facade
/// (which stamps it on write and gates reads) and the runtime resolver (which
/// re-verifies it at load on the slot path) — two literals would be a drift
/// hazard for a security-critical comparison.
pub(crate) const OWNER_ID_METADATA_KEY: &str = "owner_id";

/// Reserved `StoredCredential.metadata` key holding the revoke tombstone epoch
/// (an RFC 3339 timestamp). Its **presence** marks the credential terminally
/// retired: `revoke` stamps it (and zeroizes the secret bytes) instead of
/// deleting the row, so a workflow `slot_binding` that still points at the id
/// gets a clear typed rejection rather than a bare `NotFound`, and the id stays
/// occupied so a revoked credential cannot be resurrected under the same id.
///
/// Read fail-closed: a row carrying this key is tombstoned regardless of whether
/// the value parses as a timestamp (see [`StoredCredential::is_tombstoned`]).
pub(crate) const REVOKED_AT_METADATA_KEY: &str = "revoked_at";

/// Reserved `StoredCredential.metadata` key holding the last time the material
/// was actually validated against its provider — set on a provider-contacting
/// write (create, refresh-success, re-resolve), an RFC 3339 timestamp. This is
/// distinct from `updated_at`, which a *display-only* edit (rename / tag /
/// description) bumps without re-contacting the provider. The mandatory
/// re-validation floor ([`CredentialPolicy::decide_refresh`](crate::CredentialPolicy::decide_refresh))
/// must measure from this, not `updated_at`, or a cosmetic edit would
/// indefinitely postpone the floor. Absent on legacy rows — readers fall back to
/// `created_at` (a real validation time; fail-safe toward re-validating).
pub(crate) const LAST_VALIDATED_AT_METADATA_KEY: &str = "last_validated_at";

/// An owner-scoped credential lookup key: a credential id paired with the
/// `owner_id` that a prior tenant-scope check proved owns it.
///
/// The constructor is crate-private and the only in-crate caller is
/// [`ValidatedCredentialBinding::owner_scoped_key`](crate::service::binding::ValidatedCredentialBinding),
/// whose own constructor is reachable only through
/// `CredentialService::validate_credential_binding` (the owner gate). A caller
/// therefore cannot *express* an unscoped (cross-tenant) load on the slot
/// resolution path: the resolver re-checks the loaded row's stamped `owner_id`
/// against this key and fails closed (existence-hiding `NotFound`) on a
/// mismatch, so binding provenance is backed by a load-time owner check rather
/// than trusted on its own.
#[derive(Debug, Clone)]
pub struct OwnerScopedKey {
    owner_id: String,
    credential_id: String,
}

impl OwnerScopedKey {
    /// Crate-private constructor — obtainable only from a
    /// `ValidatedCredentialBinding`.
    pub(crate) fn new(owner_id: String, credential_id: String) -> Self {
        Self {
            owner_id,
            credential_id,
        }
    }

    /// The owning tenant's `owner_id`.
    #[must_use]
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// The credential's string identifier.
    #[must_use]
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
}

/// A stored credential with metadata.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    /// The credential ID.
    pub id: String,
    /// User-facing credential name (n8n-style "My Google Account"). `None` for
    /// system / unnamed credentials. When `Some`, unique per owner.
    pub name: Option<String>,
    /// The credential type key (`Credential::KEY`), identifying which
    /// `Credential` implementation produced this stored state.
    pub credential_key: String,
    /// Serialized credential state (encrypted at the `EncryptionLayer` boundary).
    pub data: Vec<u8>,
    /// State type identifier (`CredentialState::KIND`).
    pub state_kind: String,
    /// Schema version (`CredentialState::VERSION`).
    pub state_version: u32,
    /// Monotonic version counter (for CAS).
    pub version: u64,
    /// When this credential was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this credential was last modified.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Optional expiration time.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the credential requires interactive re-authentication.
    ///
    /// Set to `true` when a refresh attempt returns
    /// [`crate::resolve::RefreshOutcome::ReauthRequired`] (provider rejected the
    /// refresh token, e.g. OAuth2 `invalid_grant`, or sentinel-threshold
    /// escalation per credential refresh sub-spec §3.4 / §3.6). Cleared
    /// (`false`) on a successful `Refreshed` outcome.
    ///
    /// Cross-replica readers (e.g. the L2 post-backoff state-recheck
    /// predicate) consult this flag to short-circuit refresh attempts that
    /// would otherwise produce a duplicate IdP rejection — preventing
    /// `O(replicas)` IdP load on a credential that has already been
    /// rejected.
    ///
    /// Persistence: backends store this either as a dedicated column or as
    /// a key in the metadata blob. Backend row structs that do use serde
    /// should annotate this field with `#[serde(default)]` so older rows
    /// missing the field deserialize as `false`. No dedicated SQL column
    /// is required on `StoredCredential` itself (which has no serde
    /// derives).
    pub reauth_required: bool,
    /// Arbitrary metadata.
    pub metadata: serde_json::Map<String, Value>,
}

impl StoredCredential {
    /// Whether this credential has been revoked (carries a tombstone epoch).
    ///
    /// Fail-closed: a row is tombstoned iff the `revoked_at` tombstone-epoch
    /// metadata key is present, irrespective of whether its value parses as a
    /// timestamp — a malformed epoch must never read back as "still live".
    #[must_use]
    pub fn is_tombstoned(&self) -> bool {
        self.metadata.contains_key(REVOKED_AT_METADATA_KEY)
    }

    /// The revoke tombstone epoch, when present and well-formed.
    ///
    /// Returns `None` both for a live credential and for a tombstoned row whose
    /// stamp is unparseable; use [`is_tombstoned`](Self::is_tombstoned) for the
    /// authoritative liveness check and this only for observability/reporting.
    #[must_use]
    pub fn revoked_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata
            .get(REVOKED_AT_METADATA_KEY)
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
    }

    /// When the material was last validated against its provider — set on a
    /// provider-contacting write (create / refresh-success / re-resolve) via the
    /// crate-private `last_validated_at` metadata key. Distinct from `updated_at`,
    /// which a display-only edit bumps. `None` on legacy rows that predate the
    /// stamp (or when the value is unparseable); the re-validation floor falls
    /// back to `created_at` in that case (a real validation time, fail-safe toward
    /// re-validating). Use [`Self::last_validated_or_created`] for that fallback.
    #[must_use]
    pub fn last_validated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata
            .get(LAST_VALIDATED_AT_METADATA_KEY)
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
    }

    /// The last-validated time, falling back to `created_at` when no validation
    /// stamp is present (legacy row). This is the timestamp the mandatory
    /// re-validation floor measures from — never `updated_at`, which a
    /// display-only edit (rename / tag) bumps without re-contacting the provider.
    #[must_use]
    pub fn last_validated_or_created(&self) -> chrono::DateTime<chrono::Utc> {
        self.last_validated_at().unwrap_or(self.created_at)
    }

    /// Record that the material was just validated against its provider, at
    /// `at`. Call on a provider-contacting write (create / refresh-success /
    /// OAuth token exchange / re-resolve) so the mandatory re-validation floor
    /// measures from here. Do **not** call on a display-only edit (rename / tag)
    /// — that is exactly the bump this stamp exists to distinguish from.
    pub fn stamp_validated(&mut self, at: chrono::DateTime<chrono::Utc>) {
        self.metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(at.to_rfc3339()),
        );
    }
}

/// Error from store operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// Credential not found.
    #[error("credential not found: {id}")]
    NotFound {
        /// The ID that was looked up.
        id: String,
    },
    /// Version conflict on CAS put.
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// The credential ID.
        id: String,
        /// The version the caller expected.
        expected: u64,
        /// The version actually in the store.
        actual: u64,
    },
    /// Credential already exists (`CreateOnly` mode).
    #[error("credential already exists: {id}")]
    AlreadyExists {
        /// The credential ID.
        id: String,
    },
    /// Audit sink refused to record the operation. Fail-closed alarm.
    ///
    /// Per credential invariants invariant 4 + §14 "no discard-and-log": a failed
    /// audit sink surfaces as a hard error rather than a log-and-continue.
    /// The underlying store state depends on the operation and rollback
    /// feasibility:
    ///
    /// - `put(PutMode::CreateOnly)` — `AuditLayer` attempts a best-effort `delete` of the
    ///   freshly-inserted record before returning. On a clean rollback path, the write did not
    ///   become externally visible.
    /// - `put(PutMode::Overwrite | PutMode::CompareAndSwap)` / `delete` — no rollback. The mutation
    ///   may already be visible to concurrent readers; this error is a **fail-closed alarm**
    ///   signalling that the audit trail is compromised, not a guarantee that the mutation did not
    ///   commit.
    /// - `get` / `list` / `exists` — read path; no mutation to roll back.
    ///
    /// Consumers should treat this error as actionable (investigate the
    /// audit sink; retry only after the sink is healthy).
    #[error("audit sink refused: {0}")]
    AuditFailure(String),
    /// Backend error.
    #[error("store backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

/// Core CRUD trait for credential persistence.
///
/// Implementations handle raw bytes — encryption/decryption is done
/// by the `EncryptionLayer` wrapper (in `nebula-storage`), not by the
/// store itself.
pub trait CredentialStore: Send + Sync {
    /// Retrieve a stored credential by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if no credential with the given ID exists.
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn get(&self, id: &str) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Store or update a credential.
    ///
    /// The returned [`StoredCredential`] has its `version`, `created_at`,
    /// and `updated_at` fields set by the store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AlreadyExists`] when `mode` is
    /// [`PutMode::CreateOnly`] and the ID already exists.
    ///
    /// Returns [`StoreError::VersionConflict`] when `mode` is
    /// [`PutMode::CompareAndSwap`] and the stored version differs.
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Delete a credential by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if no credential with the given ID exists.
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn delete(&self, id: &str) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// List credential IDs, optionally filtered by `state_kind`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn list(
        &self,
        state_kind: Option<&str>,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// Check if a credential exists.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn exists(&self, id: &str) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_with(metadata: serde_json::Map<String, Value>) -> StoredCredential {
        StoredCredential {
            id: "cred_x".to_owned(),
            name: None,
            credential_key: "github_oauth".to_owned(),
            data: vec![1, 2, 3],
            state_kind: "oauth2_state".to_owned(),
            state_version: 1,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata,
        }
    }

    #[test]
    fn live_row_is_not_tombstoned() {
        let row = row_with(serde_json::Map::new());
        assert!(!row.is_tombstoned());
        assert!(row.revoked_at().is_none());
    }

    #[test]
    fn well_formed_epoch_parses() {
        let mut meta = serde_json::Map::new();
        meta.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            Value::String("2026-06-13T10:00:00Z".to_owned()),
        );
        let row = row_with(meta);
        assert!(row.is_tombstoned());
        assert!(row.revoked_at().is_some());
    }

    #[test]
    fn malformed_epoch_still_reads_as_tombstoned() {
        // Fail-closed: a present-but-unparseable stamp must not read back as
        // live. `is_tombstoned` is the authoritative liveness check; the
        // unparseable epoch only costs the timestamp for reporting.
        let mut meta = serde_json::Map::new();
        meta.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            Value::String("not-a-timestamp".to_owned()),
        );
        let row = row_with(meta);
        assert!(row.is_tombstoned());
        assert!(row.revoked_at().is_none());
    }

    #[test]
    fn last_validated_falls_back_to_created_when_absent() {
        // Legacy row with no validation stamp → the floor measures from
        // `created_at`, never `updated_at`.
        let row = row_with(serde_json::Map::new());
        assert!(row.last_validated_at().is_none());
        assert_eq!(row.last_validated_or_created(), row.created_at);
    }

    #[test]
    fn display_edit_does_not_postpone_the_validation_time() {
        // The floor regression: a display-only edit bumps `updated_at` to now but
        // must NOT move the validation time. With a stamp far in the past and a
        // fresh `updated_at`, `last_validated_or_created` returns the OLD stamp,
        // so the mandatory re-validation floor still fires.
        let validated = chrono::Utc::now() - chrono::Duration::days(30);
        let mut meta = serde_json::Map::new();
        meta.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(validated.to_rfc3339()),
        );
        let mut row = row_with(meta);
        row.updated_at = chrono::Utc::now(); // simulate a rename/tag edit

        let resolved = row.last_validated_or_created();
        assert!(
            resolved < row.updated_at,
            "validation time must not follow a display-only updated_at bump"
        );
        // Round-trips to the stamped instant (within sub-second rfc3339 precision).
        assert!((resolved - validated).num_seconds().abs() <= 1);
    }
}
