# nebula-core
Foundation shared by every other crate — IDs, domain keys, scope system, and shared traits.

## Invariants
- Must stay small and stable. It is imported by all 25 other crates — changes cascade everywhere.
- Adding new ID types is safe. Changing existing trait signatures is not (requires approval).
- Keys (`PluginKey`, `ActionKey`, `ParameterKey`, etc.) are normalized lowercase ASCII with only `.`, `_`, `-` as separators. Validated by `domain-key` crate at construction.

## Key Decisions
- `NodeId` identifies a *position in the workflow graph*, not the action type. `ActionKey`/`PluginKey` carry type identity. `NodeDefinition.action_key` is the binding.
- Compile-time key construction via `plugin_key!`, `action_key!`, etc. macros — validated at compile time.
- `ScopeLevel` hierarchy: Global → Organization → Project → Workflow → Execution → Action.
- `AuthScheme` trait lives here as a contract type between credential and resource crates. Requires `Serialize + DeserializeOwned + Send + Sync + Clone + 'static`, `const KIND: &'static str`, and provides default `expires_at() -> Option<DateTime<Utc>>`. `()` implements it for credential-free resources.
- `SecretString` + `serde_secret` module live here (moved from nebula-credential). Fundamental secret-safe type usable by any crate (log, auth, config, webhook) without depending on credential. `Zeroize + ZeroizeOnDrop`, `Debug`/`Display` prints `[REDACTED]`, `Serialize` redacts by default, `serde_secret` for transparent storage round-trip.

- `CredentialEvent` lives here (not in nebula-credential) so both emitter (credential) and consumer (resource) can use it without peer dependency. Plain enum, no EventBus dependency — the bus lives in consuming crates.

## Traps
- Confusing `NodeId` with `ActionKey`: multiple nodes can run the same action; they have different `NodeId`s but the same `ActionKey`.
- `domain_key::KeyParseError` vs `UuidParseError` — both exported from prelude; keys and IDs have different parse error types.
- `deps` module (`DependencyGraph` primitives) is shared across crates — don't duplicate graph logic elsewhere.

## Relations
- Imported by every other nebula crate. No nebula deps of its own — only external crates (`uuid`, `domain-key`, `zeroize`, `serde`, etc.).

<!-- reviewed: 2026-04-01 — SecretString moved here from nebula-credential -->

<!-- reviewed: 2026-04-02 -->
