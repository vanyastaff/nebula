# Environment-variable handling — cross-crate audit & refactor handoff

**Owner**: TBD on pickup
**Status**: Handoff / Plan (audit complete, **no implementation work done**)
**Branch of record**: `claude/crates-env-refactor-HcuWE`
**Author of plan**: agent handoff — env-handling sweep
**Date**: 2026-05-29

> **Scope discipline**: this is an audit + refactor plan only. No code in
> `crates/` or `apps/` was changed. Every finding below cites the
> **concrete file:line** it was observed at so the implementer can verify
> against the live tree before acting. Re-validate each finding before
> starting — env-reading sites move. Implementation should be split into
> the phases in §4 and land as separate conventional-commit PRs, not one
> mega-refactor.

---

## 1. Why this exists

Environment-variable handling is currently **scattered, duplicated, and
semantically inconsistent** across the workspace. There is no single
reader, no shared parsing conventions, and no real registry — despite
`.env.example` claiming one exists. The same variable is read in multiple
places with **different failure semantics**, and two crates hand-roll the
same unsafe test harness. This handoff inventories the surface, names the
problems, and proposes a staged refactor that stays inside the repo's
layer discipline (`deny.toml` `[wrappers]`).

---

## 2. Current-state inventory

### 2.1 Env-var registry (what is actually read, and where)

Grouped by reader. `file:line` is the read site in non-test code as of
this audit.

**Environment metadata / identity**

| Var | Read at | Notes |
|-----|---------|-------|
| `NEBULA_ENV` | `api/src/config/mod.rs:225`, `log/src/config/fields.rs:29`, `log/src/telemetry/sentry.rs:18`, `apps/server/src/compose.rs:448` | **Read in 4 independent places**, each with its own default (`"production"` vs `unwrap_or_default()` vs sentry fallback chain). No single source of truth. |
| `NEBULA_SERVICE` | `log/src/config/fields.rs:28` | |
| `NEBULA_INSTANCE` | `log/src/config/fields.rs:33` | |
| `NEBULA_REGION` | `log/src/config/fields.rs:34` | |
| `NEBULA_VERSION` | `log/src/config/fields.rs:30` | |

**Storage**

| Var | Read at | Notes |
|-----|---------|-------|
| `DATABASE_URL` | `storage/src/pg/{control_queue,oauth_state,org,pat,session,user,verification_token}.rs` (**7 sites**) + `apps/server/src/compose.rs:477,522` | Each site re-implements the read with **different failure semantics**: most `match … { Err => … }`, but `org.rs:290` uses `.ok()?` (silently returns `None`), while compose maps to `TransportInitError`. Same var, ~3 different error contracts. |
| `NEBULA_CRED_MASTER_KEY` | `storage/src/credential/key_provider.rs:199` (const `ENV_VAR` at :180) | Has a `DEV_PLACEHOLDER` sentinel. Good local pattern — name lives in a const. |

**HTTP / API** (all `API_`-prefixed; canonical parsers in `api/src/config/env.rs`)

| Var | Read at |
|-----|---------|
| `API_JWT_SECRET`, `API_BIND_ADDRESS`, `API_REQUEST_TIMEOUT`, `API_MAX_BODY_SIZE`, `API_CORS_ORIGINS`, `API_ENABLE_COMPRESSION`, `API_ENABLE_TRACING`, `API_RATE_LIMIT`, `API_KEYS`, `API_PUBLIC_URL`, `API_REQUEST_ID_HEADER` | `api/src/config/mod.rs:228–304` |
| `API_AUTH_BACKEND`, `API_IDEMPOTENCY_BACKEND`, `API_IDEMPOTENCY_{TTL_SECS,MAX_ENTRIES,MAX_REQUEST_BODY_BYTES,MAX_RESPONSE_BODY_BYTES,SWEEP_INTERVAL_SECS}` | `api/src/config/mod.rs:356–379` + `sub.rs` |
| `API_SMTP_{HOST,PORT,USERNAME,PASSWORD,FROM,TLS_MODE}` | `api/src/config/mod.rs:451–507` |
| `API_AUTH_OAUTH_<PROVIDER>_{CLIENT_ID,CLIENT_SECRET,DISCOVERY_URL,AUTHORIZE_URL,TOKEN_URL,USERINFO_URL,VERIFIED_EMAILS_URL,JWKS_URL,SCOPES}` | `api/src/config/oauth.rs:49,286–311` (dynamic, per-provider prefix) |

**Webhook transport**

| Var | Read at |
|-----|---------|
| `WEBHOOK_BASE_URL` | `apps/server/src/transport.rs:110` |
| `WEBHOOK_BIND_ADDRESS`, `WEBHOOK_BOOTSTRAP_FROM_STORAGE` | `apps/server/src/compose.rs` (documented in `.env.example`) |

**Logging**

| Var | Read at | Notes |
|-----|---------|-------|
| `NEBULA_LOG`, `RUST_LOG` | `log/src/config/env.rs:46,49` | `NEBULA_LOG` overrides `RUST_LOG`. |
| `NEBULA_LOG_FORMAT` | `log/src/config/env.rs:54` | parsed via `parse_format` |
| `NEBULA_LOG_{TIME,SOURCE,COLORS}` | `log/src/config/env.rs:61–69` | parsed via `parse_bool` |

**Telemetry (OTEL / Sentry)**

| Var | Read at | Notes |
|-----|---------|-------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `log/src/telemetry/otel.rs:69` **and** `api/src/telemetry_init.rs:53,319` | **Same var read in two crates** independently. |
| `OTEL_SERVICE_NAME` | `api/src/telemetry_init.rs:57,305` | Overlaps conceptually with `NEBULA_SERVICE` (log). |
| `NEBULA_METRICS_OTLP_INTERVAL_SECS` | `api/src/telemetry_init.rs:61,324` | |
| `SENTRY_DSN`, `SENTRY_ENV`, `SENTRY_RELEASE`, `SENTRY_TRACES_SAMPLE_RATE` | `log/src/telemetry/sentry.rs:11–25` | `SENTRY_ENV` falls back to `NEBULA_ENV`. |

**Plugin protocol**

| Var | Read at |
|-----|---------|
| `HOST_TO_PLUGIN_FRAME_CAP_ENV` (const) | `plugin-sdk/src/lib.rs:337` |
| `ENV_SOCKET_ADDR`, `ENV_SOCKET_KIND` (consts) | `plugin-sdk/src/transport.rs:82–83` |

### 2.2 Existing parsing helpers (the duplication)

| Crate | Helpers | Convention |
|-------|---------|-----------|
| `api` (`config/env.rs`) | `parse_u64_env`, `parse_positive_u64_env`, `parse_usize_env`, `parse_bool_env` | `API_`-prefix auto-prepended; typed `ApiConfigError` (`ParseInt`/`ParseEnum`/`ZeroValue`); **fail-closed** on bad input. |
| `log` (`config/env.rs`) | `parse_bool`, `parse_format` | free functions; **fail-open / lenient** (no error path). |
| `storage`, `apps/server`, `api/oauth.rs` | none — inline `std::env::var(...).unwrap_or_default()/.ok()?/.parse()` | ad hoc per call site. |

### 2.3 Test-harness duplication

| Crate | Harness | Location |
|-------|---------|----------|
| `api` | `env_lock()` (`OnceLock<Mutex<()>>`) + `clear_env()` with a **hardcoded 24-key list** | `api/src/config/env.rs:58–104` (also used by `mod.rs`, `oauth.rs`, `sub.rs`) |
| `log` | `ENV_LOCK` (`LazyLock<Mutex<()>>`) + inline `unsafe set_var` | `log/tests/config_precedence.rs:6–30`, `log/examples/sentry_test.rs` |
| `storage` | inline `set_var`/`remove_var` | `storage/tests/credential_env_provider.rs` |

Each re-declares the same `#[allow(unsafe_code, reason = "env::{set_var,
remove_var} are unsafe under edition 2024")]` justification. The two
locks are **independent** — they only serialize within a single test
binary, which is correct under nextest's per-binary processes, but the
pattern is copy-pasted rather than shared. `api`'s `clear_env()` key list
is **manually maintained** and will silently drift from the real registry.

---

## 3. Findings (ranked)

**F1 — No real single source of truth (severity: high).**
`.env.example` claims "canonical env-var registry in
`crates/api/src/config/env.rs`", but that file only contains the `API_`
typed parsers — it has no entry for `NEBULA_*`, `OTEL_*`, `SENTRY_*`,
`DATABASE_URL`, or the plugin vars. The "registry" is aspirational. Three
docs (`.env.example`, `deploy/.env.example`, per-crate READMEs) and the
code can drift independently with no mechanical check.

**F2 — Same var, divergent failure semantics (severity: high).**
`DATABASE_URL` is read at 9 sites with at least 3 different contracts
(error, `None` via `.ok()?` at `org.rs:290`, transport-error). `NEBULA_ENV`
is read at 4 sites with 3 different defaults. An operator who mis-sets one
of these gets inconsistent behavior depending on which path runs first.

**F3 — Inconsistent bool parsing (severity: medium).**
`log::parse_bool` (`config/env.rs:25`) treats *anything not in
`{"0","false","FALSE","False"}`* as `true` and never errors.
`api::parse_bool_env` (`config/env.rs:42`) accepts
`true/1/yes/on` and `false/0/no/off` and **errors** on anything else.
`NEBULA_LOG_COLORS=auto` (documented in `.env.example`!) would be parsed
as `true` by log's lenient rule — the `auto` semantics are silently lost.
Same conceptual operation, two incompatible contracts.

**F4 — Duplicate typed parsers (severity: medium).**
`u64`/`usize`/`bool` parsing is implemented in `api`, partially in `log`,
and hand-rolled everywhere else. No shared `parse::<T>` with a uniform
error.

**F5 — Test harness copy-paste + drift risk (severity: medium).**
Two independent env locks; `clear_env()`'s key list is hand-maintained;
the `unsafe` justification is re-declared in every test module.

**F6 — Conceptual overlap between vars (severity: low).**
`OTEL_SERVICE_NAME` vs `NEBULA_SERVICE`; `SENTRY_ENV` vs `NEBULA_ENV`
(already chained); `OTEL_EXPORTER_OTLP_ENDPOINT` read independently in
`log` and `api`. Worth a deliberate precedence decision rather than
incidental fallback chains.

---

## 4. Refactor plan (staged)

Two viable shapes. **Recommendation: do Phase 0–1 unconditionally
(no new crate, no layer change), then evaluate Phase 2 (shared crate)
against the churn it removes.**

### Phase 0 — Make the registry real (docs + test, no behavior change)

- Promote a single authoritative registry. Cheapest form: a
  `crates/<x>/src/.../env_vars.rs` const table per owning crate, plus a
  workspace-level doc table (this file's §2.1 can seed
  `docs/ENV.md`). Stronger form: a `const &[&str]` exported per crate that
  `.env.example` and `clear_env()` are checked against in a test.
- Add a test that fails if `.env.example` references a var no crate reads,
  or a crate reads a var absent from `.env.example`. This alone kills F1's
  drift class.
- **Blast radius**: docs + 1–2 tests. No runtime change. Conventional
  commit: `docs(env)` / `test(env)`.

### Phase 1 — In-place consolidation (no new crate; stays in layer map)

1. **Unify bool parsing semantics (F3).** Pick one contract. Recommended:
   adopt `api`'s strict `true/1/yes/on | false/0/no/off`, **plus** an
   explicit `auto` arm where a tri-state is needed (`NEBULA_LOG_COLORS`).
   Decide fail-open vs fail-closed *per consumer* but parse with one
   function. **Behavior change** — call out in PR body; add RED tests for
   `auto` and for previously-lenient inputs.
2. **Collapse `DATABASE_URL` reads (F2).** Add one
   `storage`-internal helper `fn database_url() -> Result<String, …>` with
   a single error contract; replace the 7 pg-module sites + audit
   `org.rs:290`'s `.ok()?` (is silent `None` intended there? — **open
   question, see §6**). compose keeps its own transport-error mapping but
   calls the same reader.
3. **Single `NEBULA_ENV` reader (F2/F6).** One function returning a typed
   `enum Environment { Dev, Staging, Production }` with one default;
   `api`, `log`, `sentry`, `compose` consume it. (Lands naturally in the
   shared crate if Phase 2 proceeds; otherwise put it in `log` since it's
   cross-cutting metadata, or `core`.)
- **Blast radius**: `storage` (~7 sites → 1 helper), `api`/`log` bool
  call sites, one new `NEBULA_ENV` reader. Conventional commits:
  `refactor(storage)`, `refactor(log)`, `refactor(api)`.

### Phase 2 — Extract `nebula-env` (optional; touches layer map + deny.toml)

Only if Phase 1 shows the shared surface is worth a crate. Shape:

- New **cross-cutting** crate `crates/env` → `nebula-env` (same tier as
  `log`/`error`/`metrics` in the Layered Dependency Map — importable at
  any level). Depends only on `std` + `nebula-error` + `serde` (no tokio).
- API surface:
  - `EnvReader` / free fns: `var(name) -> Result<String, EnvError>`,
    `var_opt`, `parse::<T: FromStr>`, `bool`, `list` (comma/whitespace
    split — reuses `oauth.rs:49`'s splitter), `with_prefix("API")`.
  - `EnvError { var: &'static str, kind: EnvErrorKind }` integrating with
    `nebula-error` (variant types per F4); each consumer maps it into its
    own error (`ApiConfigError`, `ProviderError`, …) at the boundary.
  - A `testing` feature exposing the env lock + an RAII
    `ScopedEnv` guard (set on construct, restore on drop) to replace the
    three hand-rolled harnesses (F5) and centralize the one `unsafe` block
    behind a safe API.
- **deny.toml**: add `nebula-env` to `[wrappers]` with its exact consumer
  allowlist (api, log, storage, apps/server, plugin-sdk as needed).
  Update the Layered Dependency Map table in `CLAUDE.md`.
- An ADR is warranted (next free number is **0086** — `docs/adr/` tops out
  at `0085`) documenting the cross-cutting placement and the
  prefix/parse/precedence conventions.
- **Blast radius**: 1 new crate, `Cargo.toml` workspace member, `deny.toml`
  wrapper entry, `CLAUDE.md` layer-map row, ADR-0086, plus migration of
  the call sites above. Non-trivial — sequence last.

### Phase 3 — Test-harness consolidation (folds into Phase 2 or stands alone)

- Replace `api::env_lock`/`clear_env` and `log::ENV_LOCK` with the shared
  `nebula-env::testing::ScopedEnv` (if Phase 2) or a single shared
  test-util module. Drive `clear_env()` off the const registry from
  Phase 0 so it can't drift.

### Phase 4 — Generate/validate `.env.example` (closes F1 mechanically)

- A small xtask/test that renders `.env.example` groups from the registry
  consts, or asserts equality. Wire into `task dev:check`.

---

## 5. Sequencing recommendation

1. **Phase 0** — registry + drift test. Standalone, zero risk, unblocks
   everything else. Land first.
2. **Phase 1** — in-place consolidation. Each sub-item is its own PR;
   F3 (bool) and F2 (`DATABASE_URL`) are independent and parallelizable.
3. **Decision gate** — re-assess whether the remaining duplication
   justifies Phase 2. If the team wants the test-harness win (F5) and the
   single `parse::<T>` (F4), proceed; otherwise stop after Phase 1.
4. **Phase 2 + 3** — shared crate + harness, with ADR-0086 and the
   `deny.toml`/`CLAUDE.md` updates in the same PR.
5. **Phase 4** — generated `.env.example`, wired into the pre-PR gate.

---

## 6. Open questions for the owner

- **Q1**: Is `storage/src/pg/org.rs:290`'s `DATABASE_URL` → `.ok()?`
  (silent `None`) intentional (e.g. optional integration test path) or a
  latent bug? Phase 1.2 needs this answered before unifying the contract.
- **Q2**: Do we want a typed `Environment` enum (`Dev/Staging/Production`)
  workspace-wide, or keep `NEBULA_ENV` as a free string? Affects Phase 1.3
  surface.
- **Q3**: Should `OTEL_SERVICE_NAME` and `NEBULA_SERVICE` be unified (one
  var, one precedence) or remain distinct for OTEL-spec compatibility?
  (F6 — leans "keep distinct, document precedence".)
- **Q4**: Phase 2 crate placement — confirm **cross-cutting** tier
  (alongside `log`) is acceptable, or should the reader live in `core`?
- **Q5**: Fail-open vs fail-closed default for bool/int parse errors —
  `api` is fail-closed, `log` is fail-open. Pick a workspace default and
  list the deliberate exceptions.

---

## 7. Cross-references

- Env files: `.env.example` (app-level), `deploy/.env.example` (Taskfile
  local-dev defaults).
- Canonical (today) API parsers: `crates/api/src/config/env.rs`.
- Log precedence model: `crates/log/src/config/env.rs`
  (`resolve_startup`, `apply_env_overrides`).
- Layer map + `deny.toml [wrappers]` discipline: `CLAUDE.md`
  §"Layered Dependency Map".
- ADR numbering: `docs/adr/` (next free = **0086**).
- Plan-doc convention this file follows:
  `docs/plans/2026-05-28-001-feat-oauth-1.1-followups-plan.md`.
