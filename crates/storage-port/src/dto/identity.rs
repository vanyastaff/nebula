//! Identity-zoo row DTOs.
//!
//! Each struct mirrors the column set of the corresponding structured
//! Postgres migration (`crates/storage/migrations/postgres/000X_*.sql`) 1:1.
//! Byte-id columns surface as opaque `String` (the typed-id encode/decode
//! happens at the adapter edge); JSONB columns surface as
//! `serde_json::Value`; timestamps surface as RFC 3339 strings.
use serde::{Deserialize, Serialize};

/// `users` row (migration 0001).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserRow {
    /// `usr_` ULID (opaque string form).
    pub id: String,
    /// Login email.
    pub email: String,
    /// Email-verification timestamp, if verified.
    pub email_verified_at: Option<String>,
    /// Display name.
    pub display_name: String,
    /// Avatar URL.
    pub avatar_url: Option<String>,
    /// Argon2id-encoded password hash (`None` for OAuth-only accounts).
    pub password_hash: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last successful-login timestamp.
    pub last_login_at: Option<String>,
    /// Lockout-until timestamp.
    pub locked_until: Option<String>,
    /// Consecutive failed-login count.
    pub failed_login_count: i32,
    /// Whether MFA is enabled.
    pub mfa_enabled: bool,
    /// Encrypted MFA secret.
    pub mfa_secret: Option<Vec<u8>>,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<String>,
}

/// `orgs` row (migration 0003).
// guard-justified: `settings` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrgRow {
    /// `org_` ULID (opaque string form).
    pub id: String,
    /// Org slug.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Creation timestamp.
    pub created_at: String,
    /// First user (opaque string form; no FK to preserve history).
    pub created_by: String,
    /// Plan tier.
    pub plan: String,
    /// Billing email.
    pub billing_email: Option<String>,
    /// Org settings blob.
    pub settings: serde_json::Value,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<String>,
}

/// `workspaces` row (migration 0004).
// guard-justified: `settings` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRow {
    /// `ws_` ULID (opaque string form).
    pub id: String,
    /// Owning org id (opaque string form).
    pub org_id: String,
    /// Workspace slug.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Description.
    pub description: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Creator id (opaque string form).
    pub created_by: String,
    /// Whether this is the org's default workspace.
    pub is_default: bool,
    /// Workspace settings blob.
    pub settings: serde_json::Value,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<String>,
}

/// Which membership table / scope domain a [`MembershipRow`] belongs to.
///
/// Stored verbatim as the `scope_kind` text column (`"org"` /
/// `"workspace"`). Modelled as a closed enum so an authorization domain
/// can never be a free-form string — an unknown value fails closed at the
/// adapter edge rather than silently widening access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeKind {
    /// Org-level membership (`org_members`).
    Org,
    /// Workspace-level membership (`workspace_members`).
    Workspace,
}

impl ScopeKind {
    /// Stable text form stored in the backend `scope_kind` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Org => "org",
            Self::Workspace => "workspace",
        }
    }

    /// Parse the backend `scope_kind` text. An unrecognized value is
    /// rejected (fail-closed: never coerce an unknown authz domain).
    ///
    /// # Errors
    /// Returns the offending string when it is neither `"org"` nor
    /// `"workspace"`.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "org" => Ok(Self::Org),
            "workspace" => Ok(Self::Workspace),
            other => Err(other.to_string()),
        }
    }
}

/// Which kind of principal holds a [`MembershipRow`].
///
/// Stored verbatim as the `principal_kind` text column (`"user"` /
/// `"service_account"`). Closed enum for the same fail-closed reason as
/// [`ScopeKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalKind {
    /// A human user.
    User,
    /// A non-human service account.
    ServiceAccount,
}

impl PrincipalKind {
    /// Stable text form stored in the backend `principal_kind` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ServiceAccount => "service_account",
        }
    }

    /// Parse the backend `principal_kind` text. An unrecognized value is
    /// rejected (fail-closed).
    ///
    /// # Errors
    /// Returns the offending string when it is neither `"user"` nor
    /// `"service_account"`.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "user" => Ok(Self::User),
            "service_account" => Ok(Self::ServiceAccount),
            other => Err(other.to_string()),
        }
    }
}

/// `org_members` / `workspace_members` row (migration 0005).
///
/// `scope_id` is the org id (for org members) or workspace id (for workspace
/// members); `scope_kind` distinguishes the two so one DTO serves both
/// membership tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MembershipRow {
    /// Org vs workspace membership domain.
    pub scope_kind: ScopeKind,
    /// Org or workspace id (opaque string form).
    pub scope_id: String,
    /// User vs service-account principal.
    pub principal_kind: PrincipalKind,
    /// Principal id (opaque string form).
    pub principal_id: String,
    /// Role name.
    pub role: String,
    /// When the principal was added/invited.
    pub added_at: String,
    /// Who added the principal (opaque string form), if recorded.
    pub added_by: Option<String>,
}

/// `resources` row (migration 0009).
// guard-justified: `config` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceRow {
    /// `res_` ULID (opaque string form).
    pub id: String,
    /// Owning workspace id (opaque string form).
    pub workspace_id: String,
    /// Resource slug.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Resource-type key.
    pub kind: String,
    /// Resource config blob.
    pub config: serde_json::Value,
    /// Creation timestamp.
    pub created_at: String,
    /// Creator id (opaque string form).
    pub created_by: String,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<String>,
}

/// `triggers` row (migrations 0010 + 0018 webhook_path).
// guard-justified: `config` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerRow {
    /// `trg_` ULID (opaque string form).
    pub id: String,
    /// Owning workspace id (opaque string form).
    pub workspace_id: String,
    /// Bound workflow id (opaque string form).
    pub workflow_id: String,
    /// Trigger slug.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Trigger kind (`manual`/`cron`/`webhook`/`event`/`polling`).
    pub kind: String,
    /// Trigger config blob.
    pub config: serde_json::Value,
    /// Trigger state (`active`/`paused`/`archived`).
    pub state: String,
    /// Service-account run-as id (opaque string form), if set.
    pub run_as: Option<String>,
    /// Extracted webhook path for O(1) dispatch (migration 0018).
    pub webhook_path: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Creator id (opaque string form).
    pub created_by: String,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<String>,
}

/// `org_quotas` + `org_quota_usage` row (migration 0014), flattened to the
/// org-level limits and counters the quota store reads/writes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaRow {
    /// Owning org id (opaque string form).
    pub org_id: String,
    /// Plan tier.
    pub plan: String,
    /// Concurrent-execution limit.
    pub concurrent_executions_limit: i32,
    /// Per-month execution limit, if capped.
    pub executions_per_month_limit: Option<i64>,
    /// Active-workflow limit, if capped.
    pub active_workflows_limit: Option<i32>,
    /// Current concurrent-execution count.
    pub concurrent_executions: i32,
    /// Executions counted in the current month.
    pub executions_this_month: i64,
    /// Month-counter reset timestamp.
    pub month_reset_at: String,
    /// Last-update timestamp.
    pub updated_at: String,
}

/// `audit_log` row (migration 0015).
// guard-justified: `details` is `Option<serde_json::Value>` (not `Eq`
// — can hold a float); the clippy `Eq`-derivable hint is a false
// positive for JSON-bearing rows.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditLogRow {
    /// ULID primary key (opaque string form).
    pub id: String,
    /// Org id (opaque string form).
    pub org_id: String,
    /// Workspace id (opaque string form), `None` for org-level events.
    pub workspace_id: Option<String>,
    /// `"user"` / `"service_account"` / `"system"`.
    pub actor_kind: String,
    /// Actor id (opaque string form), `None` for system events.
    pub actor_id: Option<String>,
    /// Action key (e.g. `workflow.created`).
    pub action: String,
    /// Target kind, if any.
    pub target_kind: Option<String>,
    /// Target id (opaque string form), if any.
    pub target_id: Option<String>,
    /// Event details blob.
    pub details: Option<serde_json::Value>,
    /// Source IP, if recorded.
    pub ip_address: Option<String>,
    /// User agent, if recorded.
    pub user_agent: Option<String>,
    /// Emission timestamp.
    pub emitted_at: String,
}

/// `blobs` row (migration 0019).
// guard-justified: `metadata` is `Option<serde_json::Value>` (not `Eq`
// — can hold a float); the clippy `Eq`-derivable hint is a false
// positive for JSON-bearing rows.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlobRow {
    /// ULID primary key (opaque string form).
    pub id: String,
    /// Owning workspace id (opaque string form).
    pub workspace_id: String,
    /// Associated execution id (opaque string form), if any.
    pub execution_id: Option<String>,
    /// Blob kind (`node_state`/`node_output`/`attachment`/`log`).
    pub kind: String,
    /// MIME content type, if known.
    pub content_type: Option<String>,
    /// Size in bytes.
    pub size_bytes: i64,
    /// SHA-256 checksum, if computed.
    pub checksum: Option<Vec<u8>>,
    /// Storage mode (`db`/`fs`/`s3`).
    pub storage_mode: String,
    /// Inline blob bytes (`None` when stored externally).
    pub data: Option<Vec<u8>>,
    /// External reference (fs path or `s3://` URI), if external.
    pub external_ref: Option<String>,
    /// Kind-specific metadata blob.
    pub metadata: Option<serde_json::Value>,
    /// Creation timestamp.
    pub created_at: String,
    /// Expiry timestamp (`None` = retained with parent).
    pub expires_at: Option<String>,
}
