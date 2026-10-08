//! Tenant directory and workspace-object row DTOs.
//!
//! Ids surface as opaque `String`s (the typed-id encode/decode happens at the
//! adapter edge); JSON columns surface as `serde_json::Value`.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An organization: the tenant root.
// guard-justified: `settings` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[expect(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrgRow {
    /// `org_` ULID (opaque string form).
    pub id: String,
    /// Org slug, unique among active orgs.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// When the org was created.
    pub created_at: DateTime<Utc>,
    /// The principal that created the org (opaque string form).
    pub created_by: String,
    /// Plan tier.
    pub plan: String,
    /// Billing email.
    pub billing_email: Option<String>,
    /// Org settings document.
    pub settings: serde_json::Value,
    /// Optimistic-CAS version.
    pub version: u64,
    /// When the org was soft-deleted.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A workspace inside an organization.
// guard-justified: `settings` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[expect(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRow {
    /// `ws_` ULID (opaque string form), unique across organizations.
    pub id: String,
    /// Owning org id (opaque string form).
    pub org_id: String,
    /// Workspace slug, unique among the org's active workspaces.
    pub slug: String,
    /// Display name.
    pub display_name: String,
    /// Description.
    pub description: Option<String>,
    /// When the workspace was created.
    pub created_at: DateTime<Utc>,
    /// The principal that created the workspace (opaque string form).
    pub created_by: String,
    /// Whether this is the org's default workspace.
    pub is_default: bool,
    /// Workspace settings document.
    pub settings: serde_json::Value,
    /// Optimistic-CAS version.
    pub version: u64,
    /// When the workspace was soft-deleted.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Which kind of principal holds a membership or a token.
///
/// Stored verbatim as the `principal_kind` text column (`"user"` /
/// `"service_account"`). A closed enum: an unknown stored value fails closed
/// at the adapter edge rather than widening access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// `resources` row (migration 0071).
// guard-justified: `config` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[expect(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Serialize, Deserialize, PartialEq)]
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
    /// Credential selectors keyed by the resource's declared slot name.
    ///
    /// These bindings are persisted separately from `config` so credential
    /// authority cannot be smuggled through resource-specific values. Values
    /// remain untrusted selectors until the credential runtime resolves them
    /// under the authenticated [`Scope`](crate::Scope).
    #[serde(default)]
    pub credential_bindings: std::collections::BTreeMap<String, String>,
    /// Operator topology settings (pool size, timeouts, concurrency mode),
    /// kept apart from `config` because they tune the runtime, not the
    /// resource. `None` uses the kind's defaults.
    #[serde(default)]
    pub topology: Option<serde_json::Value>,
    /// Operator override of the resilience the resource's kind declares
    /// (`{"rate": {"requests", "period_ms", "burst"}}`), validated against the
    /// kind's policy before it is stored. `None` enforces the declared policy
    /// as is.
    #[serde(default)]
    pub resilience_override: Option<serde_json::Value>,
    /// Creation timestamp.
    pub created_at: String,
    /// Creator id (opaque string form).
    pub created_by: String,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Soft-delete timestamp.
    pub deleted_at: Option<String>,
}

impl std::fmt::Debug for ResourceRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResourceRow")
            .field("id", &self.id)
            .field("workspace_id", &self.workspace_id)
            .field("slug", &self.slug)
            .field("display_name", &self.display_name)
            .field("kind", &self.kind)
            .field("config", &"<redacted>")
            .field("credential_binding_count", &self.credential_bindings.len())
            .field("topology", &self.topology.is_some())
            .field("resilience_override", &self.resilience_override.is_some())
            .field("created_at", &self.created_at)
            .field("created_by", &self.created_by)
            .field("version", &self.version)
            .field("deleted_at", &self.deleted_at)
            .finish()
    }
}

/// `triggers` row (migrations 0010 + 0018 webhook_path).
// guard-justified: `config` is `serde_json::Value` (not `Eq` — can
// hold a float); the clippy `Eq`-derivable hint is a false positive for
// JSON-bearing rows.
#[expect(clippy::derive_partial_eq_without_eq)]
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
