# nebula-core — Agent orientation
> Local guide for `crates/core/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Shared vocabulary — typed prefixed-ULID identifiers, normalized string keys, the hierarchical scope system, auth-scheme enums, context/accessor contracts, and lifecycle signals.
**Layer:** Core / shared-infra — depends on cross-cutting `nebula-error`; changing an identifier or key cascades workspace-wide.

## Commands

- `task bench:crate CRATE=nebula-core` — runs the `id_parse_serialize` criterion bench (`harness = false`)

## Key files

- `src/lib.rs` — module wiring, re-exports, `prelude`, compile-time key macros (`plugin_key!` etc.)
- `src/id/` — prefixed-ULID identifiers (`ExecutionId` `exe_…`, `WorkflowId` `wf_…`, …) via `domain-key`; all `Copy`, `new/nil/parse`, serde
- `src/keys.rs` — normalized validated string keys (no secret material here — credential wrappers live in `nebula-credential`)
- `src/scope.rs` — `ScopeLevel`/`Scope`/`Principal`/`ScopeResolver` (Global → … → Action)
- `src/context/` — `Context` trait, `BaseContext(Builder)`, capability traits (`HasCredentials`, `HasResources`, …)
- `src/auth.rs` — canonical `AuthScheme` trait + `AuthPattern` enum (re-exported by `nebula-credential`)
- `src/transport_digest.rs` — default-public plan, bundle, plugin-set, flavor, and artifact
  transport identities; representation only, never hashing policy
- `src/error.rs` — `CoreError`/`CoreResult` (thiserror; no anyhow)

## Conventions & never-do

- This is **vocabulary only**: local key parsing belongs here, but schema/rule engines (`nebula-schema`/`nebula-validator`), error taxonomy (`nebula-error`), resilience, and persistence do not.
- Identifiers/keys are stable opaque handles ([L1-§3.10]); changing their representation cascades — extend deliberately, never casually rename or re-encode.
- Transport digest IDs are strict lowercase 64-hex wrappers over private 32-byte values. They
  provide type separation and wire representation, not hashing, manifest, authorization,
  compatibility, or capability policy. Their operational consumers remain partial until the
  complete execution contract is adopted end to end.
- `SecretString` and secret-bearing credential wrappers live in `nebula-credential`, not here. `guard` supplies `debug_redacted`/`debug_typed` for those consumers; identifiers are not secret material.
- ID types use `domain-key` (prefixed ULIDs) — never add a direct `uuid` dependency or invent a per-type newtype.
- `CredentialId` is defined in this crate (`src/id/types.rs`); `CredentialEvent` vocabulary lives in `nebula-credential`. `AuthScheme`/`AuthPattern` are canonical *here* and re-exported there.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Identifier and digest encoding | [transport_digest_ids](tests/transport_digest_ids.rs), plus unit tests beside the changed type in [src/id/](src/id/). |
| Shared schema vocabulary | [schema_contracts](tests/schema_contracts.rs); inspect serialization consumers before changing a wire representation. |

## See also

- `README.md` — full design, identifier conventions, prelude usage
- Canon: [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.10 (shared vocabulary), §12.5 (secrets/redaction) · [docs/INTEGRATION_MODEL.md](../../docs/INTEGRATION_MODEL.md) §1
