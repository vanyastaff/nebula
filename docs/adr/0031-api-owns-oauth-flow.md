---
id: 0031
title: api-owns-oauth-flow
status: accepted
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [credential, api, oauth2, security, csrf, pkce, ssrf, canon-12.5, canon-4.5]
related:
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0022-webhook-signature-policy.md
  - docs/adr/0023-keyprovider-trait.md
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities
  - docs/STYLE.md#6-secret-handling
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
linear: []
---

# 0031. `nebula-api` owns OAuth flow HTTP ceremony

## Context

[ADR-0028](./0028-cross-crate-credential-invariants.md) establishes the
umbrella of cross-crate credential invariants. [ADR-0029](./0029-storage-owns-credential-persistence.md)
hands persistence to `nebula-storage`. [ADR-0030](./0030-engine-owns-credential-orchestration.md)
hands runtime orchestration (including token refresh) to `nebula-engine`.
This ADR codifies the third migration: **user-facing OAuth2 HTTP
ceremony moves from `nebula-credential::credentials::oauth2_flow`
into `nebula-api/src/credential/`**.

Today, `nebula-credential::credentials::oauth2_flow.rs` mixes four
distinct concerns:

1. Authorization URI construction with PKCE challenge + CSRF token.
2. Callback handling — receive authorization code, validate state,
   exchange code for tokens via HTTP POST.
3. Token refresh during workflow execution (out of scope here;
   migrated to engine per ADR-0030 §4).
4. PKCE primitives (code_verifier / code_challenge generation, SHA-256
   hashing) — pure primitives, no HTTP.

Only (1) + (2) are user-facing HTTP ceremony. They belong in the HTTP
API surface, adjacent to `nebula-api`'s existing auth and webhook
handlers. (3) moved to engine per ADR-0030. (4) stays in
`nebula-credential::secrets::crypto` — pure arithmetic, reachable by
api and engine via sibling dep.

The canon context that binds this migration:

- [§12.5 — Secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth)
  — all credential state reads pass through `CredentialStore`
  (encrypted at rest per ADR-0029). Plaintext token material lives
  only on in-memory request paths, wrapped in zeroize containers.
- [§4.5 — Operational honesty / no false capabilities](../PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities)
  — OAuth endpoints appear in `nebula-api`'s MATURITY row only after
  this migration lands; until then, `credential-oauth` is a non-
  default feature (ADR-0028 invariant 5).
- [STYLE.md §6 — Secret handling](../STYLE.md#6-secret-handling) —
  mandatory patterns for token material on request paths.
- [ADR-0022](./0022-webhook-signature-policy.md) — prior precedent for
  `nebula-api` adopting strict-by-default security posture on HTTP
  surfaces.

## Decision

### 1. Scope — what moves

The following file is split and moved from
`crates/credential/src/credentials/oauth2/flow.rs` into two
destinations:

| From | To |
|---|---|
| Authorization URI construction | `crates/api/src/credential/flow.rs` |
| Callback code → token exchange (HTTP POST to token endpoint) | `crates/api/src/credential/flow.rs` |
| Token refresh during execution | `crates/engine/src/credential/rotation/token_refresh.rs` (ADR-0030) |
| PKCE primitives | `crates/credential/src/secrets/crypto.rs` (stays) |

Additionally, new files in `nebula-api`:

```
crates/api/src/credential/
├── mod.rs
├── oauth_controller.rs   # GET /credentials/:id/oauth2/auth
│                         # GET/POST /credentials/:id/oauth2/callback
├── flow.rs               # HTTP client (reqwest) + token exchange
└── state.rs              # CSRF token generation + pending-state correlation
```

### 2. n8n parity

Pattern parity with n8n's existing production separation (spec §7):

| Step | n8n location | Nebula analog |
|---|---|---|
| Auth URI construct | `packages/cli/src/oauth/oauth.service.ts` | `nebula-api` (`api/credential/flow.rs`) |
| Callback endpoint | `packages/cli/src/controllers/oauth/oauth2-credential.controller.ts` | `nebula-api` (`api/credential/oauth_controller.rs`) |
| Token exchange | `packages/cli/src/oauth/oauth.service.ts` + `@n8n/client-oauth2` | `nebula-api` (`api/credential/flow.rs`) |
| Token refresh | `packages/core/execution-engine/utils/request-helper-functions.ts` (`refreshOrFetchToken`) | `nebula-engine` (`engine/credential/rotation/token_refresh.rs`, per ADR-0030) |
| Credential type def | `packages/nodes-base/credentials/*.credentials.ts` | `nebula-credential` (`credential/credentials/oauth2/`, stays) |

The ceremony / refresh split is n8n's operational shape, validated
across three major versions and thousands of plugin authors. We adopt
it deliberately rather than collapsing ceremony + refresh into a single
crate.

### 3. Endpoints

Three HTTP endpoints on `nebula-api`:

- **`GET /credentials/:id/oauth2/auth`** — construct authorization URI
  (client_id, redirect_uri, scope, state, PKCE challenge), persist a
  pending entry (via the storage pending store per ADR-0029 §4), and
  return either a `302 Location` redirect or a `200 application/json`
  body with `{authorize_url}` for UIs that handle the redirect
  client-side.
- **`GET /credentials/:id/oauth2/callback`** — receive authorization
  code + state. Verify state (§4.3). Consume pending entry (single-use,
  per ADR-0029 §4.3). Exchange code for tokens via HTTP POST. Encrypt
  token state via `CredentialStore` (`EncryptionLayer` per ADR-0029).
- **`POST /credentials/:id/oauth2/callback`** — identical to GET but
  for IdPs that use `response_mode=form_post` (SAML-OIDC, some
  Okta flows). Same invariants.

### 4. Security invariants (non-negotiable, CI-enforced)

The following invariants are enforced by CI tests and are canon-level
for any PR in the OAuth2 area. Violating any of them → CI fail.

#### 4.1. PKCE mandatory S256 (fail-closed on plain)

Authorization URI construction **must** include a PKCE challenge with
`code_challenge_method=S256`. `plain` is not accepted — an IdP
configured for `plain` (or no PKCE) returns
`OAuthError::PkceRequired` without attempting the request. No
fallback.

Rationale: PKCE S256 is the modern OAuth2 baseline (RFC 7636); `plain`
is a historical compromise that modern IdPs universally support
upgrading out of. A fallback path would invite silent downgrade to
`plain` on misconfiguration, which is the exact class of drift §4.5
rejects.

#### 4.2. CSRF token ≤ 10 minutes TTL, single-use, constant-time compare

The `state` parameter carries (among other things) a CSRF token
generated at authorization-URI construction time:

- TTL ≤ 10 minutes from generation. Enforced by pending-store (ADR-
  0029 §4.2).
- Single-use — consumed transactionally at callback via
  `get_then_delete` on the pending store (ADR-0029 §4.3).
- Compared against the callback-provided value with
  `subtle::ConstantTimeEq` (not `==`). Mismatch or expired →
  `OAuthError::CsrfFailure` with HTTP 401. The error message does
  not include the token value (to prevent leak via error log /
  response body).

#### 4.3. State parameter crypto-bound (HMAC, not random)

The `state` parameter is **not** a plain random hex string. It is
`base64url(hmac_sha256(server_secret, csrf_token || credential_id ||
expires_at) || csrf_token || credential_id || expires_at)`.

- HMAC secret lives in the api composition root (config injected).
- Callback recomputes HMAC and verifies constant-time **before**
  consuming the pending record. An attacker who forges a state with
  the right shape but wrong HMAC is rejected without pending-store
  lookup.
- Rejection error is indistinguishable between "HMAC failed",
  "expired", "unknown credential_id" — no side channel.

Rationale: a plain random state is vulnerable to replay if an
attacker intercepts a valid state during flight. HMAC binding ties
the state to the credential and the expiry; interception without
the HMAC secret is useless.

#### 4.4. reqwest client shape — fail-closed limits

The `reqwest::Client` used for token exchange is configured once at
api startup with:

- **TLS only** — rustls backend, no `--insecure` escape hatch.
- **Redirect cap 5** — `redirect::Policy::limited(5)`. Each redirect
  target is re-validated against the token endpoint URL allowlist
  (§4.5). A redirect to a non-allowlisted URL → hard fail.
- **Timeout 30 s** per call — `Client::builder().timeout(Duration::from_secs(30))`.
- **Response body cap 1 MiB** — streaming reader capped. Overage →
  `OAuthError::ResponseTooLarge`. Token endpoints universally
  return <100 KiB; 1 MiB is a belt-and-braces bound against a
  malicious / misconfigured endpoint trying to exhaust memory.

These match the ADR-0025 broker network verb posture — consistent
"fail-closed limits on outbound HTTP" across nebula-api and the
sandbox broker.

#### 4.5. Token endpoint URL allowlist (per-credential, workflow-config)

Each credential config declares `allowed_token_endpoints: Vec<Url>`
at registration time. At runtime, the api verifies the IdP's
token-endpoint URL (from the credential config, not from the IdP's
redirect response) is in the allowlist. **Literal URL match** — no
DNS-resolve-then-compare (defeats allowlist under DNS rebind
attacks).

Redirect chains (up to 5 per §4.4) re-validate each hop against the
allowlist. A redirect target not in the allowlist fails the request.

Rationale: without the allowlist, a misconfigured credential pointing
at `http://169.254.169.254/token` (cloud metadata endpoint) would
exfiltrate the authorization code to the metadata service on behalf
of the workflow. The allowlist defaults to empty — a credential with
no `allowed_token_endpoints` fails to activate the OAuth flow. Same
SSRF-prevent-not-detect posture as ADR-0025 §3.

#### 4.6. Zeroize on partial reqwest failure

Timeout, connection reset, partial response, or any early-return
error path leaves zero plaintext in memory:

- Request body (contains client_secret and authorization code) is a
  `Zeroizing<Vec<u8>>`. Dropped on scope exit regardless of success
  or error.
- Partial response body (if any bytes arrived before a timeout) is
  zeroized via the streaming reader's on-error `Drop`. No `.to_vec()`
  capture of the partial bytes.
- Any `OAuthError` variant that wraps a reqwest error chain filters
  the error chain through a redaction helper (same helper as
  ADR-0030 §4) before formatting.

CI test: mock reqwest failure mid-response, assert that process
memory (queried via a dedicated test helper that scans an
allocated-then-deallocated region) contains no token substring.

### 5. `reqwest` becomes a base dep of `nebula-api`

`reqwest` is already in the workspace `[workspace.dependencies]`.
Adding it to `crates/api/Cargo.toml` as a base dep (not optional) is
consistent with the migration scope. `url` accompanies it for the
typed URL handling required by §4.5 (allowlist is `Vec<Url>`, not
`Vec<String>`).

Removing `reqwest` from `nebula-credential` (per spec §9) is a
consequence of moving both refresh (to engine, ADR-0030) and
ceremony (to api, this ADR). After P10 lands, credential has no
HTTP surface.

### 6. Feature gate during rollout

`crates/api/src/credential/` is feature-gated behind
`credential-oauth` until the `e2e_oauth2_flow` integration test
(spec §13) is green. The feature is **not** default during rollout:

```toml
[features]
default = []
credential-oauth = ["dep:reqwest", "dep:url"]
```

Once the E2E test passes on CI, a separate PR flips the feature into
default (and adjusts MATURITY). Until then, operators building
`nebula-api` without `--features credential-oauth` do not get the
OAuth controller — the api MATURITY row reflects "OAuth ceremony
available, requires --features credential-oauth" honestly, not as a
silent capability claim.

ADR-0028 invariant 5 (operational honesty) applies. The feature
gate is **not** a permanent toggle — it exists only for the rollout
window.

### 7. CI gates for feature matrix

From P10 onwards, CI runs both `--all-features` and
`--no-default-features` matrix legs for `nebula-api`. Without both
legs, `credential-oauth` silently bitrots between releases (compiles
on default path but breaks under `--features credential-oauth`, or
vice versa). The matrix is a required job in `lefthook pre-push` and
the mirror `pr-validation.yml` workflow per CLAUDE.md.

### 8. Canon interaction — §12.5 on request path

Plaintext token material on the api request path lives only in
zeroize containers:

- Request body for token exchange → `Zeroizing<Vec<u8>>`.
- Response body (during parse) → `Zeroizing<String>` then parsed into
  `Zeroizing<OAuth2Tokens>`.
- Writes to `CredentialStore` pass through `EncryptionLayer` per
  ADR-0029 §1. Encrypted at rest immediately.

The api crate does not log tokens — same redaction rules as
ADR-0030 §4 apply. Tracing spans on callback carry credential_id,
status_code, latency_ms, token-endpoint host (without query). Never
the body. Never headers except `Content-Type` / `Content-Length`.

## Consequences

**Positive.**

- OAuth2 HTTP ceremony lives in the crate whose layer is HTTP.
  Operators tracing a callback from access-log to code path hit
  `api/credential/` on the first grep.
- `nebula-credential` becomes a pure contract crate post-P10 (spec
  §2): no HTTP, no reqwest, no url. Base-dep diet complete.
- n8n parity on the ceremony / refresh split means plugin authors
  familiar with n8n's architecture find analogous code in the same
  pattern.
- Security invariants §4.1-§4.6 are CI-enforced, not prose-enforced.
  A PR that weakens any invariant fails CI.
- Feature gate during rollout per §6 means the MATURITY row for
  `nebula-api` stays honest — OAuth is "available under
  `credential-oauth`" until the E2E integration test is green,
  then "stable."
- `reqwest` configuration (§4.4 + ADR-0030 §5 + ADR-0025 §3) is
  consistent across nebula-api, nebula-engine, and nebula-sandbox:
  TLS, bounded timeout, bounded body, allowlist-enforced
  destinations. One mental model for outbound HTTP.

**Negative / accepted costs.**

- `nebula-api` takes on reqwest + url base deps (P10). Increases
  compile time and binary size. Accepted: api is the HTTP layer; it
  is the right place for outbound HTTP ceremony.
- Security invariants §4 require explicit allowlist configuration
  per credential. Operators moving from the prior (permissive)
  posture must declare `allowed_token_endpoints` for every OAuth2
  credential. Migration note in `docs/UPGRADE_COMPAT.md` (follow-up).
- Feature gate §6 is an ongoing CI matrix cost (two extra legs
  per api test). Accepted — without both legs `credential-oauth`
  bitrots. This is a permanent cost during the rollout window; once
  `credential-oauth` flips to default, the cost reduces to the
  `--no-default-features` leg only.
- The state-parameter HMAC (§4.3) requires a server secret in the
  composition root, in addition to the encryption key (ADR-0023) and
  the JWT secret (per `api/src/config.rs`). Three secrets to manage.
  Accepted — each has distinct lifecycle (encryption rotates with
  credential key rotation; JWT rotates with session policy; OAuth
  state HMAC rotates with a longer cadence). Conflating them would
  be a regression.

**Neutral.**

- n8n's `@n8n/client-oauth2` is a reqwest-analog with a small
  feature set; we do not adopt the crate itself (adds a
  dependency hop) but adopt its shape.
- PKCE primitives stay in `nebula-credential::secrets::crypto` —
  pure arithmetic, reachable by api via sibling dep. Not moved.
- `CredentialStore` is consumed from `nebula-storage` per ADR-0029.
  The api crate's composition root already depends on
  `nebula-storage` for session storage and similar; no new dep
  edge.

## Alternatives considered

### A. Leave OAuth ceremony in `nebula-credential::credentials::oauth2_flow`

**Rejected.** Preserves today's layering violation: credential crate
contains HTTP ceremony. Reqwest stays as a credential base dep.
MATURITY `partial / Engine integration` for credential does not
improve. Failure to act on the spec's §2 target shape.

### B. Put ceremony in `nebula-engine` alongside refresh

**Rejected.** Engine is the execution layer; ceremony is HTTP
request/response work initiated by a user. Canonical split — see
n8n's layering (§2). A user-initiated authorization request is not
a workflow execution event.

### C. New `nebula-oauth` crate

**Rejected.** Adds a seventh workspace member for one concern that
fits cleanly into `nebula-api`. `nebula-api` already owns HTTP
routing, auth handling, webhook reception — OAuth2 endpoints are the
same kind of work.

### D. Accept plain-state instead of HMAC-state (§4.3)

**Rejected.** Plain random state is vulnerable to interception +
replay. The HMAC shape is standard OAuth2 threat-model hardening;
accepting plain is a §11.6 false-capability claim ("CSRF defense" that
does not defend against interception).

### E. Drop the token endpoint URL allowlist (§4.5), rely on workflow-
config trust

**Rejected.** Workflow-config is edited by humans; misconfigurations
happen. The allowlist is a second layer of defense, not a replacement
for config review. Same SSRF-prevent posture as ADR-0025 §3 for
broker network verbs.

### F. Allow PKCE `plain` as fallback for legacy IdPs (§4.1)

**Rejected.** Every major IdP in production (Google, Microsoft,
GitHub, GitLab, Okta, Auth0) supports `S256`. A `plain` fallback
would land us in the "legacy IdP support" bitrot class, where tests
never exercise the fallback path and it silently breaks. Fail-closed
is honest.

### G. Log response body at DEBUG for troubleshooting

**Rejected.** Access tokens in DEBUG logs is a §12.5 violation and
a common exfiltration vector. Debug-time introspection uses a
redacted dump helper (same one the redaction test helper drives)
that can be toggled per-call with strict filtering; general DEBUG
logs do not contain bodies.

## Seam / verification

Files that carry the invariants after the migration (spec §3):

- `crates/api/src/credential/mod.rs` — module root.
- `crates/api/src/credential/oauth_controller.rs` — three HTTP
  endpoints (§3). Feature-gated under `credential-oauth` during
  rollout (§6).
- `crates/api/src/credential/flow.rs` — auth URI construction, token
  exchange HTTP client. Reqwest configuration per §4.4.
- `crates/api/src/credential/state.rs` — CSRF + pending-state
  correlation, HMAC-state generation/verification (§4.3).
- `crates/storage/src/credential/pending.rs` — pending store
  (created by ADR-0029); api consumes.
- `crates/credential/src/secrets/crypto.rs` — PKCE primitives stay;
  api consumes.

Test coverage:

- `crates/api/tests/e2e_oauth2_flow.rs` — end-to-end OAuth2 cycle
  (register, authorize, mock callback, resolve, refresh, rotate,
  revoke). Spec §13 integration test. Gate for MATURITY flip.
- `crates/api/tests/oauth_pkce_enforcement.rs` — §4.1: plain rejected,
  S256 accepted. Mock IdP returns plain → api refuses.
- `crates/api/tests/oauth_csrf_invariants.rs` — §4.2: TTL expiry,
  single-use replay rejection, constant-time compare (timing-attack
  regression test).
- `crates/api/tests/oauth_state_hmac.rs` — §4.3: forged state
  rejected; HMAC tamper rejected; error-message side-channel absent.
- `crates/api/tests/oauth_reqwest_limits.rs` — §4.4: 31 s timeout
  fires; 2 MiB response fails; 6th redirect rejected; TLS-only
  (plain http://... refused).
- `crates/api/tests/oauth_endpoint_allowlist.rs` — §4.5: literal
  match required; DNS-rebind defeated (URL match not IP match);
  redirect to non-allowlisted → fail.
- `crates/api/tests/oauth_memory_zeroize.rs` — §4.6: post-request
  memory scan shows no token substring.

CI signals:

- **Layer direction in `deny.toml`**: `nebula-api` may depend on
  `nebula-storage` + `nebula-credential` + `nebula-engine`; reverse
  direction forbidden. Rule lands in P10 (same PR as the move).
- **Feature matrix** (§7): `--all-features` and
  `--no-default-features` legs required for `nebula-api` from P10.
- **Redaction CI**: per §8, token substring absent from all outputs.
  Row in `crates/api/tests/oauth_redaction.rs`.
- **MSRV 1.95**: all new files respect MSRV per
  [ADR-0019](./0019-msrv-1.95.md).

## Follow-ups

- **Phase P10 implementation PR** (spec §12) — physical creation of
  `crates/api/src/credential/` with `oauth_controller.rs`, `flow.rs`,
  `state.rs`. `reqwest` and `url` become api base deps (behind
  `credential-oauth` feature).
- **E2E integration test green** → flip `credential-oauth` to
  default, update MATURITY for `nebula-api` accordingly. Separate
  post-P11 PR.
- **Operator migration doc** — `docs/UPGRADE_COMPAT.md` entry for
  `allowed_token_endpoints` (§4.5) per-credential config field.
- **Refresh token revocation endpoint** — post-migration enhancement;
  new ADR.
- **OIDC support** — post-migration; mostly additive, but ID token
  validation has its own invariant set. New ADR.
- **[ADR-0030](./0030-engine-owns-credential-orchestration.md)
  token_refresh CI gate** — adjacent redaction gate lives in engine;
  ceremony's gate lives in api. Both cite ADR-0028 invariant 7.
