# Testing stack (Nebula)

Normative **commands** for fmt / clippy / `cargo deny` live in `docs/QUALITY_GATES.md` and `CLAUDE.md`. This document describes **libraries and runners** we standardize on for unit and integration tests.

## Runner: `cargo-nextest`

- **Config:** [`.config/nextest.toml`](../.config/nextest.toml) — profiles `default`, `ci` (retries, JUnit to `target/nextest/ci/junit.xml`), and `agent`.
- **CI:** [`.github/workflows/test-matrix.yml`](../.github/workflows/test-matrix.yml) uses `taiki-e/install-action@cargo-nextest` and `cargo nextest run -p … --profile ci`.
- **Local:** `cargo nextest run --workspace --profile ci` (or `-p <crate>` while iterating). Prefer nextest over plain `cargo test` for large workspaces (parallelism, flakiness metadata, JUnit).

## Crates (by use case)

| Crate | Role | Typical crates |
|--------|------|----------------|
| **insta** + **`cargo insta review`** | Snapshot (JSON, strings) — run review after changing expected output; **never** snapshot secrets or raw ciphertext. | `nebula-storage` credential crypto shape tests, future API error bodies |
| **pretty_assertions** | Readable `assert_eq!` / `assert_ne!` diffs. | Any test with non-trivial equality |
| **rstest** | Parametric / table tests (`#[rstest]`, `#[case]`). | Crypto, validation matrices |
| **wiremock** | In-process HTTP mock server (async) for `reqwest` and Axum-style clients. | `nebula-credential` (OAuth token JSON), `nebula-api` (outbound HTTP in tests) |
| **mockall** | `#[automock]` trait doubles in **unit** or small integration tests. | Boundaries you own (not third-party `reqwest`); pair with `wiremock` for HTTP |
| **assert_cmd** + **predicates** | Spawn CLI binaries, assert exit code and stdout/stderr. | `nebula-cli` (`tests/cli_smoke.rs`) |
| **assert_fs** | Temp dirs / fixture files with a clear child-file API. | `nebula-storage` (`FileKeyProvider`, filesystem layers) |
| **proptest** | Property-based tests (workspace dependency). | See existing engine / core usages |

## Per-crate baseline (library crates)

Every crate that ships **library** tests under `src/…` and/or `tests/*.rs` and participates in CI (`test-matrix.yml` or obvious siblings) declares a **shared baseline** in `[dev-dependencies]`:

`insta`, `pretty_assertions`, `rstest` — `{ workspace = true }` from the root [Cargo.toml](../Cargo.toml).

**Extra** (only where the shape of tests needs it):

| Addition | Crates (today) |
|----------|----------------|
| `wiremock` | `nebula-api`, `nebula-credential`, **`nebula-engine`** (token / HTTP client tests) |
| `mockall` | `nebula-api`, `nebula-credential` (trait doubles in `tests/*_smoke.rs`) |
| `assert_fs` | `nebula-storage` |
| `assert_cmd`, `predicates` | `nebula-cli` (binary smoke) |

**Baseline only** (no extra test-only HTTP/CLI/fs row): `nebula-action`, `nebula-core`, `nebula-error`, `nebula-eventbus`, `nebula-execution`, `nebula-expression`, `nebula-log`, `nebula-metrics`, `nebula-metadata`, `nebula-plugin`, `nebula-plugin-sdk`, `nebula-resilience`, `nebula-resource`, `nebula-runtime`, `nebula-sandbox`, `nebula-schema`, `nebula-sdk`, `nebula-system`, `nebula-validator`, `nebula-workflow`, `nebula-telemetry`.

**Baseline + extras:** `nebula-api`, `nebula-credential`, `nebula-storage` (see table); `nebula-engine` adds `wiremock`; `nebula-cli` uses `assert_cmd` / `predicates` (binary), not the three-crate baseline.

## Workspace wiring

- Versions live in the **root** [Cargo.toml](../Cargo.toml) under `[workspace.dependencies]`. Individual crates add only `[dev-dependencies]` with `{ workspace = true }`.
- Prefer **using** the baseline in new tests (`use pretty_assertions::assert_eq;`, `#[rstest]`, `insta::assert_*_snapshot!`) rather than growing bespoke helpers.

## Security

- **Credential / encryption tests:** use **insta** only on *structural* or *redacted* values (lengths, version numbers). Do not commit snapshots of base64 keys, tokens, or plaintext secrets.
- **Mock HTTP:** use **wiremock** for TLS-capable `reqwest` against `http://` mock URLs in tests, not `mockall` for the HTTP stack itself.

## See also

- [awesome-rust-testing](https://github.com/hoodie/awesome-rust-testing) (curated ecosystem list)
- [nextest book](https://nexte.st/docs/installation/) — installation
- [insta](https://github.com/mitsuhiko/insta) — redactions and JSON snapshots
