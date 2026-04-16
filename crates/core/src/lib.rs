//! # Nebula Core
//!
//! Vocabulary crate for the Nebula workflow engine.
//!
//! Provides typed identifiers (prefixed ULIDs), domain keys, scope system,
//! auth contracts, error types, and spec 23 context/accessor/guard/lifecycle
//! primitives.
//!
//! ## Key Components
//!
//! - **Identifiers**: Prefixed ULID types -- `ExecutionId` (`exe_01J9...`), `WorkflowId`
//!   (`wf_01J9...`), etc.
//! - **Keys**: `PluginKey`, `ActionKey`, `ParameterKey`, `CredentialKey`, `ResourceKey`, `NodeKey`
//!   -- normalized string keys.
//! - **Scope System**: `ScopeLevel`, `Scope`, `Principal`, `ScopeResolver`.
//! - **Auth Contracts**: `AuthScheme`, `AuthPattern`.
//! - **Context**: `Context` trait, `BaseContext`, capability traits.
//! - **Accessors**: `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`,
//!   `EventEmitter`, `Clock`.
//! - **Guards**: `Guard`, `TypedGuard` RAII traits.
//! - **Lifecycle**: `LayerLifecycle`, `ShutdownOutcome`.
//! - **Observability**: `TraceId`, `SpanId`.

// ── Modules ─────────────────────────────────────────────────────────────────

/// Accessor trait definitions for capability injection.
pub mod accessor;
/// Authentication scheme contract types and pattern classification.
pub mod auth;
/// Context system -- base trait + capabilities.
pub mod context;
/// Credential lifecycle events for cross-crate signaling.
pub mod credential_event;
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

pub use auth::{AuthPattern, AuthScheme};
pub use context::{
    BaseContext, BaseContextBuilder, Context, HasCredentials, HasEventBus, HasLogger, HasMetrics,
    HasResources,
};
pub use credential_event::CredentialEvent;
pub use dependencies::*;
pub use error::*;
pub use guard::{Guard, TypedGuard};
#[allow(deprecated)] // OrganizationId re-exported for migration period
pub use id::*;
pub use keys::*;
pub use lifecycle::{LayerLifecycle, ShutdownOutcome};
pub use obs::{SpanId, TraceId};
pub use scope::*;

/// Common prelude for Nebula crates.
pub mod prelude {
    // Core result alias
    // Parse errors
    pub use domain_key::{KeyParseError, UlidParseError};

    // Dependency error type
    pub use crate::dependencies::DependencyError;
    pub use crate::error::{CoreError, CoreResult};
    // Identifiers (ULID-backed)
    #[allow(deprecated)] // OrganizationId re-exported for migration period
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
