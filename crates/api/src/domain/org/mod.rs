//! Organization domain — org settings, members, service accounts.
//!
//! Self-contained per domain-module layout: route table ([`routes`]), HTTP handlers
//! ([`handler`]), request/response DTOs ([`dto`]), and the canonical
//! in-memory [`MembershipStore`](crate::state::MembershipStore) impl
//! ([`membership`]) live together. Authenticated + org-scoped.
//!
//! ## honest capability status (Phase 3, "Option 1" honest contract)
//!
//! The **member-management** triad is implemented end-to-end against the
//! shared [`MembershipStore`](crate::state::MembershipStore):
//! `GET /orgs/{org}/members`, `POST /orgs/{org}/members` (direct
//! add-by-principal — the fake email-invitation contract was dropped, see
//! [`dto`]), and `DELETE /orgs/{org}/members/{principal}`. The org-record
//! endpoints (`GET`/`PATCH`/`DELETE /orgs/{org}`) and the service-account
//! endpoints stay **honest 501** (honest capability contract): there is no org-record
//! store (name/plan/created_at) and no end-to-end
//! `Principal::ServiceAccount` auth path, so shipping them would be a
//! false capability. Their `#[deprecated]` + 501 + ` (planned)`
//! annotations are unchanged and enforced by
//! `tests/openapi_canon_compliance.rs`.
//!
//! ## Provisioning & durability (provisioning durability / credential secrecy)
//!
//! The first-party server wires [`MembershipStore`](crate::state::MembershipStore)
//! to the selected tenant-directory backend. Optional operator bootstrap accepts
//! only an owner identity already present in the configured authentication backend
//! and provisions the tenant atomically. The in-memory reference adapter remains
//! available for tests and embedded composition. See `crates/api/README.md`
//! ("Org membership durability") and
//! `apps/server/src/compose.rs::default_state`.

pub mod dto;
pub mod handler;
pub mod membership;
pub mod routes;

pub use membership::{BootstrapSeedError, InMemoryMembershipStore};
