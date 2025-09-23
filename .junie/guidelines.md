# Nebula Project Guidelines (for Junie)

This document captures project-specific practices that help you build, test, and debug the Nebula workspace efficiently. It assumes an experienced Rust developer working on Windows (msvc) with Rust 1.89+ and Edition 2024.

## Workspace overview
- Workspace root: C:\\Users\\vanya\\RustroverProjects\\nebula
- Members (non-exhaustive): crates/nebula-{action,config,core,credential,error,log,memory,parameter,resilience,resource,system,value} and others listed in Cargo.toml.
- MSRV: 1.89 (see [workspace.package] rust-version in Cargo.toml)
- Edition: 2024
- Resolver: 3
- Lints: Extensive rust/clippy/rustdoc lints are configured at the workspace level. Expect warnings escalated in CI. See workspace lints in Cargo.toml.
- Formatting/lints configs: rustfmt.toml, clippy.toml at repo root.

## Build and configuration
- Toolchain: stable-x86_64-pc-windows-msvc, Rust 1.89 or newer.
- Build all crates (debug):
  - cargo build
- Build release profile variants provided by workspace:
  - cargo build --release
  - cargo build --profile release-with-debug
- Profiles: dev, release, test, bench, plus custom release-with-debug, embedded, wasm.
- Common deps are centralized via [workspace.dependencies] in root Cargo.toml.

Notes
- Some crates enable strict lints (missing_docs, rustdoc) which can fail CI if violated. Prefer running clippy locally before pushing (see below).
- When the workspace is mid-change, not all crates may compile or tests may fail. Prefer selective builds/tests per crate (cargo -p <crate>) during focused development.

## Testing
Nebula has many unit and integration tests across crates. Running everything at once can surface unrelated failures while you work. Use these patterns:

- Run tests for a single crate:
  - cargo test -p nebula-core
- Run a specific integration test file (avoids unit tests embedded in other modules of the same crate):
  - cargo test -p nebula-core --test <file_stem>
  Example: cargo test -p nebula-core --test junie_smoke
- Run a single test by name (supports substring filtering):
  - cargo test -p nebula-core test_name_substring
- Include test output (disable quiet): omit -q and optionally set RUST_BACKTRACE=1 for panics.

Test authoring
- Unit tests: place in the same module guarded by #[cfg(test)] mod tests { ... }.
- Integration tests: place under <crate>/tests/*.rs, each file compiles to a separate test binary. These are best when you want to run only that file without triggering all unit tests.
- Test utilities/fixtures: consider placing under <crate>/src/testing or a private module with #[cfg(test)], re-used via mod.

Demonstrated flow used while writing these guidelines
- We validated the selective testing flow by targeting a single crate (nebula-core). Full workspace tests currently fail due to unrelated compilation issues in other members, but targeted runs are effective.
- A temporary integration test can be used to smoke-test the harness without running all existing tests:
  1) Create file: crates/nebula-core/tests/junie_smoke.rs
     Content:
     
     #[test]
     fn junie_smoke_addition() {
         assert_eq!(2 + 2, 4);
     }
     
  2) Run only this file:
     cargo test -p nebula-core --test junie_smoke
  3) Remove the file after verification to keep the repo clean.

Environment and logging
- Some crates use tracing. To see runtime logs in tests, set:
  - RUST_LOG=debug (and run tests without -q), or enable tracing-subscriber as configured in the crate.
- For flaky async tests, prefer tokio::test with an explicit runtime configuration (features = ["full"] are enabled at the workspace level).

## Adding new tests (guidelines)
- Prefer deterministic tests. Avoid timing-sensitive sleeps; use tokio time::pause/advance or deterministic clocks.
- Avoid network calls in unit tests. Use mocks (mockall), temp dirs (tempfile), and in-memory constructs.
- When asserting complex structures, use pretty_assertions for readable diffs (import from workspace deps).
- Keep public API behavior covered; for internal details, test via public interfaces where practical.

## Code style and linting
- Format code:
  - cargo fmt --all
- Lint (fast pass):
  - cargo clippy -q -p <crate> --all-features -- -D warnings
- Lint (workspace): may be noisy if some crates are WIP:
  - cargo clippy --workspace --all-features -- -D warnings
- Key lints configured in root Cargo.toml (selected):
  - Rust: unsafe_code=warn, missing_docs=warn, rust_2018_idioms=warn, unwrap_used=warn, expect_used=warn, etc.
  - Clippy: pedantic=warn with several allows; restriction and correctness lints enabled.
  - Rustdoc: all=warn.

## Build/test tips on Windows
- If you observe a transient toolchain error (e.g., rustc component not applicable), re-run the command. Ensure rustup is healthy:
  - rustup show
  - rustup update stable
- Use PowerShell path separators (\\) when invoking tools manually.

## Before submitting changes (checklist)
- cargo fmt --all
- cargo check -p <crate> (or cargo build) for the crates you touched
- cargo clippy -p <crate> -- -D warnings
- Run targeted tests for affected crates:
  - cargo test -p <crate> [--test <file_stem>] [test_name]
- Keep changes localized; avoid workspace-wide churn unless necessary.

## Known caveats at the time of writing
- A full cargo test on the entire workspace may currently fail due to issues in some members (e.g., nebula-parameter). Use selective per-crate testing during development.
- Some crates enable strict documentation lints; changes to public items may require doc updates and/or examples to satisfy missing_docs.


---

# AI Agent Programming Guidelines for Nebula (Junie Edition)

This section defines precise, actionable rules for AI agents (Claude, ChatGPT, Cursor, etc.) contributing to the Nebula workspace. It complements the project guidance above and should be followed strictly.

## Core Directives
- ALWAYS respect existing workspace configuration: Rust 1.89+, Edition 2024, resolver = 3, Windows MSVC toolchain.
- NEVER modify workspace-level Cargo.toml without explicit permission.
- FOLLOW established code patterns in existing crates; prefer copying internal conventions over inventing new ones.
- MAINTAIN strict lint compliance as configured in the workspace (rust/clippy/rustdoc).
- TEST all code changes with targeted tests before claiming completion (per-crate focus).

Context
- Working directory: C:\\Users\\vanya\\RustroverProjects\\nebula
- Target platform: Windows stable-x86_64-pc-windows-msvc
- MSRV: 1.89 (see [workspace.package] rust-version)

## Project Structure Awareness
Workspace members (non-exhaustive):
- crates/nebula-{action,config,core,credential,error,log,memory,parameter,resilience,resource,system,value}

Crate dependency policy
- Check [workspace.dependencies] in root Cargo.toml before adding deps; use dep.workspace = true where possible.

## Development Workflow

1) Before making changes
- cargo check -p <target-crate>
- cargo test -p <target-crate> --lib
- cargo test -p <target-crate> --tests

2) Code implementation rules
- Public modules and items must have documentation (missing_docs is enabled in workspace lints).
- Error handling: use nebula_error::{Error, Result}; no unwrap/expect; propagate with ? and convert errors explicitly.
- Logging: prefer nebula_log/tracing over println!; provide helpful context.
- Async: avoid blocking in async contexts; use tokio::fs for I/O.
- Imports: standard -> external -> workspace -> current crate -> super/self.
- Item order within modules: type aliases, constants, traits, structs/enums, impls, functions.
- Keep names descriptive; avoid cryptic abbreviations.

Example skeleton
    //! Module docs (required)
    use nebula_error::Result;

    /// Example public struct
    pub struct Example {
        configuration: Config,
    }

    impl Example {
        /// Constructor
        pub fn new(configuration: Config) -> Self { Self { configuration } }

        /// Example fallible method
        pub async fn process(&self, input: Value) -> Result<Output> {
            let validated = self.validate(input)?;
            let out = self.transform(validated)?;
            Ok(out)
        }
    }

Error handling dos and don'ts
```rust
use nebula_error::{Error, Result};
let value = maybe.ok_or_else(|| Error::custom("missing value"))?; // DO
let v = maybe.unwrap(); // DON'T
```

3) Testing strategy
- Integration tests: crates/<crate>/tests/<name>.rs (each file is a separate test binary) — use for public API.
- Unit tests: alongside code with #[cfg(test)] mod tests { .. } — use for internal logic.
- Running:
  - cargo test -p <crate>
  - cargo test -p <crate> --test <file_stem>
  - cargo test -p <crate> name_substring
- Prefer deterministic tests; avoid timing-sensitive sleeps; use tokio time utilities.

4) Code quality checks (before commit)
- cargo fmt --all
- cargo clippy -p <crate> --all-features -- -D warnings
- cargo doc -p <crate> --no-deps

## Task-Specific Guidance

Adding features
- Reuse existing patterns from similar crates; keep API consistent.
- Add comprehensive tests (happy/error paths).
- Update crate docs if public API changes.

Refactoring
- Maintain API compatibility unless approved; keep refactors separate from features.
- Run all tests for the affected crate(s) and ensure docs build.

Creating new crates (pattern)
```toml
[package]
name = "nebula-newcrate"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
authors.workspace = true

[dependencies]
nebula-core.workspace = true
tokio = { workspace = true, features = ["full"] }

[lints]
workspace = true
```

Debugging failing tests
- Use: cargo test -p <crate> -- --nocapture
- Enable logs: set RUST_LOG=debug (run tests without -q)
- Consider cargo expand for macro debugging (local tooling requirement).

## Pitfalls to Avoid
- unwrap()/expect() in production code.
- Ignoring Results; handle or propagate with ?.
- Vague error messages; provide context.
- Synchronous I/O in async code.
- Hardcoded absolute paths.
- println! for logging.

## Performance and Memory
- Use tokio::spawn appropriately; avoid blocking the async runtime.
- Prefer references; clone only when needed and justified.
- Prefer iterator adapters; collect only when necessary.

## Progress Reporting Template
- Use status updates that list Completed, In Progress, and Next Steps when interacting via Junie.

Success criteria for completion
- All targeted tests pass: cargo test -p <crate>
- No clippy warnings: cargo clippy -p <crate> -- -D warnings
- Code formatted: cargo fmt --all --check
- Docs build: cargo doc -p <crate> --no-deps
- No unwrap()/expect(); all public items documented; robust error handling.
