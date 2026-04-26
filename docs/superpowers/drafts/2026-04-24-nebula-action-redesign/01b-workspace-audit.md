# 01b — nebula-action workspace / CI / tooling audit

Phase 0 ground truth for the nebula-action redesign cascade. Manifests, CI, dep shape, tooling gates — everything ABOUT `crates/action` but not its Rust source. Findings only; no fixes proposed.

Severity legend: 🔴 CRITICAL / 🟠 MAJOR / 🟡 MINOR.

All line refs verified against the main repo tree on 2026-04-24 (not the `.worktrees/nebula/upbeat-mendel-b30f89/` worktree, which holds only `.claude/` this session).

---

## 1. nebula-action crate Cargo manifest fingerprint

`crates/action/Cargo.toml`, 66 lines.

### Identity / metadata
- Inherits all `[package]` fields from workspace (`version`, `edition`, `rust-version`, `keywords`, `authors`, `description`, `license`, `repository`, `homepage`, `documentation`) — clean, no per-crate overrides.
- `edition = "2024"`, `rust-version = "1.95"`, `version = "0.1.0"` via workspace.
- No `publish = false` set — crate is intended for crates.io at some point (contrast: `apps/cli`, `crates/sandbox` pin `publish = false`). 🟡 MINOR — pre-release manifest hygiene worth confirming before the first publish wave.

### Direct dependency list (15 runtime + 1 alias = 16 direct deps)
| Dep | Pin source | Notes |
|---|---|---|
| `nebula-action-macros` | `path = "macros"` | sibling proc-macro crate |
| `nebula-core` | `path = "../core"` | core layer |
| `nebula-credential` | `path = "../credential"` | credential handles via `CredentialGuard` |
| `nebula-error` | workspace | typed errors |
| `nebula-metadata` | `path = "../metadata"` | `ActionMetadata` base |
| `nebula-schema` | `path = "../schema"` | `ValidSchema` parameters |
| `nebula-resource` | `path = "../resource"` | `DeclaresDependencies`, resource handles |
| `semver` | workspace (1) | version handling |
| `serde` + `serde_json` | workspace | serialization |
| `thiserror` | workspace (2.0) | error derives |
| `tokio-util` | workspace (0.7) | cancellation tokens |
| `chrono` | workspace | `Timing`, poll cursors |
| `zeroize` | `version = "1.8.2"` (inline!) | crypto payload zeroization |
| `parking_lot` | workspace | sync |
| `tokio` | workspace, features `["time","sync"]` | runtime primitives |
| `tracing` | workspace | spans |
| `http` / `bytes` / `url` | workspace | webhook HTTP vocabulary |
| `hmac` / `sha2` / `hex` / `base64` / `subtle` | workspace | webhook HMAC signature primitives |

### Dependency-shape findings

- 🟠 MAJOR — **`zeroize = { version = "1.8.2" }` is pinned inline in `crates/action/Cargo.toml:36` instead of `workspace = true`.** Workspace already declares `zeroize = { version = "1.8.2", features = ["std"] }` at `Cargo.toml:116`. Inline pin silently drops the `std` feature and de-unifies the version in the feature-unification graph. Two sites differ (action inline + workspace), so any future bump needs to touch both. Drift risk is load-bearing because crypto crates are shared with `crates/credential`, `crates/api`, and the webhook verification path.
- 🟡 MINOR — **`nebula-core = { path = "../core" }` is listed under both `[dependencies]` and `[dev-dependencies]`** (lines 24 and 57). Cargo tolerates this (dev-deps are additive), but it obscures whether the dep needs extra features in tests vs prod. Typical idiom is to list once in `[dependencies]` and only repeat in `[dev-dependencies]` when extra features are needed; there is no feature delta here.
- 🟡 MINOR — `hex` is listed in both `[dependencies]` (line 52) and `[dev-dependencies]` (line 59). Same pattern as `nebula-core`; dev-dep line adds nothing.
- 🟡 MINOR — `http = { workspace = true }` pulls `http = "1.4"`; the HTTP vocabulary is load-bearing for the webhook domain (`WebhookRequest: http::Method + http::HeaderMap`), so any major bump is a cross-workspace cascade, not an action-local change.
- 🟡 MINOR — Layer adherence good: no `nebula-engine`, `nebula-api`, `nebula-storage`, `nebula-sandbox`, `nebula-sdk`, `nebula-plugin-sdk` in `[dependencies]`. `deny.toml` bans those upward edges (lines 52–81), so the manifest is compliant.
- 🟡 MINOR — Dev-dep set is **minimal**: `tokio` (full test features), `hex`, `insta`, `pretty_assertions`, `rstest`, plus the self-reference to `nebula-core`. No `wiremock` despite a large webhook domain, no `mockall`, no `proptest`. Whether this is a gap is a Phase 1 pain point (not an audit finding).

### No-default-features support
- 🟠 MAJOR — `ci.yml` lines 110–117 run `cargo check --no-default-features` for `nebula-resilience`, `nebula-log`, `nebula-expression`, `nebula-api`, `nebula-credential`. **`nebula-action` is absent.** Its `default = []` is already empty so a no-default-features check would be redundant-but-cheap; the absence just confirms nobody treats action as having a no-default contract today.

---

## 2. nebula-action-macros subcrate manifest fingerprint

`crates/action/macros/Cargo.toml`, 28 lines.

### Identity / layout
- `proc-macro = true`, `test = false`, `doctest = false` — standard proc-macro shape.
- Inherits all workspace `[package]` fields. `description` is overridden locally (line 8).
- No `publish = false`.

### Dependencies (4)
| Dep | Pin | Notes |
|---|---|---|
| `nebula-macro-support` | `path = "../../sdk/macros-support"` | shared syn/quote helpers |
| `semver` | workspace | for version parsing in attr parser |
| `syn` | `"2.0"`, features `["full","extra-traits"]` (inline) | |
| `quote` | `"1.0"` (inline) | |
| `proc-macro2` | `"1.0"` (inline) | |

### Findings
- 🟡 MINOR — **`syn`, `quote`, `proc-macro2` are pinned inline, not through workspace deps.** The workspace does not declare them, so this is the only call site. Acceptable for a single-consumer pin, but the `nebula-plugin-macros`, `nebula-credential/macros`, `nebula-error/macros`, `nebula-schema/macros`, `nebula-validator/macros`, `nebula-resource/macros` crates likely do the same thing — a workspace-level pin would be more maintainable. Out of scope for action redesign, noted for future hygiene sweep.
- 🟠 MAJOR — **No `[dev-dependencies]` at all.** Specifically **no `trybuild` / `macrotest`.** The proc-macro is a 359-line `#[proc_macro_derive(Action, attributes(action, nebula))]` (`macros/src/action.rs` 63 LOC, `action_attrs.rs` 243 LOC, `lib.rs` 53 LOC). The macro attribute surface (per `lib.rs:17–49`) covers `key`, `name`, `description`, `version`, `credential`, `credentials`, `resource`, `resources`, `parameters` — with some documented rejection rules (string-valued `credential`, non-unit struct rejection). These compile-fail branches are not covered by any harness; only `crates/action/tests/derive_action.rs` exists (happy-path derive compilation + runtime assertions). This is the **macro test harness gap** the user called out.
- 🟡 MINOR — `nebula-macro-support` is reachable only via action-macros; any breaking change in macro-support is a single blast-radius crate for action today, but will grow as the plan calls for more `#[derive(Action)]` growth.

---

## 3. Workspace dependency shape (direct + transitive flags)

### Workspace members count
- Workspace lists 36 members in `Cargo.toml:2–37`. No `nebula-action-*` crates other than `crates/action` and `crates/action/macros`. **Confirmed**: there is no third action-adjacent crate hiding in the tree.

### Action's dep tree fan-in to workspace
The direct crate deps (`nebula-core`, `nebula-credential`, `nebula-error`, `nebula-metadata`, `nebula-resource`, `nebula-schema`, plus the action-macros sibling) pull in the entire "core" + business layer. No upward deps (engine/api/storage/sandbox) — clean.

### Transitive crypto stack (pulled via webhook domain)
Action uses `hmac`, `sha2`, `hex`, `base64`, `subtle` directly (verified via file grep). Workspace pins (`Cargo.toml:112–118`):
- `sha2 = "0.11"`, `hmac = "0.13"`, `hex = "0.4"`, `subtle = "2.6"`, `base64 = "0.22.1"`, `zeroize = "1.8.2"`.
- 🟡 MINOR — These versions are shared with `nebula-api` (`crates/api/Cargo.toml:77–80`) and transitively with `nebula-credential`, `nebula-storage`. Feature unification means a bump touches every crate simultaneously.
- 🟠 MAJOR — `hmac = "0.13"` workspace / transitive: `deny.toml:95` lists `hmac` under the `skip = [...]` block, indicating **a known duplicate version in the tree**. `skip` ≠ `allow` — it tells `cargo-deny bans` to ignore the duplicate, not fix it. Same for `sha2` (line 103), `rand` / `rand_chacha` / `rand_core`, `digest`, `crypto-common`, `block-buffer`, `cpufeatures`, `const-oid`. This is pre-existing tech debt; action inherits it via webhook.

### `deny.toml` posture affecting action
- `licenses.allow` (lines 21–33) is permissive: Apache-2.0, BSD-2/3, CC0-1.0, CDLA-Permissive-2.0, ISC, MIT, MPL-2.0, Unicode-3.0, Unicode-DFS-2016, Zlib. No `exceptions`.
- `advisories.ignore` (lines 14–18) has three live exceptions:
  - `RUSTSEC-2023-0071` (rsa via jsonwebtoken) — not action's concern.
  - `RUSTSEC-2026-0097` (rand 0.10 unsound panic-logger + TLS interaction) — transitively reachable via `jsonwebtoken` + any crypto pull; `rand` is in the skip list so it's already duplicated. Not action-specific.
  - `RUSTSEC-2026-0098` (webpki URI name-constraint handling) — reqwest rustls path, not action's direct code.
- 🟡 MINOR — `multiple-versions = "deny"` (line 42), mitigated by the `skip` list (lines 82–117) and the `figment@0.10` `skip-tree`. The skip list currently carries 30+ crates. Not action-specific; inherited.
- 🟢 action has **no direct `unsafe` code** (`#![forbid(unsafe_code)]` at `lib.rs:34`). Workspace lint `unsafe_code = "warn"` at `Cargo.toml:177`; action escalates to `forbid`.

### Layer enforcement in `deny.toml`
- 🟢 `nebula-action` is **not listed** in the `bans.deny` list, meaning nothing prevents upward or sibling crates from depending on it. Consumer crates (per section 9) include `engine`, `api`, `sandbox`, `sdk`, `plugin`, `cli` — all correct per layer direction.
- 🟡 MINOR — there is no explicit layer-ban entry for `nebula-action` (the way engine/storage/sandbox/sdk have one). If canon wants action to be purely-"business" and not depended on by, e.g., `nebula-credential` → a positive ban would enforce that. Today it's "implicit correct" because no lower-layer crate happens to reach for it. Not a current bug, but a missing guardrail for the redesign.

---

## 4. Feature flag inventory (active + unstable + planned)

### On `nebula-action`
Manifest lines 14–20:
- `default = []`
- `unstable-retry-scheduler = []` — **empty feature**, no deps wired. Per comment lines 16–19: "Exposes the engine-level retry surface (`ActionResult::Retry`). Currently a planned capability without persisted attempt accounting (canon §11.2). The engine does not honor the variant end-to-end — enabling this flag only un-hides the type; it does not wire a scheduler. Do not enable in production."

### On `nebula-engine`
`crates/engine/Cargo.toml:21`: `unstable-retry-scheduler = ["nebula-action/unstable-retry-scheduler"]` — engine forwards the flag. Per comment lines 13–20, this is a **convenience alias** only; the engine's retry-detection path uses the always-available `ActionResult::is_retry()` predicate, so feature unification is correctness-safe regardless of downstream activation.

### No other `nebula-action/*` feature forwarding
Searched all `Cargo.toml` files — engine is the only forwarder. Action itself declares no other feature flags.

### Findings
- 🟠 MAJOR — **Canon §11.2 drift is real and documented in two places** (action manifest comment + engine manifest comment) but **exposed behind a feature that is empty (`[]`)**. A dead-code flag with no deps and no cfg-gates means "enabling the feature" on the engine has zero build-time effect today; the gate is purely documentary. The `ActionResult::Retry` variant is almost certainly `#[cfg(feature = "unstable-retry-scheduler")]`-gated on the type side (rust-senior's scope). The risk is **feature unification unsafety**: because the engine maps `nebula-engine/unstable-retry-scheduler` → `nebula-action/unstable-retry-scheduler`, any workspace-level feature unification (e.g., `cargo check --all-features` in CI) turns it on. See §6 finding on `ci.yml:109`.
- 🟠 MAJOR — `ci.yml:109` runs `cargo check --workspace --all-features --all-targets` — **this unconditionally turns on `unstable-retry-scheduler` for every CI run**, despite the "do not enable in production" note. That's arguably fine for the *type-check* path (which is exactly what the variant is for), but if the variant is ever guarded by invariants that break under unification, CI is already running with it on. Good news: `cargo nextest` in `test-matrix.yml` runs **without** `--all-features` except for `nebula-engine` and `nebula-api` (lines 170–175), so runtime tests for action itself run with `default = []` (i.e., without `unstable-retry-scheduler`).
- 🟡 MINOR — **No feature-gated test selection** — action has no `#[cfg(feature = "…")]` test modules, so the retry-scheduler feature is never explicitly exercised at runtime in action's own test suite (only on the type-check side via `--all-features`).

---

## 5. MSRV / toolchain / edition

### Current state (verified)
- `rust-toolchain.toml:17` — `channel = "1.95.0"` (pinned stable). Components: rustfmt, clippy, rust-src, rust-analyzer; profile minimal.
- Workspace (`Cargo.toml:52–54`): `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.95"`.
- `resolver = "3"` (line 49).
- Action inherits all three via `.workspace = true`.

### CI enforcement
- `ci.yml:147–169` — dedicated MSRV job uses `toolchain: "1.95"` and runs `cargo check --workspace --all-targets`. Tier 2 (skipped on draft PRs unless `run-full-ci` label).
- `ci.yml:60–66` — fmt job uses **nightly rustfmt** (per comment lines 56–59, `rustfmt.toml` sets unstable options).
- `ci.yml:79`, `101`, `135`, `161`, `189`, `211`, `236` — all other jobs pin `toolchain: "1.95"`.
- `test-matrix.yml:127`, `151` — tests use `dtolnay/rust-toolchain@stable` (NOT pinned to 1.95). 🟡 MINOR — means tests effectively run on whatever stable rustup resolves at CI time. With MSRV = stable already, this is low-risk drift; if stable ever moves ahead of a 1.95-specific regression, the MSRV job would catch it first.
- `cross-platform.yml:40`, `codspeed.yml:179`, `semver-checks.yml:32`, `security-audit.yml` — all `stable`, same note.
- `udeps.yml:28` — nightly.

### Findings
- 🟢 Alignment is coherent: action's manifest inherits MSRV 1.95; CI's MSRV job targets 1.95; local toolchain is pinned 1.95. No action-crate-specific MSRV drift.
- 🟡 MINOR — test-matrix uses `@stable` not `@1.95`; if rustup upgrades silently mid-cascade, action's tests could pass locally (1.95) but hit new lints under the stable-plus-one nightly surface. Non-blocking for the redesign itself; worth mentioning if the redesign lands during an MSRV bump window.

---

## 6. CI coverage of action crate (jobs + commands + line refs)

### Hits on `nebula-action` in workflows
- `test-matrix.yml:66` — Action is in the FULL matrix list (first entry): `["nebula-action", "nebula-api", "nebula-core", ..., "nebula-workflow"]`.
- `test-matrix.yml:176` — Generic matrix branch: `cargo nextest run -p "${{ matrix.package }}" --profile ci --no-tests=pass`. Action runs **without any feature flags**.
- `ci.yml:88` — clippy: `cargo clippy --workspace -- -D warnings` (includes action).
- `ci.yml:66` — fmt: `cargo +nightly fmt --all -- --check` (includes action).
- `ci.yml:109` — type check: `cargo check --workspace --all-features --all-targets` (pulls in action with `unstable-retry-scheduler` on).
- `ci.yml:143` — doctests: `cargo test --workspace --doc` (includes action; action has ~20+ doc examples based on the grep).
- `ci.yml:169` — MSRV: `cargo check --workspace --all-targets` on 1.95 (includes action).
- `ci.yml:197` — docs: `cargo doc --no-deps --workspace` with `RUSTDOCFLAGS: -D warnings` (includes action).
- `ci.yml:240` — `cargo deny check` (includes action's subtree).

### Jobs that DO NOT run against action
- `ci.yml:199–219` — `bench` job runs only `cargo bench -p nebula-log --bench log_hot_path`. No action bench.
- `codspeed.yml:88–102` — Filters match only `crates/resilience`, `crates/validator`, `crates/core`, `crates/eventbus`, `crates/expression`, `crates/log`, `crates/system`. **Action is NOT a CodSpeed shard.**
- `cross-platform.yml:5–8` + `:47–51` — Only triggers on `crates/sandbox/**`, `crates/runtime/**`, `crates/plugin-sdk/**`, and only runs `cargo test -p nebula-sandbox -p nebula-runtime -p nebula-plugin-sdk` in the matrix. **Action gets no cross-platform (macOS/Windows) test coverage**, despite containing webhook signature verification (HMAC on raw bytes — unlikely to be platform-sensitive, but not formally verified).
- `udeps.yml` — weekly workspace-wide `cargo +nightly udeps --workspace --all-targets` (line 35). Covers action.

### Aggregator
- `ci.yml:242–273` — `required` job gates `fmt + clippy + check + doctests + msrv + doc + deny`. The `bench` job is NOT in the required aggregator (line 200 uses `if: github.event_name != 'pull_request'`), so bench failures do not block PRs — they only run on `push:`.
- `test-matrix.yml:196–225` — `tests` aggregator gates `select-matrix + warm-cache + test-crates`.

### Findings
- 🟠 MAJOR — **No action-specific benchmark coverage anywhere.** Bench on `ci.yml:219` is `nebula-log`-only. CodSpeed skips action. Action has **1,852 LOC in `webhook.rs`** alone (HMAC verification hot path) and **1,680 LOC in `result.rs`** (dispatcher). If any regression is introduced, CI won't catch it perf-wise. For a redesign cascade with DX/perf implications, this is worth flagging.
- 🟠 MAJOR — **No trybuild / compile-fail harness in CI.** `ci.yml` doctests step (`cargo test --workspace --doc`) runs doc examples, but these are `use`-level snippets. The proc-macro's rejection paths (`credential = "string"`, non-unit struct, missing required attrs) have no compile-fail test → no CI enforcement that bad derive usage yields a good error message.
- 🟠 MAJOR — **`test-matrix.yml:66` FULL list includes `"nebula-runtime"` but `crates/runtime/` does not exist in the workspace.** Verified by `ls crates/` (24 dirs, none named `runtime`) and by `Cargo.toml:2–37` (no `crates/runtime` member). Result: on every push to `main` and every `workflow_dispatch`, the test matrix spawns a shard that runs `cargo nextest run -p nebula-runtime --profile ci --no-tests=pass` → **cargo errors with "package `nebula-runtime` not found"**. Because the matrix has `fail-fast: false` (line 142), other shards continue, but the `tests` aggregator (line 200) requires all shards success, so `Tests` fails on main-push. Either (a) CI is currently red on main-push and nobody noticed because PRs use diff-scope; (b) there is a silent `cargo nextest` quirk that treats missing `-p` as pass; or (c) `--no-tests=pass` somehow masks it. Regardless, this is a dead reference that should be cleaned up. Out of action's redesign scope but directly adjacent and worth filing.
- 🟠 MAJOR — **`.github/CODEOWNERS:52` lists `/crates/runtime/ @vanyastaff`** — same dead reference as above. CODEOWNERS validator (`codeowners.yml:22`) uses `experimental_checks: "avoid-shadowing,no-unowned-patterns"` but NOT `files` check on nonexistent paths — wait, it does include `"files,duppatterns,syntax"`. If `files` verifies path existence, the validator should be failing. Either the validator is permissive about missing dirs or this hasn't been triggered since runtime was removed.
- 🟡 MINOR — Action has no no-default-features entry in `ci.yml:110–117`. Since `default = []`, that's a no-op gate, not a real gap.
- 🟡 MINOR — `semver-checks.yml:27` — advisory-only (`continue-on-error: true`) during alpha. Action semver bumps won't fail CI; this is intentional per comment lines 25–27.
- 🟡 MINOR — `security-audit.yml` — weekly `cargo audit`; workspace-wide, covers action transitively. No action-specific gate.

---

## 7. Dev tooling gaps (trybuild, macrotest, miri, coverage, benches, semver-checks)

| Tool | Workspace-wide? | Action? | Notes |
|---|---|---|---|
| `trybuild` | ❌ | ❌ | Not in any workspace dep list. No `tests/compile_fail/` or `tests/ui/` under action or action-macros. Macro rejection surface is untested at compile time. 🟠 MAJOR given the macro breadth. |
| `macrotest` | ❌ | ❌ | Same — no expansion snapshot tests. If the macro output changes subtly (e.g., spans, ordering), no harness detects it. 🟠 MAJOR. |
| `insta` (snapshot) | ✅ (workspace dep, line 147) | ✅ (action dev-dep line 60) | Available but unclear whether action uses it for macro output. Tests dir doesn't show `.snap` files by filename pattern (would need src grep). |
| `miri` | ❌ | ❌ | No `.github/workflows/miri.yml`. Action is `#![forbid(unsafe_code)]` so miri value is low for UB, but it would still catch UB in deps. Not action-specific; no one in the workspace runs miri. 🟡 MINOR. |
| coverage (`cargo llvm-cov` / `tarpaulin`) | ❌ | ❌ | No coverage workflow. No `codecov.yml`. 🟡 MINOR — alpha-stage, OK to defer. |
| `cargo bench` (criterion / codspeed) | ✅ for `log`, `resilience`, `validator`, `core`, `eventbus`, `expression`, `system` | ❌ | No benches for action. See §6. 🟠 MAJOR. |
| `cargo-semver-checks` | ✅ advisory via `semver-checks.yml` | ✅ (incl. action) | Advisory-only (`continue-on-error: true`); won't block redesign PRs. 🟡 MINOR. |
| `cargo-udeps` | ✅ weekly via `udeps.yml` | ✅ (includes action) | Weekly schedule, nightly toolchain. 🟢 fine. |
| `cargo-deny` | ✅ CI + lefthook | ✅ | Runs on every PR via `ci.yml:240` and on every commit via `lefthook.yml:30–32`. 🟢. |
| `typos` + `taplo` | ✅ via `hygiene.yml` | ✅ | Covers action. 🟢. |
| `proptest` | ✅ (workspace line 141) | ❌ | Not a dev-dep on action. 🟡 MINOR — webhook HMAC + port routing could plausibly benefit. |
| `wiremock` | ✅ (workspace line 152) | ❌ | Action's webhook domain is *inbound*, so wiremock (outbound HTTP mock) is the wrong tool here. 🟢 not a gap. |
| `mockall` | ✅ (workspace line 148) | ❌ | Could mock `TriggerScheduler`, `ExecutionEmitter`, `ResourceAccessor` traits for isolated tests. Not in dev-deps. 🟡 MINOR. |
| `rstest` | ✅ | ✅ (dev-dep line 62) | 🟢. |
| `pretty_assertions` | ✅ | ✅ (dev-dep line 61) | 🟢. |
| `tokio` (test features) | ✅ | ✅ full set (line 58) | 🟢. |

### Top three tooling gaps
1. 🟠 MAJOR — **No macro harness** (trybuild/macrotest) for `#[derive(Action)]`. The macro has 9+ documented attribute rules with hard rejections; none are regression-proof.
2. 🟠 MAJOR — **No benchmarks** for action. Webhook HMAC verify, result dispatch, port routing are hot paths.
3. 🟡 MINOR — **No proptest/mockall** in dev-deps despite workspace availability. Whether this matters is a Phase 1 question.

---

## 8. Lefthook / pre-push gate alignment

`lefthook.yml` (49 lines) and `scripts/pre-push-crate-diff.sh` (45 lines) verified.

### Pre-commit stages
- `fmt-check` — `cargo +nightly fmt --all -- --check` (matches `ci.yml:66`) ✅
- `clippy` — `cargo clippy --workspace --all-targets -q -- -D warnings` (matches `ci.yml:88` modulo `--all-targets`) ✅
- `typos` — `typos --quiet` (matches `hygiene.yml:34`) ✅
- `taplo` — `taplo fmt --check` (matches `hygiene.yml:37`) ✅
- `cargo-deny` — `cargo deny --log-level error check 2>&1` (matches `ci.yml:240`) ✅

### Pre-push stages
- `crate-diff-gate` runs `scripts/pre-push-crate-diff.sh`:
  - Computes changed-crates vs `origin/main` (line 11).
  - Runs `cargo nextest run -p nebula-$crate --profile agent` for each changed crate (line 36).
  - Runs `cargo check -p nebula-$crate --all-features --all-targets --quiet` (line 37).
  - Runs `cargo check --no-default-features --quiet` ONLY for `resilience`, `log`, `expression` (lines 40–44).

### Alignment vs CI required jobs

| CI required job (`ci.yml:249`) | Mirrored in lefthook? | Notes |
|---|---|---|
| `fmt` | ✅ pre-commit `fmt-check` | exact match |
| `clippy` | ✅ pre-commit `clippy` | lefthook adds `--all-targets` — stricter than CI |
| `check` | 🟡 pre-push crate-diff (`cargo check -p X --all-features`) | **partial**: pre-push is **diff-scoped** per-crate, CI is `--workspace`. On PR with no crate changes, pre-push does nothing; CI still runs. Divergence by design, called out in the sh script. |
| `doctests` (`cargo test --workspace --doc`) | ❌ | Commented "Doctests/docs/MSRV remain CI-owned checks" (lefthook.yml:45). Intentional. 🟡 MINOR divergence. |
| `msrv` | ❌ | Same note. |
| `doc` (`cargo doc --no-deps --workspace` with `-D warnings`) | ❌ | Same note. |
| `deny` | ✅ pre-commit `cargo-deny` | exact match |
| (test-matrix.yml) `Tests` aggregator | 🟡 partial | pre-push runs `cargo nextest -p <changed-crates> --profile agent`. CI uses `--profile ci --no-tests=pass`. Different profiles, different scope. |

### Findings
- 🟠 MAJOR — **Lefthook pre-push does NOT mirror `doctests`, `msrv`, or `doc` jobs.** This is documented as intentional in `lefthook.yml:45` ("Doctests/docs/MSRV remain CI-owned checks"). Per user feedback memory (`feedback_lefthook_mirrors_ci.md` — "lefthook pre-push MUST mirror every CI required job"), this is a live divergence from the stated policy. Whether to fix is a product call; flagging per audit scope. Action-specific impact: action has 20+ doctest examples (per §6 and the `lib.rs` grep), none of which run in pre-push. A redesign PR touching action signatures could land a doctest regression that only surfaces in CI.
- 🟢 `fmt`, `clippy`, `deny` are mirrored exactly.
- 🟡 MINOR — `ci.yml:110–117` no-default-features specific crates are mirrored in the pre-push script (lines 40–44) for `resilience`, `log`, `expression` — but `ci.yml` also runs it for `nebula-api` (both default and `credential-oauth`) and `nebula-credential`. Pre-push does not. Divergence, not action-specific.

---

## 9. Reverse-dependency fingerprint (consumers of nebula-action public API)

### Cargo-declared consumers (10 crates + 1 app)
Verified by grepping `nebula-action =` across all `Cargo.toml` files:

1. `crates/action/macros` (intra-action) — Cargo self-reference via `nebula-action-macros`.
2. `crates/engine` (`Cargo.toml:27`) — **primary consumer**, forwards `unstable-retry-scheduler`.
3. `crates/api` (`Cargo.toml:35`) — webhook transport & routing.
4. `crates/sandbox` (`Cargo.toml:16`) — sandboxed action runners.
5. `crates/sdk` (`Cargo.toml:17`) — public SDK re-export façade.
6. `crates/plugin` (`Cargo.toml:22`) — plugin resolves `Action`.
7. `apps/cli` (`Cargo.toml:66`) — CLI dev command, action listing, testing.

**Not** cargo-declared but uses re-exports transitively:
- `crates/workflow` — `connection.rs` references `nebula_action::control::ControlAction` in rustdoc only (no runtime dep). 🟢 doc-only.
- `crates/storage` — `execution_repo.rs:425` rustdoc comment refers to `nebula_action::ActionResult<Value>` as semantic parallel. 🟢 doc-only.
- `crates/execution` — `status.rs:146` rustdoc parallels `nebula_action::TerminationCode`. 🟢 doc-only.

### Public API surface imports (observed, by consumer)
Grep for `use nebula_action::…` shows 69 files total. Summary by consumer:

**`nebula-engine` — by far the deepest consumer** (27+ import sites in `engine.rs`, `runtime.rs`, `registry.rs`, `error.rs`, `stream_backpressure.rs`):
- `ActionError`, `ActionResult`, `ActionMetadata`, `ActionHandler`, `ActionContext`
- `PortKey`, `BranchKey`, `TerminationCode`, `BreakReason`
- `Overflow` (stream backpressure)
- `capability::default_resource_accessor`
- `handler::ActionHandler`, `trigger::TriggerHandler`
- `resource::ResourceHandler`
- `stateful::StatefulHandler`
- `result::ActionResult` (re-aliased as `AR`)

**`nebula-api` — webhook transport/routing** (4 files):
- `TriggerHandler`, `TriggerRuntimeContext`, `TriggerContext`
- `WebhookConfig`, `WebhookEndpointProvider`
- Plus `crates/api/tests/webhook_transport_integration.rs` test imports.

**`nebula-sandbox` — dyn-handler bridge** (7 files: `runner`, `remote_action`, `handler`, `process`, `in_process`, `discovery`, `discovered_plugin`):
- `Action`, `ActionContext`, `ActionError`, `ActionMetadata`, `ActionResult`
- `StatelessHandler`, `ActionHandler`
- Spans both in-process and out-of-process runners.

**`nebula-sdk` — prelude façade** (5 files):
- `src/lib.rs:47` — `pub use nebula_action;` (full re-export)
- `prelude.rs` — 20+ types (`Action`, `ActionContext`, `ActionError`, `ActionResult`, `DeclaresDependencies`, `Field`, `PollTriggerAdapter`, `Schema`, `StatefulActionAdapter`, `StatelessAction`, `StatelessActionAdapter`, `TriggerContext`, `TriggerEvent`, `TriggerEventOutcome`, `ValidSchema`, `WebhookRequest`, `WebhookTriggerAdapter`, `field_key`, `ActionMetadata`, `DeduplicatingCursor`, `PollAction`, `PollConfig`, `PollCursor`, `PollResult`, `InputPort`, `OutputPort`, `BreakReason`, `BatchAction`, `BatchItemResult`, `PageResult`, `PaginatedAction`, `StatefulAction`, `SpyEmitter`, `SpyLogger`, `SpyScheduler`, `StatefulTestHarness`, `TestContextBuilder`, `TriggerTestHarness`, `WebhookAction`, `WebhookHttpResponse`, `WebhookResponse`, `impl_batch_action`, `impl_paginated_action`)
- Plus `runtime.rs`, `action.rs`, `testing.rs`.

**`nebula-plugin`** (2 src files + 1 test):
- `Action`, `ActionMetadata`, `DeclaresDependencies`

**`apps/cli`** (5 files: `actions.rs`, `dev/action.rs`, `run.rs`, `watch.rs`, `replay.rs`):
- `ActionHandler`, `ActionResult`, `BreakReason`, `SpyEmitter`
- `Context`, `ActionError`, `ActionMetadata`, `Action`, `DeclaresDependencies`, `StatelessAction`

**`examples/hello_action.rs`** — one usage site via the prelude.

### Public API shape
Per `crates/action/src/lib.rs:91–153`, the `pub use` exports 63 named items across 12 module roots:
- `action::Action` (1)
- `capability::*` — `ExecutionEmitter`, `TriggerHealth`, `TriggerHealthSnapshot`, `TriggerScheduler` (4)
- `context::*` — 8 types
- `control::*` — 4 types
- `error::*` — 5 types + constants
- `handler::*` — 1
- `metadata::*` — 4
- `nebula_action_macros::Action` (1)
- re-exports from `nebula_core`, `nebula_credential`, `nebula_schema` (10+ types)
- `output::*` — 24 types
- `poll::*` — 9 types + constants
- `port::*` — 6
- `resource::*` — 3
- `result::*` — 7
- `stateful::*` — 9
- `stateless::*` — 6
- `testing::*` — 8
- `trigger::*` — 5
- `validation::*` — 3
- `webhook::*` — 20 types + free functions

### Findings
- 🟠 MAJOR — **The SDK prelude re-exports a very wide slice of nebula-action**, essentially committing the full-type façade downstream. Any rename/relocation in action cascades directly to `nebula-sdk::prelude::*`, which is the officially-sanctioned user-facing API. The redesign must treat `nebula-sdk/src/prelude.rs:15–33` as a **public contract surface**.
- 🟠 MAJOR — **`nebula-engine` is tightly coupled** — 27+ import sites across `engine.rs`, `runtime.rs`, `registry.rs`. A redesign that touches `ActionHandler`, `ActionResult`, `ActionMetadata`, `ActionError`, `PortKey`, or `TerminationCode` will ripple into engine's dispatcher in non-trivial ways.
- 🟠 MAJOR — **`nebula-sandbox` imports action's dyn-handler contract** (`StatelessHandler`, `ActionHandler`) into both in-process and out-of-process runners. If the handler trait family changes shape, sandbox's process/in-process adapters need new ABI.

---

## 10. Migration blast radius estimate

### Summary
- **7 direct reverse-deps** (5 crates: engine, api, sandbox, sdk, plugin + 1 app: cli + the action-macros sibling).
- **3 indirect/doc-only references** (workflow, storage, execution — doc comments only, no compile edge).
- **69 source files** across the workspace import `nebula_action::*` symbols.
- **63 public items** are re-exported through `crates/action/src/lib.rs` lines 91–153.
- **~40+ SDK-prelude items** are re-exported to end users through `nebula-sdk::prelude` (line 15–33).

### Blast-radius weight by consumer
| Consumer | Import sites | Risk category | Notes |
|---|---|---|---|
| `nebula-engine` | 27+ | 🔴 HEAVY | Runtime dispatcher, registry, and error paths all bind action types. |
| `nebula-sandbox` | 7 files | 🟠 MODERATE | Dyn-handler ABI. In-process + out-of-process runners. |
| `nebula-sdk` | 5 files | 🟠 MODERATE | Public prelude facade (contract) + 40+ re-exports. |
| `nebula-api` | 4 files | 🟡 LIGHT | Webhook domain only. |
| `nebula-plugin` | 3 files | 🟡 LIGHT | Just `Action`, `ActionMetadata`, `DeclaresDependencies`. |
| `apps/cli` | 5 files | 🟡 LIGHT | Dev command surface. |
| Action's own tests + doc examples | 13 test files + ~20 doctests | 🟡 LIGHT | Self-contained. |
| workflow/storage/execution (doc-only) | 3 comment references | 🟢 NEGLIGIBLE | No compile coupling. |

### Total reach
**Compile-cascade touch count = ~55 files in 6 crates + 1 app** if the public-facing type names change. Even a rename-only refactor of e.g. `ActionResult::Retry` or `ActionError::Retryable` would require coordinated updates across engine runtime, sandbox runners, sdk prelude, and api webhook. If the redesign additionally changes the trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`, `ControlAction`), the sandbox dyn-handler ABI is the most fragile piece (binary-compat via `StatelessHandler`/`ActionHandler` on the process/in-process boundary).

### Semver-contract consideration
- `semver-checks.yml:27` is **advisory-only during alpha**. A redesign can land breaking changes today without CI blocking. After alpha, the crate is on 0.1.0 with the SDK prelude as the pinned user surface — breaking changes become expensive.
- 🟢 feedback memory `feedback_hard_breaking_changes.md` + `feedback_adr_revisable.md` + `feedback_bold_refactor_pace.md` all align: hard breaking changes are acceptable right now for spec-correct outcomes; the blast radius just sets the scale, not the gate.

---

## 11. Ground truth summary (1-page executive, top 10 observations)

**Verified 2026-04-24. All line refs against main repo tree (worktree is `.claude`-only this session).**

1. 🟠 MAJOR — **No macro test harness** (no `trybuild` / `macrotest` dev-dep). 359-LOC `#[derive(Action)]` with 9+ attribute rejection rules → zero compile-fail regression coverage. `crates/action/macros/Cargo.toml` has no `[dev-dependencies]` section at all.
2. 🟠 MAJOR — **No benchmarks for action.** `ci.yml:219` benches only `nebula-log`. CodSpeed skips action. Hot paths (webhook HMAC, result dispatch, port routing in ~7,500 LOC across `webhook.rs` / `result.rs` / `port.rs`) have no perf guard.
3. 🟠 MAJOR — **Canon §11.2 `unstable-retry-scheduler` is a dead empty feature** at `crates/action/Cargo.toml:20`. Engine forwards it (`crates/engine/Cargo.toml:21`). `ci.yml:109` `cargo check --all-features` turns it on every run. `ActionResult::Retry` un-hides but has no end-to-end engine wiring — documented drift.
4. 🟠 MAJOR — **`test-matrix.yml:66` includes dead `nebula-runtime`** in FULL list; `crates/runtime/` does not exist. On push/workflow_dispatch, matrix spawns a shard that should fail with "package not found". Also in `.github/CODEOWNERS:52`. Not action's scope but directly adjacent.
5. 🟠 MAJOR — **`zeroize` pinned inline** (`crates/action/Cargo.toml:36`, `"1.8.2"`) instead of `workspace = true`. Drops `std` feature; drift risk vs workspace pin at `Cargo.toml:116`.
6. 🟠 MAJOR — **Lefthook pre-push does not mirror `doctests` / `msrv` / `doc` jobs** (lefthook.yml:45 — intentional but contradicts user-stated policy in `feedback_lefthook_mirrors_ci.md`). Action has 20+ doctests; none gated locally.
7. 🟠 MAJOR — **SDK prelude re-exports ~40+ action types** (`crates/sdk/src/prelude.rs:15–33`). Redesign must treat this as the public user-facing contract surface; rename = cascade.
8. 🟠 MAJOR — **Engine is tight-coupled to action** — 27+ import sites across `engine.rs`, `runtime.rs`, `registry.rs`. `ActionHandler`, `ActionResult`, `ActionMetadata`, `ActionError`, `PortKey`, `TerminationCode` all wired into the dispatcher directly.
9. 🟠 MAJOR — **No layer-enforcement deny rule for `nebula-action`** in `deny.toml`. Engine/storage/sandbox/sdk have positive bans; action relies on implicit correctness. Missing guardrail for the redesign.
10. 🟡 MINOR — **`nebula-core` and `hex` listed in both `[dependencies]` and `[dev-dependencies]`** in `crates/action/Cargo.toml` (lines 24+57, 52+59) with no feature delta — dev-dep lines are no-ops. Manifest hygiene.

**Migration blast radius — single sentence:** 7 direct reverse-deps (engine, api, sandbox, sdk, plugin, cli + action-macros sibling), 69 source files importing `nebula_action::*`, 63 public items re-exported from action's lib.rs, 40+ cascaded through `nebula-sdk::prelude` — engine and sandbox carry the heaviest weight.
