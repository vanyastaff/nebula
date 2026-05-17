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
//!
//! ## Provisioning & durability (canon §11.6 / §12.5)
//!
//! The member endpoints require an explicitly-provisioned
//! [`MembershipStore`](crate::state::MembershipStore) — the default
//! `apps/server` binary deliberately leaves it **unwired** (`None`), so
//! these endpoints return an honest **503** there (the
//! `membership_or_503` port-absent path), and
//! [`crate::middleware::rbac`] stays inert (no spurious 404 on any
//! route). Auto-seeding a bootstrap owner into the default binary was
//! removed (PR #671 P1): the default `AuthBackend` is empty, so an
//! auto-seeded owner could never authenticate and a seeded store would
//! 404-deadlock every org/workspace route (a deployment-level §4.5 false
//! capability); a hardcoded auto-seeded admin would also be a
//! default-credential surface (§12.5). An operator/integrator provisions
//! it via [`membership::InMemoryMembershipStore::seeded_bootstrap`] +
//! [`crate::AppState::with_membership_store`], registering the same
//! bootstrap-owner identity in the wired `AuthBackend` so it can
//! authenticate. State is process-local (lost on restart; not shared
//! across replicas) — same local-first posture as `me/*` and the
//! `memory` idempotency backend. See `crates/api/README.md`
//! ("Org membership durability") and
//! `apps/server/src/compose.rs::default_state`.

pub mod dto;
pub mod handler;
pub mod membership;
pub mod routes;

pub use membership::{BootstrapSeedError, InMemoryMembershipStore};
