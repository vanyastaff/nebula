# nebula-core
Foundation shared by every other crate — IDs, domain keys, scope system, and shared traits.

## Invariants
- Must stay small and stable. Imported by all 25 crates — changes cascade everywhere.
- Adding new ID types is safe. Changing trait signatures requires approval.
- Keys normalized lowercase ASCII (`.`, `_`, `-` separators), validated by `domain-key` at construction.

## Key Decisions
- `NodeId` = graph position, `ActionKey`/`PluginKey` = type identity. Multiple nodes can share an `ActionKey`.
- `BaseCtx` trait: unified base context for all subsystems (scope, org/execution/node/user/tenant/workflow IDs). Cancellation excluded (requires tokio-util) — downstream extension traits add it.
- `AuthScheme` trait: `const KIND: &'static str` (replaced former `pattern()` method; `AuthPattern` enum removed). `()` implements it for credential-free resources.
- `SecretString` + `serde_secret` live here — usable by any crate without depending on credential.
- `CredentialEvent` lives here to avoid credential↔resource peer dependency.
- `OwnerId` removed from core ID types.

## Traps
- `NodeId` vs `ActionKey` confusion — see Key Decisions above.
- `KeyParseError` vs `UuidParseError` — keys and IDs have different parse error types.
- `deps` module is shared — don't duplicate graph logic elsewhere.

## Relations
- Imported by every nebula crate. No nebula deps.

<!-- reviewed: 2026-04-07 — BaseCtx added, AuthPattern removed, OwnerId removed, ScopeLevel derives Default -->
