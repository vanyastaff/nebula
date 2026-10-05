//! Version 1 of the Nebula HTTP transport contract.

/// Public API version token.
pub const API_VERSION: &str = "v1";

/// Public API path prefix.
pub const API_BASE_PATH: &str = "/api/v1";

/// Plane-A identity requests, responses and provider vocabulary.
pub mod auth;

/// Catalog request and response bodies.
pub mod catalog;

/// Plane-B credential-management and acquisition wire vocabulary.
pub mod credential;

/// Execution request and response bodies.
pub mod execution;

/// Health request and response bodies.
pub mod health;

/// Unsupported operator-only wire vocabulary.
pub mod internal;

/// Me request and response bodies.
pub mod me;

/// Org request and response bodies.
pub mod org;

/// RFC9457 failure bodies and structured diagnostics.
pub mod problem;

/// Resource request and response bodies.
pub mod resource;

/// Cross-domain pagination and role wire vocabulary.
pub mod shared;

/// Webhook request and response bodies.
pub mod webhook;

/// Workflow request and response bodies.
pub mod workflow;

/// Workspace membership request and response bodies.
pub mod workspace_membership;
