# nebula-core
Foundation shared by every other crate — IDs, domain keys, scope system, and shared traits.

## Invariants
- Must stay small and stable. It is imported by all 25 other crates — changes cascade everywhere.
- Adding new ID types is safe. Changing existing trait signatures is not (requires approval).
- Keys (`PluginKey`, `ActionKey`, `ParameterKey`, etc.) are normalized lowercase ASCII with only `.`, `_`, `-` as separators. Validated by `domain-key` crate at construction.

## Key Decisions
- `NodeId` = graph position, `ActionKey`/`PluginKey` = type identity. Multiple nodes can share an `ActionKey`.
- Compile-time key construction via `plugin_key!`, `action_key!`, etc. macros.
- `ScopeLevel` hierarchy: Global → Organization → Project → Workflow → Execution → Action.
- `AuthScheme` trait: contract between credential and resource crates. `()` = no auth. Declares `fn pattern() -> AuthPattern` (`const KIND` removed).
- `AuthPattern` enum (`#[non_exhaustive]`): `NoAuth` + 12 auth categories + `Custom`. `()` maps to `NoAuth`.
- `SecretString` + `serde_secret` live here — usable by any crate without depending on credential.
- `CredentialEvent` lives here (not in nebula-credential) so both emitter and consumer avoid peer dependency. Uses typed `CredentialId` (Copy), no EventBus dependency.

## Traps
- Confusing `NodeId` with `ActionKey`: multiple nodes can run the same action; they have different `NodeId`s but the same `ActionKey`.
- `domain_key::KeyParseError` vs `UuidParseError` — both exported from prelude; keys and IDs have different parse error types.
- `deps` module (`DependencyGraph` primitives) is shared across crates — don't duplicate graph logic elsewhere.

## Relations
- Imported by every other nebula crate. No nebula deps of its own — only external crates (`uuid`, `domain-key`, `zeroize`, `serde`, etc.).

<!-- reviewed: 2026-04-07 — AuthPattern.NoAuth added, () maps to NoAuth -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
