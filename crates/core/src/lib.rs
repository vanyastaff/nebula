//! # nebula-core
//!
//! Shared vocabulary for the Nebula workflow engine — the one crate every other crate
//! can safely depend on for typed identifiers, normalized keys, scope levels, auth scheme
//! enums, context contracts, and lifecycle signals.
//!
//! See `crates/core/README.md` for the full role description and contract invariants.
//!
//! ## Purpose
//!
//! Provides the stable opaque handles shared by every other crate. Without this crate,
//! each would invent its own ULID newtype, scope concept, or auth enum — and diverge.
//!
//! ## Public API
//!
//! - **Identifiers** — `ExecutionId` (`exe_…`), `WorkflowId` (`wf_…`), `NodeId`, `UserId`,
//!   `TenantId`, `ProjectId`, `OrganizationId`, `ResourceId`, `CredentialId` (`cred_…`), `RoleId`.
//! - **Keys** — `PluginKey`, `ActionKey`, `CredentialKey`, `ParameterKey`, `ResourceKey`, `NodeKey`
//!   — normalized string keys with validation.
//! - **Scope** — `ScopeLevel`, `Scope`, `Principal`, `ScopeResolver` (Global → … → Action).
//! - **Context** — `Context` trait, `BaseContext`, `BaseContextBuilder`, capability traits
//!   (`HasCredentials`, `HasResources`, `HasMetrics`, `HasEventBus`, `HasLogger`).
//! - **Accessors** — `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`,
//!   `EventEmitter`, `Clock`.
//! - **Guards** — `Guard`, `TypedGuard` RAII guard traits for scoped resource and credential
//!   lifecycle.
//! - **Auth** — `AuthScheme` trait, `AuthPattern` enum (module `auth`).
//! - **Lifecycle** — `LayerLifecycle`, `ShutdownOutcome`.
//! - **Observability** — `TraceId`, `SpanId`.
//! - **Errors** — `CoreError` (typed, thiserror; no anyhow).
//! - **Roles** — `OrgRole`, `WorkspaceRole`, `effective_workspace_role` (module `role`).
//! - **Permissions** — `Permission`, `PermissionDenied` (module `permission`).
//! - **Tenancy** — `TenantContext`, `ResolvedIds` (module `tenancy`).
//! - **Slugs** — `Slug`, `SlugKind`, `SlugError`, `is_prefixed_ulid()` (module `slug`).

// ── Modules ─────────────────────────────────────────────────────────────────

/// Accessor trait definitions for capability injection.
pub mod accessor;
/// Authentication scheme contract types and pattern classification.
pub mod auth;
/// Context system -- base trait + capabilities.
pub mod context;
/// Dependency declaration types.
pub mod dependencies;
/// Guard traits for RAII resource/credential wrappers.
pub mod guard;
/// Unique identifiers for Nebula entities (prefixed ULIDs).
pub mod id;
/// Hierarchical cancellation primitive.
pub mod lifecycle;
/// Observability identity types.
pub mod obs;
/// Granular permission definitions.
pub mod permission;
/// Organization and workspace role enums.
pub mod role;
/// Scope system for resource lifecycle management.
pub mod scope;
/// Shared serde helpers (duration serialization, etc.).
pub mod serde_helpers;
/// Validated slug strings for human-readable identifiers.
pub mod slug;
/// Multi-tenant context and resolved IDs.
pub mod tenancy;

mod error;
mod keys;

// ── Re-exports ──────────────────────────────────────────────────────────────

pub use auth::{AuthPattern, AuthScheme};
pub use context::{
    BaseContext, BaseContextBuilder, Context, HasCredentials, HasEventBus, HasLogger, HasMetrics,
    HasResources,
};
pub use dependencies::*;
pub use error::*;
pub use guard::{Guard, TypedGuard, debug_redacted, debug_typed};
#[allow(deprecated)] // OrganizationId re-exported for migration period
pub use id::*;
pub use keys::*;
pub use lifecycle::{LayerLifecycle, ShutdownOutcome};
pub use obs::{SpanId, TraceId};
pub use permission::Permission;
pub use role::{OrgRole, WorkspaceRole, effective_workspace_role};
pub use scope::*;
pub use slug::{Slug, SlugError, SlugKind, is_prefixed_ulid};
pub use tenancy::{PermissionDenied, ResolvedIds, TenantContext};

/// Named parse-error type for [`PluginKey`] — `<PluginKey as std::str::FromStr>::Err`.
///
/// Provides a stable name for use in error variant payloads and public APIs
/// that report key-validation failures without spelling out the long
/// `<PluginKey as FromStr>::Err` form.
pub type PluginKeyParseError = <PluginKey as std::str::FromStr>::Err;

/// Common prelude for Nebula crates.
pub mod prelude {
    // Core result alias
    // Parse errors
    pub use domain_key::{KeyParseError, UlidParseError};

    // Dependency error type
    pub use crate::dependencies::DependencyError;
    pub use crate::error::{CoreError, CoreResult};
    // Identifiers (ULID-backed)
    #[expect(deprecated, reason = "OrganizationId re-exported for migration period")]
    pub use crate::id::{
        AttemptId, CredentialId, ExecutionId, InstanceId, OrgId, OrganizationId, ResourceId,
        ServiceAccountId, SessionId, TriggerEventId, TriggerId, UserId, WorkflowId,
        WorkflowVersionId, WorkspaceId,
    };
    // Domain keys (normalized string keys)
    pub use crate::keys::{
        ActionKey, CredentialKey, NodeKey, ParameterKey, PluginKey, ResourceKey,
    };
    // Scope
    pub use crate::scope::{Principal, Scope, ScopeLevel, ScopeResolver};
    // Compile-time-validated key construction macros
    pub use crate::{
        action_key, credential_key, node_key, parameter_key, plugin_key, resource_key,
    };
}
