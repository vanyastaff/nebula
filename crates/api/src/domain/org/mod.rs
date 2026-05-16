//! Organization domain — org settings, members, service accounts.
//!
//! Self-contained per canon §12.7: route table ([`routes`]), HTTP handlers
//! ([`handler`]), request/response DTOs ([`dto`]), and the canonical
//! in-memory [`MembershipStore`](crate::state::MembershipStore) impl
//! ([`membership`]) live together. Authenticated + org-scoped.
//!
//! ## §4.5 status (Phase 3, "Option 1" honest contract)
//!
//! The **member-management** triad is implemented end-to-end against the
//! shared [`MembershipStore`](crate::state::MembershipStore):
//! `GET /orgs/{org}/members`, `POST /orgs/{org}/members` (direct
//! add-by-principal — the fake email-invitation contract was dropped, see
//! [`dto`]), and `DELETE /orgs/{org}/members/{principal}`. The org-record
//! endpoints (`GET`/`PATCH`/`DELETE /orgs/{org}`) and the service-account
//! endpoints stay **honest 501** (canon §4.5): there is no org-record
//! store (name/plan/created_at) and no end-to-end
//! `Principal::ServiceAccount` auth path, so shipping them would be a
//! false capability. Their `#[deprecated]` + 501 + ` (planned)`
//! annotations are unchanged and enforced by
//! `tests/openapi_canon_compliance.rs`.

pub mod dto;
pub mod handler;
pub mod membership;
pub mod routes;

pub use membership::{BootstrapSeedError, InMemoryMembershipStore};
