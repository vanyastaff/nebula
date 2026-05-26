# PR1 — CSRF enforcement wiring — Reviewer Report

**Commit reviewed:** `f0c8ad10eb3d29b729b9037a664d02b12e3d52e4`
**Branch:** `feat/api-csrf-enforce`
**Worktree:** `.worktrees/csrf-enforce`
**Base:** `origin/main` at `e66bedb1`
**Verdict:** **LGTM with nits**

## Verdict justification

The diff faithfully implements §PR1 of the plan: `csrf_middleware` is layered
on `credential_routes` and on a new session-bearing `auth_mfa_session_routes`
sub-router with the documented `.layer(csrf).layer(auth)` order (axum applies
the outer-most `.layer()` first, so the request flows `auth → csrf → handler`,
which is the contract `csrf_middleware` documents at
`crates/api/src/middleware/csrf.rs:23`). The `mfa_verify` dual-handler is
correctly split into a session-required enrollment-confirm path and a
cookie-less `mfa_complete_login` at `POST /auth/login/mfa`; the cookie-less
endpoint cannot mint a session for the wrong identity because
`InMemoryAuthBackend::verify_mfa` resolves the user solely from the
`challenge_token` it removes from `mfa_challenges`
(`crates/api/src/domain/auth/backend/in_memory.rs:309-337`). Independent
re-run of `cargo nextest run -p nebula-api` reproduces 417/417 passed,
1 skipped — matching the worker's claim. `cargo clippy -p nebula-api
--all-targets -- -D warnings` is clean. `csrf_middleware` itself was not
touched (plan-forbidden). The single non-PR1 test adjustment in
`e2e_oauth2_flow.rs` is legitimate — without the CSRF pair the request
would be rejected at 403 by the freshly-layered `csrf_middleware` *before*
reaching the disabled-route handler that asserts 410-Gone, so the 410
contract this test pins becomes unreachable.

Nits below are documentation drift and a small CI-doc hygiene regression;
none of them block merge.

## Blocker findings (must fix before merge)

_None._

## Nit findings (nice-to-fix; not blocking)

1. **OpenAPI `security` drift on session-bearing MFA endpoints.**
   `crates/api/src/domain/auth/handler.rs:291` (`mfa_enroll`) and `:329`
   (`mfa_verify`) still declare `security(())` (i.e. no auth) even though
   the new wiring places them behind `auth_middleware` + `csrf_middleware`.
   The companion `me` handlers correctly advertise the requirement via
   `security(("bearer" = []), ("api_key" = []))`
   (`crates/api/src/domain/me/handler.rs:126,167,234,272,309,379`). SDK
   generators that read the spec will not know these endpoints need
   credentials. For `mfa_enroll` this is pre-existing drift (the handler
   previously extracted the cookie inline), but for `mfa_verify` the PR
   introduces the divergence by removing the cookie-less branch — the
   `security(())` annotation is now strictly wrong. Recommend updating
   both `#[utoipa::path]` blocks to advertise the bearer/cookie scheme
   plus the `csrf` scheme already registered in
   `crates/api/src/openapi/mod.rs:111-119`.

2. **Two new `cargo doc --no-deps -p nebula-api` warnings.**
   `crates/api/src/domain/auth/routes.rs:13` and `:38` link to
   `crate::domain::build_openapi_router`, which is `fn` (private).
   `rustdoc` emits two `public documentation for ... links to private
   item` warnings. The plan's full-slice verification gate (M10.5)
   demands warning-free `cargo doc --no-deps --workspace`; PR1's
   per-PR acceptance only mentions `task dev:check`, which does not
   build docs, so this is not a hard block now — but it would fail the
   closing M10.5 gate. Fix: either retarget the link (e.g.
   `[`build_openapi_router`]` → backticked plain text, or to the
   public `crate::domain::create_routes`), or annotate the items with
   `#[doc(hidden)]` exposure. Trivial.

3. **Drift smoke does not pin the MFA route surface.**
   `crates/api/tests/openapi_spec.rs:411-415` only asserts on
   `/api/v1/auth/{signup,login,logout}`. The new `/api/v1/auth/login/mfa`
   (split out by this PR) and the pre-existing `/api/v1/auth/mfa/enroll`
   + `/api/v1/auth/mfa/verify` are not in the `expected` array, so a
   future accidental removal of the new endpoint would not fail drift
   smoke. Recommend adding the three MFA paths to the expected list in a
   follow-up.

4. **Pre-existing OpenAPI cookie-name drift (NOT introduced by this PR).**
   `crates/api/src/openapi/mod.rs:114-117` describes the `csrf` security
   scheme as "The token MUST match the value in the `__Host-csrf`
   cookie." but the actual cookie is named `nebula_csrf`
   (`crates/api/src/domain/auth/backend/session.rs:30`). The new README
   section correctly uses `nebula_csrf`. Flag for follow-up cleanup —
   not part of PR1 scope.

5. **`crates/api/.pi-lens/` not in `.gitignore`.** Worker flagged this
   already. The directory is a tooling cache; consider adding a global
   ignore line in a separate `chore(repo):` commit. Not blocking PR1.

6. **`MfaVerifyRequest`/`MfaVerifyResponse` were removed outright.**
   The OpenAPI spec is the public contract and the schema names changed
   from `MfaVerifyResponse` (untagged `oneOf`) to two distinct
   `LoginResponse` / `AckResponse` shapes. The plan accepts this for
   pre-1.0 API, but SDK generators that already pinned to those names
   will rebuild against new types. The worker's offer to restore them as
   `#[deprecated]` aliases for one cycle is sensible if any external
   SDK has been generated; if no published SDK exists yet, the clean
   break is preferable. Decision needed from product/release lead — see
   open-questions list.

## Re-run evidence

- `cargo nextest run -p nebula-api` from the worktree:
  **417 tests run: 417 passed, 1 skipped** in ~32 s (matches worker's
  self-report; the skipped is a `DATABASE_URL`-gated case).
- `cargo clippy -p nebula-api --all-targets -- -D warnings`:
  **clean** (no warnings, no errors; 0.65 s incremental).
- `cargo fmt -p nebula-api -- --check`: **clean** (no output).
- `cargo test -p nebula-api --doc`: **2 passed, 1 ignored**.
- `cargo doc -p nebula-api --no-deps`: **builds, 2 new warnings** (see
  Nit 2). HEAD~1 (base) produced 0 warnings — the 2 are introduced by
  this PR's routes-module docs.
- `rg -n "MfaVerifyRequest|MfaVerifyResponse"` inside the worktree:
  **ZERO HITS** (matches the worker's claim; the false-positive in the
  parent repo grep is the un-merged copy of `crates/api/` still under
  `C:/Users/vanya/RustroverProjects/nebula/`, which is the pre-PR1
  tree — `.worktrees/` is gitignored, so rg from the parent path
  scans the *old* checkout).

## Specific file:line audits

### Middleware ordering — `crates/api/src/domain/mod.rs`

`crates/api/src/domain/mod.rs:85-91`:
```rust
let auth_mfa_session_routes = auth::routes::mfa_session_router()
    .layer(middleware::from_fn(csrf_middleware))
    .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));
```

`crates/api/src/domain/mod.rs:135-140`:
```rust
let credential_routes = credential::routes::router()
    .layer(middleware::from_fn(csrf_middleware))
    .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));
```

Both blocks mirror the existing `me_routes` (`:99-104`) and `tenant_routes`
(`:107-123`) layering shape. The axum semantics of `Router::layer(X).layer(Y)`
wrap Y *around* X, so the request flow is `Y → X → handler` =
`auth_middleware → csrf_middleware → handler`. This is the contract
declared at the top of `crates/api/src/middleware/csrf.rs:23-24` ("Must run
AFTER auth middleware"). Verified.

The cookie-less `mfa_complete_login` lives on the un-layered `auth_routes`
sub-router (`crates/api/src/domain/auth/routes.rs:20-32`) which is the
right place — neither `auth_middleware` nor `csrf_middleware` runs on it,
matching the plan's "cookie-less by construction, CSRF-exempt" intent.
There is no stray `.merge(...)` of `auth_mfa_session_routes` into anything
that would also pick up a credential-write surface.

### MFA handler split — `crates/api/src/domain/auth/handler.rs`

**`mfa_verify` (now enrollment-confirm only) — `:339-350`.** Reads the
session cookie via `principal_from_cookie`, derives `user_id`, calls
`backend.confirm_mfa_enrollment(&user_id, &body.code)`. No session
minted on this path. `#[tracing::instrument(...)]` at `:338`. The
`#[utoipa::path]` annotation at `:325-336` still uses `security(())` —
see Nit 1. Body type is `MfaConfirmEnrollRequest`, which matches the
`Json<MfaConfirmEnrollRequest>` extractor — verified.

**`mfa_complete_login` (new) — `:371-379`.** Pure cookie-less path:
no `principal_from_cookie`, no header parsing for session. Resolves
identity entirely through `backend.verify_mfa(&body.challenge_token,
&body.code)`. The backend (`in_memory.rs:309-337`) removes the
challenge from `mfa_challenges` (so it is single-use), validates
expiry, fetches the matching user, runs TOTP verify against
`user.mfa_secret`, and returns that user's profile. There is no path
on which the request body alone can name a different user — the
challenge token is the identity authority, exactly as the plan
intends. `mint_session_response(backend, user)` then mints a session
for the *returned* user. `#[tracing::instrument(...)]` at `:372`.
`#[utoipa::path]` at `:357-368` declares `request_body =
MfaLoginCompleteRequest`, which matches the
`Json<MfaLoginCompleteRequest>` extractor signature.

**Error mapping consistency.** Both handlers funnel `AuthError →
ApiError` through `map_err(ApiError::from)`. The
`From<AuthError> for ApiError` impl
(`crates/api/src/domain/auth/backend/error.rs:73-103`) covers all
relevant variants:
- `InvalidToken` → `Unauthorized` (401) for expired/bogus challenge.
- `InvalidMfaCode` → `Unauthorized` (401) for a wrong TOTP.
- `Internal` → `Internal` (500) for the "mfa challenge for non-mfa user"
  bail-out at `in_memory.rs:331`.

No new error variants were introduced, so no DoD-mandated typed-error
work was needed for these handlers.

### Test adequacy

**`crates/api/tests/auth_mfa_csrf.rs` (new, 168 LOC).**
The three cases register a real user against `InMemoryAuthBackend`,
mint a real session via `backend.create_session(&profile.user_id)`,
then `build_app` to exercise the full middleware stack. This is the
"AuthMethod::Session" path the reviewer prompt demanded — not a JWT
shortcut. Concretely:

- `mfa_enroll_returns_403_when_csrf_header_missing_with_session`
  (`:67-94`): sends session cookie pair, no `x-csrf-token` header,
  asserts 403 + the body mentions "csrf". `auth_middleware`
  successfully resolves the session (because the user/session exist
  in the backend) → installs `AuthContext { auth_method: Session }`
  → `csrf_middleware` reaches the cookie/header check → header
  missing → 403. This proves the chain ordering at runtime.
- `mfa_verify_enroll_path_returns_403_when_csrf_header_missing`
  (`:98-122`): same chain with the verify URL.
- `mfa_complete_login_succeeds_without_csrf_header` (`:126-167`):
  posts to `/api/v1/auth/login/mfa` with no cookie, no header,
  asserts `status != 403` and concretely `status == 401` (because the
  made-up `challenge_token` returns `InvalidToken`). This proves
  `csrf_middleware` is NOT layered on the cookie-less route.

**`crates/api/tests/seam_credential_write_path_validation.rs` (2 new
cases — `:178-217` missing-header, `:221-251` cookie/header mismatch).**
Both use JWT auth (`create_test_jwt()`), which is also covered by the
"cookie auth method" branch in `csrf_middleware` (`AuthMethod::Jwt`),
and assert exactly 403. The mismatch case feeds a different value to
the header than to the cookie — exercises the second `Err` branch in
`csrf_middleware:74-78`.

**Coverage gap NOT closed by this PR (acceptable):** there is no
*positive* test asserting that a PAT-authenticated request succeeds
against `/credentials/*` without sending the CSRF pair. That branch
of `csrf_middleware` (`AuthMethod::Pat | ApiKey` short-circuit at
`:42-46`) is unit-tested elsewhere implicitly (the access_e2e tests
use PAT auth on tenant routes and pass), but a dedicated assertion
"PAT write to credential route without CSRF headers is 2xx, not 403"
would make the exemption surface explicit. Recommend adding in a
follow-up; not a blocker because the implementation path is the same
unmodified code that already passes its own dedicated PAT tests.

**Cross-test sweep:** every other state-changing test against
`/api/v1/orgs/.../credentials/*` already uses helpers that attach the
CSRF pair: `auth_json` in `tests/credential_e2e.rs:129-139` and in
`tests/seam_credential_write_path_validation.rs:54-64`, plus
explicit `Request::builder()` blocks at `credential_e2e.rs:257-263`
(`DELETE`) and `:525-533` (`DELETE`). The only test that was missing
the headers was `e2e_oauth2_flow.rs::system_level_oauth_callback_post_route_is_disabled`,
which the PR correctly fixed.

### README accuracy — `crates/api/README.md:363-400`

The CSRF section's route table cross-matches the actual layering in
`crates/api/src/domain/mod.rs`:

| README claim | Code reality |
|---|---|
| `/api/v1/me/*` CSRF enforced | `:99-104` layers csrf+auth |
| `/api/v1/orgs/{org}/workspaces/{ws}/credentials/*` CSRF enforced | `:135-140` layers csrf+auth (credential_routes) |
| `/api/v1/orgs/{org}/*` (tenant) CSRF enforced | `:107-123` layers csrf+auth (tenant_routes) |
| `/api/v1/auth/mfa/{enroll,verify}` CSRF enforced | `:85-91` layers csrf+auth (auth_mfa_session_routes) |
| `/api/v1/auth/login/mfa` CSRF-exempt | `auth/routes.rs:30` on un-layered `auth_routes` |
| PAT / API-key requests exempt | `csrf.rs:41-46` short-circuit on `AuthMethod::{Pat,ApiKey}` |

All entries verified. The README's claim "Header contract — callers send
the matching token as `X-CSRF-Token: <value>`. The middleware compares
header against cookie byte-for-byte" matches `csrf.rs:55-79` (literal
`==` comparison of two `String`s).

Caveat (pre-existing, NOT this PR's regression): the README correctly
names the cookie `nebula_csrf` but the OpenAPI security-scheme
description in `crates/api/src/openapi/mod.rs:117` still says
`__Host-csrf`. Flag for follow-up — see Nit 4.

### `e2e_oauth2_flow.rs` adjustment

`crates/api/tests/e2e_oauth2_flow.rs:329-360`:
the test posts a system-credential OAuth callback to
`/api/v1/credentials/{id}/oauth2/callback` and asserts 410-GONE
(the system-level callback POST is disabled — provider-keyed callback
is the only supported shape). With `csrf_middleware` now layered on
`credential_routes`, a JWT-authenticated POST without the CSRF pair
is rejected with 403 *before* the handler runs. The test's 410-GONE
contract is therefore unreachable without the headers.

The two-line addition at `:344-345` (`x-csrf-token` + `cookie`) makes
the test reach its asserted production behavior. This is a legitimate
mitigation, not a cover-up: the underlying contract (system-level
callback POST → 410) is unchanged. The other tests in
`e2e_oauth2_flow.rs` and in `credential_e2e.rs` that POST/DELETE
against `/credentials/*` already carry the CSRF pair via the shared
`auth_json`/`auth_get` helpers, so no further test triage was
required. Verified by re-running the full nextest suite: 417/417.

## Open questions to flag to the human

1. **`MfaVerifyRequest`/`MfaVerifyResponse` deprecated-alias question.**
   The worker offered to restore the old union types as `#[deprecated]`
   re-exports if any external SDK generator has been published against
   them. Plan says API is pre-1.0 so a clean break is acceptable, but
   confirmation from product/release lead would lock the answer.

2. **Should `mfa_enroll` / `mfa_verify` advertise their security scheme
   in OpenAPI?** Right now they declare `security(())`. Updating both
   to `security(("bearer" = []), ("api_key" = []), ("csrf" = []))` is
   a one-line-each change and would make SDK contracts honest. Need
   confirmation that the team treats the served OpenAPI spec as the
   authoritative SDK input (the README implies yes).

3. **Should the drift-smoke `expected` array grow to include the MFA
   routes** (`/api/v1/auth/mfa/enroll`, `/api/v1/auth/mfa/verify`,
   `/api/v1/auth/login/mfa`)? Trivial to add; recommended.

## Recommendation

**Merge after addressing nits** — but the only nits that touch product
contract are #1 and #3. Concretely:

- **Should land in this PR (small, 1-2 LOC each):**
  - Nit 1: update `security(())` on `mfa_enroll` and `mfa_verify` to
    advertise bearer + csrf schemes.
  - Nit 2: silence the two new `cargo doc` warnings on
    `crates/api/src/domain/auth/routes.rs:13,38`.

- **Can be follow-up commits on `main` (not blocking PR1):**
  - Nit 3: add the three MFA routes to the drift-smoke expected list.
  - Nit 4: fix the pre-existing `__Host-csrf` description in
    `crates/api/src/openapi/mod.rs:117`.
  - Nit 5: add a global `.pi-lens/` ignore line.
  - Nit 6: deprecated-alias decision per open question 1.

If the team prefers absolute minimal-touch PR1 to keep the diff narrow,
all nits can be deferred — none of them are correctness-blocking and
the closing M10.5 doc-warning gate runs after PR3 anyway. The CSRF
enforcement contract this PR delivers is sound, complete, and well-tested.
