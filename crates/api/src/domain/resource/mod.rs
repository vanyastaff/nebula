//! Resource domain — workspace resource catalog (config-CRUD + read-only
//! runtime status).
//!
//! Self-contained per canon §12.7: HTTP handlers ([`handler`]) and
//! request/response DTOs ([`dto`]) live together; the live route table is
//! assembled in [`crate::domain::workspace`] (tenant-prefix nesting) on an
//! `OpenApiRouter` so served paths and the published OpenAPI spec share one
//! source of truth (ADR-0047).
//!
//! The implemented surface is config-CRUD over the persisted resource
//! *definitions* (`list`/`get`/`create`/`update` (CAS)/`delete` (soft))
//! plus a **read-only** runtime-status projection
//! ([`handler::get_resource_status`]). Resource lifecycle
//! (acquire/refresh/revoke/drain/reload) is engine-owned and is
//! intentionally NOT exposed over HTTP — there is deliberately no
//! acquire/release/drain route (INTEGRATION_MODEL §13.1). These endpoints
//! are real (no ADR-0047 501 stub).

pub mod dto;
pub mod handler;
