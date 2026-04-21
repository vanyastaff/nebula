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
//!   `TenantId`, `ProjectId`, `OrganizationId`, `ResourceId`, `RoleId`.
//! - **Keys** — `PluginKey`, `ActionKey`, `CredentialKey`, `ParameterKey`, `ResourceKey`, `NodeKey`
//!   — normalized string keys with validation.
//! - **Scope** — `ScopeLevel`, `Scope`, `Principal`, `ScopeResolver` (Global → … → Action).
//! - **Context** — `Context` trait, `BaseContext`, `BaseContextBuilder`, capability traits
//!   (`HasCredentials`, `HasResources`, `HasMetrics`, `HasEventBus`, `HasLogger`).
//! - **Accessors** — `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`,
//!   `EventEmitter`, `Clock`.
//! - **Guards** — `Guard`, `TypedGuard` RAII guard traits.
//! - **Lifecycle** — `LayerLifecycle`, `ShutdownOutcome`.
//! - **Observability** — `TraceId`, `SpanId`.
//! - **Errors** — `CoreError` (typed, thiserror; no anyhow).
//!
//! ## Credential-specific types moved to `nebula-credential`
//!
//! `AuthScheme`, `AuthPattern`, `CredentialEvent`, and `CredentialId` previously
//! lived here as re-exports. They now live in [`nebula-credential`][nc] directly,
//! so credential-domain vocabulary no longer pollutes the cross-cutting base.
//!
//! [nc]: https://docs.rs/nebula-credential

// ── Modules ─────────────────────────────────────────────────────────────────

/// Accessor trait definitions for capability injection.
pub mod accessor;
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
/// Scope system for resource lifecycle management.
pub mod scope;
/// Shared serde helpers (duration serialization, etc.).
pub mod serde_helpers;

mod error;
mod keys;

// ── Re-exports ──────────────────────────────────────────────────────────────

pub use context::{
    BaseContext, BaseContextBuilder, Context, HasCredentials, HasEventBus, HasLogger, HasMetrics,
    HasResources,
};
pub use dependencies::*;
pub use error::*;
pub use guard::{Guard, TypedGuard};
#[allow(deprecated)] // OrganizationId re-exported for migration period
pub use id::*;
pub use keys::*;
pub use lifecycle::{LayerLifecycle, ShutdownOutcome};
pub use obs::{SpanId, TraceId};
pub use scope::*;

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
        AttemptId, ExecutionId, InstanceId, OrgId, OrganizationId, ResourceId, ServiceAccountId,
        SessionId, TriggerEventId, TriggerId, UserId, WorkflowId, WorkflowVersionId, WorkspaceId,
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
