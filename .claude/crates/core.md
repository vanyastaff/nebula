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

## Traps
- Confusing `NodeId` with `ActionKey`: multiple nodes can run the same action; they have different `NodeId`s but the same `ActionKey`.
- `domain_key::KeyParseError` vs `UuidParseError` — both exported from prelude; keys and IDs have different parse error types.
- `deps` module (`DependencyGraph` primitives) is shared across crates — don't duplicate graph logic elsewhere.

## Relations
- Imported by every other nebula crate. No nebula deps of its own — only external crates (`uuid`, `domain-key`, etc.).

<!-- reviewed: 2026-03-19 -->
