# nebula-core — Agent orientation
> Agent quick-map for `crates/core/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** The one crate every other crate depends on for shared vocabulary — typed prefixed-ULID identifiers, normalized string keys, the hierarchical scope system, auth-scheme enums, context/accessor contracts, and lifecycle signals.
**Layer:** Cross-cutting / Core (bottom of the stack) — nothing here depends upward; changing an identifier or key cascades workspace-wide.

## Commands
- `cargo check -p nebula-core`
- `cargo nextest run -p nebula-core`  ·  doctests: `cargo test -p nebula-core --doc`
- `task bench:crate CRATE=nebula-core` — runs the `id_parse_serialize` criterion bench (`harness = false`)

## Key files
- `src/lib.rs` — module wiring, re-exports, `prelude`, compile-time key macros (`plugin_key!` etc.)
- `src/id/` — prefixed-ULID identifiers (`ExecutionId` `exe_…`, `WorkflowId` `wf_…`, …) via `domain-key`; all `Copy`, `new/nil/parse`, serde
- `src/keys.rs` — normalized validated string keys (no secret material here — credential wrappers live in `nebula-credential`)
- `src/scope.rs` — `ScopeLevel`/`Scope`/`Principal`/`ScopeResolver` (Global → … → Action)
- `src/context/` — `Context` trait, `BaseContext(Builder)`, capability traits (`HasCredentials`, `HasResources`, …)
- `src/auth.rs` — canonical `AuthScheme` trait + `AuthPattern` enum (re-exported by `nebula-credential`)
- `src/transport_digest.rs` — experimental plugin-set/flavor/artifact transport IDs behind
  `unstable-worker-flavor`; representation only, never hashing policy
- `src/error.rs` — `CoreError`/`CoreResult` (thiserror; no anyhow)

## Conventions & never-do
- This is **vocabulary only**: no validation (`nebula-schema`/`nebula-validator`), no error taxonomy (`nebula-error`), no resilience, no storage/persistence — do not pull those concerns down here.
- Identifiers/keys are stable opaque handles ([L1-§3.10]); changing their representation cascades — extend deliberately, never casually rename or re-encode.
- Transport digest IDs are explicitly unstable until the worker-flavor contract is adopted end to
  end. When enabled, they remain strict lowercase 64-hex wrappers over private 32-byte values.
  Keep hashing, manifests, and capability interpretation out of this crate.
- `SecretString` and credential-related key types must keep `Debug` redacted ([L2-§12.5]) — no secret material in logs or error strings. Use `debug_redacted`/`debug_typed` from `guard`.
- ID types use `domain-key` (prefixed ULIDs) — never add a direct `uuid` dependency or invent a per-type newtype.
- `CredentialId` is defined in this crate (`src/id/types.rs`); `CredentialEvent` vocabulary lives in `nebula-credential`. `AuthScheme`/`AuthPattern` are canonical *here* and re-exported there.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.
- Library code uses typed `thiserror`/`CoreError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, identifier conventions, prelude usage
- Canon: `docs/PRODUCT_CANON.md` §3.10 (shared vocabulary), §12.5 (secrets/redaction) · `docs/INTEGRATION_MODEL.md` §1
