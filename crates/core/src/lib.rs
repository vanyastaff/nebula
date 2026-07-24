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
//! - **Identifiers** — `ExecutionId` (`exe_…`), `WorkflowId` (`wf_…`), `WorkflowVersionId`
//!   (`wfv_…`), `OrgId` (`org_…`), `WorkspaceId` (`ws_…`), `UserId` (`usr_…`),
//!   `ServiceAccountId` (`svc_…`), `ResourceId` (`res_…`), `CredentialId` (`cred_…`),
//!   `TriggerId` (`trg_…`), `TriggerEventId` (`evt_…`), `AttemptId` (`att_…`),
//!   `InstanceId` (`nbl_…`), `SessionId` (`sess_…`) — all defined in this crate.
//! - **Experimental transport digest IDs** — available only with
//!   `unstable-worker-flavor`; fixed-width representation only.
//! - **Keys** — `PluginKey`, `ActionKey`, `CredentialKey`, `ParameterKey`, `ResourceKey`, `NodeKey`
//!   — normalized string keys with validation.
//! - **Scope** — `ScopeLevel`, `Scope`, `Principal`, `ScopeResolver` (Global → Organization → Workspace → Workflow → Execution).
//! - **Context** — `Context` trait, `BaseContext`, `BaseContextBuilder`, capability traits
//!   (`HasCredentials`, `HasResources`, `HasMetrics`, `HasEventBus`, `HasLogger`).
//! - **Accessors** — `ResourceAccessor`, `CredentialAccessor`, `Logger`, `MetricsEmitter`,
//!   `EventEmitter`, `Clock`.
//! - **Guards** — `Guard`, `TypedGuard` RAII guard traits for scoped resource and credential
//!   lifecycle.
//! - **Auth** — `AuthScheme` trait, `AuthPattern` enum (module `auth`).
//! - **Lifecycle** — `LayerLifecycle`, `ShutdownOutcome`.
//! - **Observability** — `TraceId`, `SpanId`, `W3cTraceContext`, W3C trace parsing.
//! - **Errors** — `CoreError` (typed, thiserror; no anyhow).
//! - **Roles** — `OrgRole`, `WorkspaceRole`, `effective_workspace_role` (module `role`).
//! - **Permissions** — `Permission` (module `permission`). `PermissionDenied` (module `tenancy`).
//! - **Tenancy** — `TenantContext`, `ResolvedIds` (module `tenancy`).
//! - **Slugs** — `Slug`, `SlugKind`, `SlugError`, `is_prefixed_ulid()` (module `slug`).

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// ── Modules ─────────────────────────────────────────────────────────────────

/// Accessor trait definitions for capability injection.
pub mod accessor;
/// Authentication scheme contract types and pattern classification.
pub mod auth;
/// Validated newtype for workflow branch identifiers.
pub mod branch_key;
/// Context system -- base trait + capabilities.
pub mod context;
/// Dependency declaration types.
pub mod dependencies;
/// Guard traits for RAII resource/credential wrappers.
pub mod guard;
/// Unique identifiers for Nebula entities (prefixed ULIDs).
pub mod id;
/// Shared validation logic for port/branch key newtypes.
pub mod key_validation;
/// Hierarchical cancellation primitive.
pub mod lifecycle;
/// Observability identity types.
pub mod obs;
/// Granular permission definitions.
pub mod permission;
/// Validated newtype for action port identifiers.
pub mod port_key;
/// Organization and workspace role enums.
pub mod role;
/// Scope system for resource lifecycle management.
pub mod scope;
/// Shared serde helpers (duration serialization, etc.).
pub mod serde_helpers;
/// Validated slug strings for human-readable identifiers.
pub mod slug;
/// Async-aware lazy initialization wrapper (`Lazy<X>`).
pub mod sync;
/// Multi-tenant context and resolved IDs.
pub mod tenancy;
/// Fixed-width transport identifiers with canonical lowercase hexadecimal encoding.
#[cfg(feature = "unstable-worker-flavor")]
pub mod transport_digest;

mod error;
mod keys;

// ── Re-exports ──────────────────────────────────────────────────────────────

pub use auth::{AuthPattern, AuthScheme};
pub use branch_key::BranchKey;
pub use context::{
    BaseContext, BaseContextBuilder, Context, HasCredentials, HasEventBus, HasLogger, HasMetrics,
    HasResources,
};
pub use dependencies::*;
pub use error::*;
pub use guard::{Guard, TypedGuard, debug_redacted, debug_typed};
pub use id::*;
pub use key_validation::{KeyValidationError, KeyValidationErrorKind};
pub use keys::*;
pub use lifecycle::{LayerLifecycle, ShutdownOutcome};
pub use obs::{
    ParsedTraceparent, SpanId, TRACESTATE_MAX_BYTES, TraceId, W3C_TRACEPARENT, W3C_TRACESTATE,
    W3cTraceContext, W3cTraceContextError, parse_traceparent,
};
pub use permission::Permission;
pub use port_key::PortKey;
pub use role::{OrgRole, WorkspaceRole, effective_workspace_role};
pub use scope::*;
pub use slug::{Slug, SlugError, SlugKind, is_prefixed_ulid};
pub use sync::Lazy;
pub use tenancy::{PermissionDenial, PermissionDenied, ResolvedIds, TenantContext, WorkspaceGrant};
#[cfg(feature = "unstable-worker-flavor")]
pub use transport_digest::{
    ArtifactSetDigest, PluginSetId, TransportDigestParseError, WorkerFlavorRevisionId,
};

// ── Compile-time key macros ─────────────────────────────────────────────────

/// Constructs a [`PortKey`] from a string literal, validated at **compile time**.
///
/// Invalid literals cause a compile error, not a runtime panic.
///
/// # Example
///
/// ```
/// use nebula_core::port_key;
/// let k = port_key!("out");
/// assert_eq!(k.as_str(), "out");
/// ```
#[macro_export]
macro_rules! port_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::PortKey::is_valid_key_const($s),
            "invalid port key literal"
        );
        $crate::PortKey::new($s).unwrap()
    }};
}

/// Constructs a [`BranchKey`] from a string literal, validated at **compile time**.
///
/// Invalid literals cause a compile error, not a runtime panic.
///
/// # Example
///
/// ```
/// use nebula_core::branch_key;
/// let k = branch_key!("true");
/// assert_eq!(k.as_str(), "true");
/// ```
#[macro_export]
macro_rules! branch_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::BranchKey::is_valid_key_const($s),
            "invalid branch key literal"
        );
        $crate::BranchKey::new($s).unwrap()
    }};
}

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
    pub use crate::id::{
        AttemptId, CredentialId, ExecutionId, InstanceId, OrgId, ResourceId, ServiceAccountId,
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
        action_key, branch_key, credential_key, node_key, parameter_key, plugin_key, port_key,
        resource_key,
    };
}
