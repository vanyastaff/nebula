# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once a stable release ships. While the workspace is `frontier`, breaking
changes are expected between minor releases — call them out here.

## [Unreleased]

### Added

- **Plane-A OAuth identity providers from operator secrets (ROADMAP
  §M3.1)** — shipped via 5-PR chain (#757 ADR-0085 + #758 trait +
  #759 real authorize URL + OIDC discovery + #761 real
  `complete_oauth` + `external_identities` + #*** docs). Operators
  configure IdP-client credentials via
  `API_AUTH_OAUTH_<PROVIDER>_*` env vars; `PgAuthBackend::complete_oauth`
  no longer returns `NotImplemented` — it runs the canonical OAuth
  flow (code exchange → userinfo → REQ-oauth-006 short-circuit →
  email truth-table → session mint). New migration
  `0029_external_identities.sql` adds the `(provider, subject) →
  user_id` linkage with `ON DELETE CASCADE`. Three providers in 1.0:
  `google`, `microsoft`, `github`. GitHub triggers a second
  `/user/emails` fetch to resolve verified email per ADR-0085 D-5
  wave-6. id_token JWKS signature validation deferred to 1.1 (D-16);
  userinfo response is authoritative. See `crates/api/README.md`
  "OAuth identity providers (Plane A)" for operator setup.

### Changed

- **`nebula-resource`:** crate documentation scrubbed of plan-IDs, ADR
  numbers, internal issue/PR references, and stale temp-file links.
  Rewrote `docs/README.md` and `docs/topology-reference.md` to the v4
  three-topology surface (`Pooled`, `Resident`, `Bounded` with sealed
  `Cap` typestate). Added `# Errors` / `# Cancellation` / `# Drop` /
  `# Panics` sections to the `Resource` trait lifecycle methods, the
  `ResourceGuard` type, and the `Manager::register` / `acquire_*`
  entry points.
- Renamed runnable examples from `m6_*` to `resource_*`
  (`m6_postgres_pool` → `resource_postgres_pool`,
  `m6_resident_http` → `resource_resident_http`,
  `m6_telegram_multi_workflow` → `resource_telegram_multi_workflow`).
  Workspace `cargo run -p nebula-examples --example …` invocations
  updated accordingly.

### Removed

- `nebula-resource::docs/recovery.md` `WatchdogHandle` /
  `WatchdogConfig` section — these types are not in the public surface.
  Drive `Resource::check()` directly or compose `nebula-resilience`'s
  health-probe layer.

## How to read this file

- **Added** — new public API or capability.
- **Changed** — non-breaking behavior changes, refactors, or documentation
  improvements that may change reader expectations.
- **Deprecated** — public API still present but slated for removal.
- **Removed** — public API gone in this release.
- **Fixed** — bug fixes.
- **Security** — security-relevant fixes.

Per-crate changelogs may appear under `crates/<name>/CHANGELOG.md` once a
crate stabilises. Until then, this workspace-level changelog is the single
source of truth.
