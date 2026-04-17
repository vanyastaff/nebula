# Delete `nebula-config` + replace with `figment`

**Date:** 2026-04-17
**Author:** Claude (Opus 4.7)
**Authority:** Subordinate to `docs/PRODUCT_CANON.md`. Aligns with canon §14 (no false capabilities / framework-before-product) + §17 (DoD).
**Status:** draft — awaiting user approval before implementation

---

## Motivation

`nebula-config` is a 5 300-LoC crate that was built speculatively for "host config with hot reload + file watchers + env interpolation." None of those features have real consumers:

- `watchers/*` (1 131 LoC, 20% of crate) — zero call sites. Hot reload infrastructure nobody uses.
- `interpolation.rs` (262 LoC) — env var expansion inside config values. Not imported anywhere.
- `loaders/env.rs` (396 LoC) + `loaders/composite.rs` (206 LoC) — also not imported.

Actual usage across the workspace is **three items**: `Config`, `ConfigBuilder`, `ConfigSource::File`. And even those hide a bigger problem:

- `apps/cli/src/config.rs` has a typed `CliConfig` struct already derived `Serialize + Deserialize`. It uses `ConfigBuilder` only as a glorified file loader.
- `crates/api/src/state.rs` has `pub config: Arc<Config>` in `AppState`. **Grep confirms: this field is assigned in the constructor and never read anywhere.** The real API config is `ApiConfig::from_env()` built via plain `std::env`, not `nebula-config`.

The crate is **framework-before-product** (canon §14 anti-pattern): it imagines a future where Nebula needs programmable, layered, hot-reloading config for plugins and services. That future never arrived. The tests/examples that do use it could trivially depend on any standard config solution.

`nebula-config-macros` is a derive macro for this crate with **zero consumers** anywhere in-tree — confirmed orphan by the workspace health audit (#25).

## Decision

Delete both `nebula-config` and `nebula-config-macros`. Replace the **actual** config need (CLI loading) with [`figment`](https://crates.io/crates/figment) — the layered-providers config library used by Rocket. For API, simply remove the dead `AppState::config` field.

Net effect: **-5 300 LoC workspace**, same public behaviour for api/cli, dependency count +1 (figment).

## Target state

### `apps/cli/src/config.rs`

```rust
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};

pub fn load(global_path: Option<&Path>, local_path: Option<&Path>) -> Result<CliConfig, CliError> {
    let mut fig = Figment::from(Serialized::defaults(CliConfig::default()));
    if let Some(p) = global_path {
        if p.exists() {
            fig = fig.merge(Toml::file(p));
        }
    }
    if let Some(p) = local_path {
        if p.exists() {
            fig = fig.merge(Toml::file(p));
        }
    }
    fig = fig.merge(Env::prefixed("NEBULA_").split("_"));
    fig.extract().map_err(CliError::from)
}
```

Resolution order (highest wins): CLI flags → env → local TOML → global TOML → defaults. Matches existing documented order in the current `apps/cli/src/config.rs` module comment.

### `crates/api/src/state.rs`

```rust
pub struct AppState {
    // config: Arc<Config>,  // REMOVED — dead field
    pub workflow_repo: Arc<dyn WorkflowRepo>,
    pub execution_repo: Arc<dyn ExecutionRepo>,
    pub control_queue_repo: Arc<dyn ControlQueueRepo>,
    pub jwt_secret: JwtSecret,
    // ... remaining fields unchanged
}
```

`ApiConfig::from_env()` remains the production config loader for the API server — it already does the right thing via `std::env` and typed parsing.

### Tests

`crates/api/tests/common/mod.rs`, `integration_tests.rs`, `knife.rs`, `examples/simple_server.rs` currently use `nebula_config::ConfigBuilder` as a test helper. Replace with direct `ApiConfig::for_test()` (already exists, gated behind `test-util` feature) or constructor calls — whichever the file pattern dictates.

### Workspace

- Remove `crates/config` and `crates/config/macros` from `[workspace].members` in root `Cargo.toml`.
- Delete `crates/config/**` and `crates/config/macros/**` directories entirely.
- Remove `nebula-config = { path = "../config" }` from `crates/api/Cargo.toml` and `apps/cli/Cargo.toml`.
- Add `figment = { version = "0.10", features = ["toml", "env"] }` to `apps/cli/Cargo.toml` (not a workspace-wide dep — only cli needs it).

## Migration plan

Single atomic PR (per CLAUDE.md "delete over deprecate"). Steps (sequential within the one commit):

1. Add `figment` dep to `apps/cli/Cargo.toml`.
2. Rewrite `apps/cli/src/config.rs::load()` with figment.
3. Remove `pub config: Arc<Config>` from `AppState` + remove `config` param from `AppState::new` + update all callers (`simple_server.rs`, tests).
4. Update `crates/api/tests/common/mod.rs` + `knife.rs` + `integration_tests.rs` test helpers. If they only need an `AppState`, and `Config` was passed for the dead field, just remove that argument.
5. Remove `nebula-config` dep from `crates/api/Cargo.toml` and `apps/cli/Cargo.toml`.
6. Remove `crates/config` + `crates/config/macros` from workspace members.
7. `rm -rf crates/config crates/config/macros`.
8. Run: `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p nebula-api -p nebula-cli`, `cargo +nightly fmt --all`.
9. Knife §13 test must still pass (canon §13 scenario unchanged).

## Verification

- `cargo nextest run -p nebula-api -p nebula-cli` → all tests green
- `cargo check --workspace` → zero warnings
- `rg nebula_config` → zero hits
- `rg "crates/config"` → zero hits (outside of git history)
- API `knife_scenario_end_to_end` → still passes
- CLI `nebula run --config path/to/nebula.toml` → still loads config correctly

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| figment license / advisory conflict with `deny.toml` | Check before PR: `cargo deny check`. figment is Apache-2.0/MIT, deps are standard crates |
| Env var name → struct field mapping subtly different in figment vs nebula-config | Figment's `Env::prefixed("NEBULA_").split("_")` matches the current `NEBULA_RUN_CONCURRENCY` → `run.concurrency` convention. Verify with integration test |
| `AppState::new` signature breaks downstream callers | In-tree only (3 callers). All updated in same PR |
| `cli_config_loader` tests (if any) break | Re-check after rewrite; fix in same commit |
| figment pulls heavy dep chain | Check `cargo tree`. Historically figment is lean (~4 transitive deps beyond serde/toml) |

## Out of scope

- Adding NEW config features (env secrets, remote config, hot reload). Canon §14: don't ship what we don't need. If a real consumer appears, add then.
- Migrating `ApiConfig::from_env()` to figment. `std::env` is fine for ~5 env vars; adding figment there is over-engineering.
- Adding figment to other crates. Only CLI needs a layered loader.

## Canon alignment

- **§14 "framework before product":** crate removed because its feature set outran actual consumers.
- **§12.7 "no orphan modules":** `nebula-config-macros` had no consumers — deleted.
- **§17 "Definition of done":** README drift fix included (remove any mention of `nebula-config` from workspace docs if present).

## Follow-ups

Closes audit tasks:
- **#16 (partial) — feature-gate InMemory*Repo types** — unrelated but touches same crates; keep as separate task.
- **#25 — config-macros decision** (from Sprint D roadmap) — closed by this PR.

No new follow-ups created.
