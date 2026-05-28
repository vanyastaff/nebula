# OpenSpec Change Status

**Change**: `oauth-providers-from-operator-secrets`
**Status**: ✅ **SHIPPED / ARCHIVED**
**Closed**: 2026-05-28
**Deliverable**: ROADMAP §M3.1 final OAuth-providers checkbox

## PR chain (all merged)

| # | PR | Commit | Scope |
|---|---|---|---|
| 1 | [#757](https://github.com/vanyastaff/nebula/pull/757) | `4d938bdb` | ADR-0085 + SDD planning artifacts |
| 2 | [#758](https://github.com/vanyastaff/nebula/pull/758) | `62c8c601` | `AuthBackend` trait sig + `OAuthProvidersConfig` + compose validation + `test_support` module |
| 3 | [#759](https://github.com/vanyastaff/nebula/pull/759) | `27c3d7c7` | Real authorize URL via `flow::build_authorization_uri` + OIDC discovery cache + `ProviderNotConfigured` variant |
| 4 | [#761](https://github.com/vanyastaff/nebula/pull/761) | `b7b703a1` | Real `complete_oauth` + `external_identities` table + REQ-oauth-006 short-circuit + userinfo helper |
| 5 | [PR-5](pending) | _this PR_ | Docs + ROADMAP flip + 1.1 follow-up plan |

## What landed

- 13 ADR decisions (D-1 through D-16, with D-9-WAVE6 / D-15-WAVE6
  hardening from PR-757 review marathon).
- 27 RED tests across the chain (was 21 baseline; +6 added by
  PR-757 review waves for security + data-model).
- ~3,150 LOC of SDD planning artifacts (audit trail of decisions).
- ~2,400 LOC of production implementation (~1,300 PR-2 + ~600 PR-3
  + ~1,300 PR-4).
- 479 tests passing on `main` post-merge (was 450 pre-chain).
- Single new PG migration `0029_external_identities.sql`.

## Decision audit trail (live)

- **D-1**: env-managed `[auth.oauth.providers.<name>]` config (no DB
  rows, no UI in 1.0).
- **D-3-RECON4**: `redirect_uri` auto-derived from
  `ApiConfig::public_url` (no allow-list).
- **D-5-RECON4**: `OAuthEndpoints` tagged union (Oidc vs Manual)
  with `verified_emails_url` (D-5 wave-6 GitHub addition).
- **D-6**: `AuthError::ProviderNotConfigured` → HTTP 503.
- **D-7**: IdP tokens discarded after session mint.
- **D-8**: `external_identities (provider, subject)` PK + CASCADE,
  `user_id BYTEA` matching `users.id`.
- **D-9-WAVE6**: anti-SSRF gate generalized; flag-aware vs strict
  validator split (F.2 wave-7).
- **D-11-RECON3**: reuse `mint_pkce` + `flow::build_authorization_uri`
  + `OAuthStateRepo` (Plane A; NOT Plane B helpers).
- **D-12-RECON3**: reuse `flow::exchange_code`.
- **D-13**: Plane A does NOT route through
  `Interactive::continue_resolve`.
- **D-14**: `nebula_test_util` cfg-gated `test_support` module (NOT
  a Cargo feature, to avoid transitive activation).
- **D-15-WAVE6**: OIDC discovery cache; per-child-URL re-validation
  before cache insert.
- **D-16**: id_token JWKS signature validation deferred to 1.1
  (userinfo authoritative).

## Superseded sub-decisions (audit only)

- D-2, D-4, REQ-cred-001 (recon-2)
- D-3 original allow-list (recon-4)
- D-11 / D-12 original phrasings (recon-3)

## 1.1 follow-ups

See `docs/plans/2026-05-28-001-feat-oauth-1.1-followups-plan.md` for
the deferred work:

- id_token JWKS signature validation (D-16 defer).
- DNS-resolution SSRF defense-in-depth via custom `reqwest::Client`
  resolver.
- Auth0 / Okta / generic OIDC `OAuthProvider` enum extension.

## Review marathon summary

| PR | Wave-1 issues | Waves to converge | Peak severity |
|----|----|----|----|
| #757 (ADR) | 18 | 8 | **P1 SSRF** |
| #758 (impl) | 9 | 3 | P2 |
| #759 (impl) | 7 | 3 | major |
| #761 (impl) | 5 | 1 (single wave!) | P1 mechanical |

Defect class trajectory across chain:
- PR-1: stale-reference cleanup (waves 1-3) → security/data-model
  (waves 4-7) → 0 (wave-8 with pre-push grep discipline)
- PR-2..4: each subsequent PR cleaner than the last as
  bot-review lessons were applied proactively at design time.

The chain is the **most-reviewed deliverable in M3.1**; the audit
trail under this directory is intentionally preserved for future
ADR-grade work.
