# Batch 4 — API & Sandbox Security Hardening

**Date:** 2026-04-14
**Status:** Design — not yet implemented
**Target implementer:** rust-senior
**Issues covered:** #319 (JWT fallback secret), #320 (CORS missing X-API-Key), #312 (webhook method gate), #316 (sandbox unbounded read_line)

---

## Shared concerns

Three of the four issues (#319, #320, #312) live entirely inside `nebula-api`. They touch different files (`config.rs`, `app.rs`, `webhook/transport.rs`) with **no shared types** — their only coupling is the crate boundary. #316 is in `nebula-sandbox` and is completely independent (different crate, different severity triage, different reviewer lens — security-lead should second-review it).

The only cross-issue contract to get right is that **#319's startup validation must not break #320's/#312's regression tests**. The integration test file `crates/api/tests/integration_tests.rs` currently calls `ApiConfig::default()` ~22 times. After #319, `default()` no longer produces a usable runtime config — tests must either call a new `ApiConfig::for_test()` helper (preferred) or set an explicit test-only secret.

## API breakage scope

| Item | Before | After | Break |
|---|---|---|---|
| `ApiConfig::default()` | returns runtime-usable config with fake secret | **removed** (no `impl Default`) | **yes** — 22 test call-sites, 1 README, 1 example |
| `ApiConfig::from_env()` | `Result<Self, Box<dyn Error>>` | `Result<Self, ApiConfigError>` | yes (typed error) |
| new `ApiConfig::for_test()` | — | `#[cfg(any(test, feature = "test-util"))] fn for_test() -> Self` | additive |
| new `ApiConfigError` | — | typed enum, `#[non_exhaustive]` | additive |
| `WebhookTransport::router` | `.route(&route, any(webhook_handler))` | `.route(&route, post(webhook_handler))` | behavior (non-POST → 405) |
| `build_cors_layer` | header list omits `x-api-key` | adds `HeaderName::from_static("x-api-key")` | additive |
| `ProcessSandbox` / `PluginHandle::recv_envelope` | unbounded `read_line` | capped via `take + read_until` | additive internal |
| new `SandboxError::PluginLineTooLarge` (or equivalent on `ActionError` path) | — | `#[error("plugin line exceeded cap: {cap} bytes")]` | additive |

Nothing here is load-bearing on downstream crates — `nebula-api` is at the top of the layer stack, and `nebula-sandbox` is consumed only by `nebula-runtime`.

## PR split decision

**Two PRs, not one.**

1. **PR-A — `fix(api): security hardening batch 4`** → #319 + #320 + #312. Single crate (`nebula-api`), single reviewer lens (API + web security), single integration-test file touched. Bundling matches the "one PR over churn" preference.
2. **PR-B — `fix(sandbox): cap plugin line length`** → #316. Different crate, different reviewer (security-lead should co-review), different test harness. Bundling it with PR-A would muddle the blast-radius story and delay the API fix behind sandbox review.

Merge order does not matter; the two PRs do not share files or types.

---

# Issue #319 — API JWT fallback secret (HIGH / security)

## Root cause
`ApiConfig::default()` hardcodes `jwt_secret: "dev-secret-change-in-production"` and `ApiConfig::from_env()` silently falls back to the same literal when `API_JWT_SECRET` is unset — a publicly-known HS256 key signs every token until someone notices.

## Fix strategy

**Principle: the illegal state (empty / short / known-dev secret in production mode) must be unrepresentable at the type that the middleware holds.**

### 1. Delete the fallback

- Remove `impl Default for ApiConfig` entirely. There is no correct default for a field whose wrong value is a full auth bypass.
- Remove the `unwrap_or_else(|_| "dev-secret-change-in-production"...)` line in `from_env`.

### 2. Introduce a typed secret newtype in `crates/api/src/config.rs`

```rust
/// Validated HS256 signing key. Construction is the ONLY place the length /
/// entropy / known-bad-value checks live. Any `JwtSecret` in hand is valid.
#[derive(Clone)]
pub struct JwtSecret(Arc<str>);

impl JwtSecret {
    /// Minimum length for HS256. RFC 7518 §3.2 says "a key of the same size
    /// as the hash output"; for HS256 that's 32 bytes. We require 32+.
    pub const MIN_BYTES: usize = 32;

    pub fn new(raw: impl Into<Arc<str>>) -> Result<Self, ApiConfigError>;
    pub fn as_bytes(&self) -> &[u8];
}

impl std::fmt::Debug for JwtSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwtSecret([REDACTED])")
    }
}
```

`JwtSecret::new` rejects, in order:
1. length `< MIN_BYTES` (32)
2. the literal `"dev-secret-change-in-production"` (belt-and-suspenders: even if someone leaks it back in via env, startup fails)
3. any value whose Shannon-entropy estimate is implausibly low — **deferred; do NOT implement in this PR.** Length + known-bad is enough. Entropy heuristics cause false positives.

The `ApiConfig.jwt_secret` field becomes `JwtSecret`, and `AppState.jwt_secret: Arc<str>` is replaced by `AppState.jwt_secret: JwtSecret`. The middleware calls `state.jwt_secret.as_bytes()` — identical call shape.

### 3. Typed error

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiConfigError {
    #[error("API_JWT_SECRET is required in production mode (NEBULA_ENV={0})")]
    MissingJwtSecret(String),

    #[error("API_JWT_SECRET is too short ({got} bytes, minimum {min})")]
    JwtSecretTooShort { got: usize, min: usize },

    #[error("API_JWT_SECRET matches the well-known development placeholder — refusing to start")]
    JwtSecretIsDevPlaceholder,

    #[error("API_BIND_ADDRESS invalid: {0}")]
    BindAddress(#[source] std::net::AddrParseError),

    #[error("API_{var} invalid: {source}")]
    ParseInt { var: &'static str, #[source] source: std::num::ParseIntError },

    #[error("API_{var} invalid: {source}")]
    ParseBool { var: &'static str, #[source] source: std::str::ParseBoolError },
}
```

This replaces the `Box<dyn Error>` return in `from_env`.

### 4. Dev-mode behavior

**No `--dev` flag.** The project already has `NEBULA_ENV` (see `crates/core/src/constants.rs:110`, `crates/log/src/telemetry/sentry.rs:16`). Reuse it.

```rust
impl ApiConfig {
    pub fn from_env() -> Result<Self, ApiConfigError> {
        let env_mode = std::env::var("NEBULA_ENV").unwrap_or_else(|_| "development".to_string());
        let is_dev = matches!(env_mode.as_str(), "development" | "dev" | "local");

        let jwt_secret = match std::env::var("API_JWT_SECRET") {
            Ok(s) => JwtSecret::new(s)?,                          // always validate
            Err(_) if is_dev => JwtSecret::generate_ephemeral(),  // random per-process, logs a warning ONCE
            Err(_) => return Err(ApiConfigError::MissingJwtSecret(env_mode)),
        };
        // ...
    }
}
```

- `JwtSecret::generate_ephemeral()` — uses `rand::rngs::OsRng` to produce 32 random bytes, hex-encodes them, logs **one** `warn!` line: `"API_JWT_SECRET unset; generated ephemeral secret for NEBULA_ENV=development. Tokens will be invalidated on restart."`
- In `production`/`staging`/anything not in the dev allow-list, a missing `API_JWT_SECRET` is a hard error at startup. **Fail loud, fail early, fail before binding the socket.** No log-and-continue.

### 5. Test helper

```rust
#[cfg(any(test, feature = "test-util"))]
impl ApiConfig {
    /// Build a config suitable for integration tests. Uses a fixed, known,
    /// obviously-test-only secret that is REJECTED by `JwtSecret::new` in
    /// production paths. Only reachable when the `test-util` feature is on.
    pub fn for_test() -> Self { /* bypasses JwtSecret::new via a private ctor */ }
}
```

The `test-util` feature gate is what keeps production builds from ever touching the weak secret. A plain `#[cfg(test)]` is not enough because `tests/integration_tests.rs` is compiled as an integration-test binary, not under `#[cfg(test)]` of the library crate.

## Call-site impact

- `crates/api/tests/integration_tests.rs` — all 22 `ApiConfig::default()` → `ApiConfig::for_test()`. Add `test-util` to `dev-dependencies` features of the crate's own `[dev-dependencies]` self-reference, or gate differently. (rust-senior: pick the idiomatic axum/tower pattern already in use elsewhere in the workspace.)
- `crates/api/examples/simple_server.rs` — replace `ApiConfig::default()` with `ApiConfig::from_env().expect("API_JWT_SECRET required")` and update the doc comment to say "set `API_JWT_SECRET` before running".
- `crates/api/README.md` — same change, docs snippet.
- `crates/api/src/state.rs:25` — change field type `jwt_secret: Arc<str>` → `jwt_secret: JwtSecret`.
- `crates/api/src/middleware/auth.rs:95` — `state.jwt_secret.as_bytes()` already works, no change.
- `crates/api/examples/simple_server.rs:29` — `api_config.jwt_secret.clone()` already works (JwtSecret is Clone).

## Test strategy (named)

- `config::tests::from_env_rejects_missing_secret_in_production` — sets `NEBULA_ENV=production`, unsets `API_JWT_SECRET`, asserts `Err(MissingJwtSecret)`.
- `config::tests::from_env_rejects_short_secret` — `API_JWT_SECRET="short"`, asserts `JwtSecretTooShort { got: 5, min: 32 }`.
- `config::tests::from_env_rejects_dev_placeholder` — sets the literal, asserts `JwtSecretIsDevPlaceholder`.
- `config::tests::from_env_generates_ephemeral_in_dev` — `NEBULA_ENV=development`, unsets secret, asserts `Ok` and two successive calls produce different secrets.
- `config::tests::jwt_secret_debug_is_redacted` — formats `JwtSecret`, asserts output contains `[REDACTED]` and not the key.

All use `temp-env` or the existing workspace pattern for serial env-var manipulation — **not** `std::env::set_var` directly under parallel nextest execution.

---

# Issue #320 — CORS missing `X-API-Key` (MEDIUM)

## Root cause
`build_cors_layer` in `crates/api/src/app.rs:107-112` lists `content-type`, `authorization`, `accept`, `x-request-id` in `allow_headers` but omits `x-api-key`, so browser preflight for API-key auth fails before reaching middleware.

## Fix strategy

One-line addition in `crates/api/src/app.rs` `build_cors_layer`:

```rust
.allow_headers([
    header::CONTENT_TYPE,
    header::AUTHORIZATION,
    header::ACCEPT,
    header::HeaderName::from_static(X_REQUEST_ID),
    header::HeaderName::from_static("x-api-key"),    // new
])
```

Reuse `middleware::auth::X_API_KEY` rather than hardcoding the string — wire `pub static X_API_KEY` out of the auth module (it is already `static` there at `auth.rs:22`) and reference it:

```rust
.allow_headers([
    header::CONTENT_TYPE,
    header::AUTHORIZATION,
    header::ACCEPT,
    header::HeaderName::from_static(X_REQUEST_ID),
    crate::middleware::auth::X_API_KEY.clone(),
])
```

This closes the "auth module says X, CORS says Y" contract drift at the type level: there is now exactly one place the header name lives.

**No conditional logic** ("only add when API keys are configured"). The CORS policy must match the *protocol surface*, not the *current tenant config*. An admin enabling API keys later should not need a server restart for preflight to work.

## Call-site impact

None. Single-file change.

## Test strategy (named)

- `cors::tests::preflight_allows_x_api_key` — builds the app, sends `OPTIONS /api/v1/workflows` with `Origin: https://app.example`, `Access-Control-Request-Method: GET`, `Access-Control-Request-Headers: x-api-key`, asserts `200` and `Access-Control-Allow-Headers` response header contains `x-api-key` (case-insensitive).
- `cors::tests::preflight_allows_authorization` — regression lock for existing header.

---

# Issue #312 — Webhook accepts all methods (MEDIUM / security)

## Root cause
`WebhookTransport::router` at `crates/api/src/webhook/transport.rs:197` uses `.route(&route, any(webhook_handler))`. The docs at line 1–22 say `POST /{prefix}/{trigger_uuid}/{nonce}` but every method dispatches through the handler.

## Fix strategy

Replace `any` with `post` at the routing boundary — the framework handles the 405, we do not hand-roll method checks inside the handler.

```rust
use axum::routing::post;            // drop `any`

pub fn router(&self) -> Router {
    let route = format!(
        "{prefix}/{{trigger_uuid}}/{{nonce}}",
        prefix = self.inner.config.path_prefix,
    );
    Router::new()
        .route(&route, post(webhook_handler))    // was any(webhook_handler)
        .layer(DefaultBodyLimit::max(self.inner.config.body_limit_bytes))
        .with_state(self.clone())
}
```

Axum returns `405 Method Not Allowed` with a correct `Allow: POST` header automatically for non-POST requests on a `post`-routed path.

### Why not "accept all methods and gate in handler"

Defense-in-depth says the *routing layer* is the right boundary: middlewares, WAF rules, and metrics that scope on "which route was hit" never even count the offending request as a webhook dispatch. Gating in the handler means the oneshot channel, rate limiter, and routing-map lookup all run first. Cheaper *and* more correct to drop at the router.

### Handler parameter cleanup

The handler currently takes `method: Method` (line 252) and passes it to `WebhookRequest::try_new`. Keep the extractor — POST has a method too and the downstream action still needs to see `Method::POST`. No code change inside the handler body.

## Call-site impact

- `crates/api/src/webhook/transport.rs` — import change, one-line routing change.
- `crates/api/tests/webhook_transport_integration.rs` — add regression tests (see below). No existing test breaks because they all use POST.

## Test strategy (named)

- `webhook_transport::tests::rejects_get_with_405`
- `webhook_transport::tests::rejects_put_with_405`
- `webhook_transport::tests::rejects_delete_with_405`
- `webhook_transport::tests::rejects_patch_with_405`
- `webhook_transport::tests::post_still_dispatches` — regression lock.

All assert `response.status() == 405` AND `response.headers().get("allow").unwrap() == "POST"`.

---

# Issue #316 — ProcessSandbox unbounded `read_line` (HIGH / security)

## Root cause
Both the handshake path (`crates/sandbox/src/process.rs:333`) and the envelope path (line 126) call `BufReader::read_line` with no length cap, so a newline-starved or gigabyte-sized line from an untrusted plugin grows the receive buffer without bound until allocator failure / OOM kill.

## Fix strategy

**Cap both paths with `AsyncBufReadExt::take(limit).read_until(b'\n', &mut buf)` and reject-with-typed-error on overflow.** Do not truncate silently; a truncated JSON envelope that still happens to parse is worse than a clean error.

### Limits

Two separate constants at the top of `crates/sandbox/src/process.rs`:

```rust
/// Max bytes accepted for the plugin handshake line.
/// A handshake is a short socket/pipe address string plus protocol version.
/// 4 KiB is ~40x more headroom than any realistic handshake.
const HANDSHAKE_LINE_CAP: usize = 4 * 1024;

/// Max bytes accepted for a single runtime envelope line (JSON + trailing `\n`).
/// 1 MiB starting point — matches `nebula-action`'s default body limit
/// (see `WebhookTransportConfig::body_limit_bytes`). If real plugins need
/// more, make this configurable via `ProcessSandbox::new` later.
const ENVELOPE_LINE_CAP: usize = 1024 * 1024;
```

**Note for rust-senior on the 1 MiB number:** raise this ceiling only if a concrete plugin exceeds it; do not preemptively bump to "be safe". A too-high cap defeats the purpose.

### Read primitive

The `BufRead::read_line` call is the problem — it internally loops on `fill_buf` with no length check. Replace with `take(cap).read_until(b'\n', ...)`, then detect overflow by "did we hit `cap` without finding a newline":

```rust
// In PluginHandle::recv_envelope
async fn recv_envelope(&mut self) -> Result<PluginToHost, SandboxError> {
    self.line_buf.clear();
    let mut bytes = self.line_buf.as_bytes().to_vec();     // or use Vec<u8> field
    let n = (&mut self.reader)
        .take(ENVELOPE_LINE_CAP as u64 + 1)                // +1 to distinguish "full" from "overflow"
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(SandboxError::PluginTransport)?;

    if n == 0 {
        return Err(SandboxError::PluginClosed);
    }
    if n > ENVELOPE_LINE_CAP || !bytes.ends_with(b"\n") {
        return Err(SandboxError::PluginLineTooLarge {
            cap: ENVELOPE_LINE_CAP,
        });
    }
    // parse bytes as UTF-8 → JSON
}
```

**Field type change:** `PluginHandle::line_buf` becomes `Vec<u8>` instead of `String` so we can use `read_until` without a UTF-8 validation in the hot path. Parse JSON from `&[u8]` directly via `serde_json::from_slice`. Trim the trailing `\n` before parsing.

### Error variant

Introduce a local `SandboxError` if one does not already exist, or add variants to the existing path. The envelope path currently returns `ActionError::fatal(String)`; that is too coarse — the caller cannot distinguish "plugin DoS attempt" from "plugin serialization bug" for metrics/alerting.

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SandboxError {
    #[error("plugin envelope line exceeded cap of {cap} bytes — possible DoS or protocol violation")]
    PluginLineTooLarge { cap: usize },

    #[error("plugin closed transport without sending a response envelope")]
    PluginClosed,

    #[error("plugin handshake line exceeded cap of {cap} bytes")]
    HandshakeLineTooLarge { cap: usize },

    #[error("plugin transport I/O error")]
    PluginTransport(#[source] std::io::Error),

    #[error("plugin sent malformed envelope")]
    MalformedEnvelope(#[source] serde_json::Error),
}
```

If wiring a new error type through the `SandboxRunner` trait is too invasive for this PR, rust-senior may instead funnel `PluginLineTooLarge` through `ActionError::fatal` **with a structured sentinel string** (`"plugin_line_too_large:cap=..."`) — but the preferred path is the typed variant.

### Connection invalidation on overflow

On `PluginLineTooLarge`, the handle is **not reusable** — we do not know where we are in the stream. `dispatch_envelope` already clears the handle on any `try_dispatch` error (`process.rs:219`), so the existing retry-once path correctly forces a respawn. **Verify this invariant in a test** — the dispatch loop must not infinite-loop by oversized-retrying forever. The existing "one retry" ceiling in `dispatch_envelope` already guarantees this, but the test codifies it.

### Handshake path

Same primitive, separate cap:

```rust
// replace read_line at process.rs:333
let mut handshake_buf: Vec<u8> = Vec::with_capacity(256);
let read_result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
    (&mut stdout_reader)
        .take(HANDSHAKE_LINE_CAP as u64 + 1)
        .read_until(b'\n', &mut handshake_buf)
        .await
})
.await;
// ... same timeout/empty/overflow handling
```

### Stderr drainer

`drain_plugin_stderr` at line 431 also uses `read_line` but already has an implicit bound via `sanitize_plugin_string` (truncates at 1024 chars AFTER the read). That's still unbounded memory on the read itself — **fix it the same way** with a small cap (say `STDERR_LINE_CAP = 8 * 1024`), silently discard anything past the cap (stderr is log output, not protocol; truncation is acceptable here because the contract already says "1024 chars max in the log line"). This is the single place where truncation is OK; **everywhere else, reject loudly**.

## Call-site impact

- `crates/sandbox/src/process.rs` — internal only. `PluginHandle.line_buf` type change. Two `read_line` sites replaced. Optional stderr drainer cap.
- `SandboxRunner` trait — no signature change (the error still flows through `ActionError` unless rust-senior chooses the typed-error path).
- No callers outside `crates/sandbox` touch `PluginHandle` directly.

## Test strategy (named)

New test module in `crates/sandbox/tests/` (or `src/process.rs` `#[cfg(test)]` submodule):

- `process_sandbox::tests::rejects_oversized_envelope_line` — spawn a mock plugin (fake stdout/pipe) that emits `ENVELOPE_LINE_CAP + 1` bytes without a newline, assert `Err(PluginLineTooLarge)` and that sandbox's `handle` is cleared afterwards.
- `process_sandbox::tests::rejects_newline_starved_handshake` — mock plugin emits `HANDSHAKE_LINE_CAP + 1` bytes of `b'A'` on stdout, no newline, assert `HandshakeLineTooLarge`.
- `process_sandbox::tests::accepts_envelope_exactly_at_cap` — boundary: `ENVELOPE_LINE_CAP - 1` body bytes + `\n` = exactly `ENVELOPE_LINE_CAP`, must parse successfully. Guards the off-by-one in `take(cap + 1)`.
- `process_sandbox::tests::oversized_line_does_not_infinite_retry` — spawn a plugin that always oversizes, call `dispatch_envelope`, assert the function returns after exactly two attempts (handle-clear + one retry) rather than looping.
- `process_sandbox::tests::stderr_drain_bounded_on_newline_starved_stream` — emit 1 MB of stderr without a newline, assert the drain task memory stays flat and does not OOM.

Mock plugins for these tests: use `tokio::io::duplex` to build in-process `AsyncRead/AsyncWrite` pairs, bypassing actual process spawn. The `PluginHandle::new` signature takes a `Child` for `kill_on_drop`; extract a helper constructor gated on `#[cfg(test)]` that skips the child requirement, or use a sentinel `Command::new("true")` if cross-platform test infra is tricky — rust-senior's call.

---

# Open questions / non-scope

- **Signature verification audit.** The JWT middleware at `crates/api/src/middleware/auth.rs:99` already calls `jsonwebtoken::decode` which DOES validate the HS256 signature — this is not a "presence check only" bug. The `validate_exp = true` flag is also set. No scope expansion needed. (Confirmed by reading the file, not by inference.)
- **Length-prefix framing for plugin transport.** The issue body suggests "prefer length-delimited framing over newline-delimited JSON". That is a bigger protocol change affecting `nebula-plugin-sdk` and every plugin binary in the wild. **Out of scope for #316.** Track as a separate design doc if it's worth doing.
- **JWT algorithm rotation (HS256 → RS256/EdDSA).** Out of scope. Current fix is strictly about the signing *key*, not the signing *algorithm*.
- **API key storage.** Static API keys via `API_KEYS` env var is fine for now; moving to a database-backed store with hashing is a separate issue.

---

# Handoff

- **rust-senior** — implement both PRs; critical sections are the `JwtSecret` newtype construction in PR-A and the `take + read_until` overflow discrimination in PR-B.
- **security-lead** — second-review PR-B specifically for the cap values and the connection-invalidation invariant. Second-review PR-A for the dev-mode ephemeral secret path (make sure it cannot leak into production by accident — e.g. `NEBULA_ENV=developmnt` typo should NOT silently fall through).
