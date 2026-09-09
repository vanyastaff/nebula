# nebula-plugin — Agent orientation
> Local guide for `crates/plugin/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** In-process plugin registration and immutable compilation — `Plugin`, `ResolvedPlugin`, mutable/frozen registries, and the pure Graph-v1 compiler. The engine owns runtime admission and dispatch.
**Layer:** Business — depends only downward (root AGENTS.md -> Layered Dependency Map).

## Commands

- Derive macro lives in the sibling crate `crates/plugin/macros/` (`nebula-plugin-macros`) — re-exported as `#[derive(Plugin)]`.

## Key files

- `src/lib.rs` — crate facade + public re-exports; module map.
- `src/plugin.rs` — the `Plugin` base trait (`actions()`/`credentials()`/`resources()`/`on_load`/`on_unload`).
- `src/resolved_plugin.rs` — `ResolvedPlugin`: eager component caches; enforces `{plugin.key()}.` namespace invariant + within-plugin dup rejection at construction (ADR-0027).
- `src/registry.rs` — `PluginRegistry`: `PluginKey → Arc<ResolvedPlugin>`; `all_*` / `resolve_*` accessors.
- `src/flavor.rs` — default-public canonical `PluginSet` and `WorkerFlavorRevision` derivation.
- `src/flavor_context.rs` — canonically ordered execution-facing view of a frozen registry.
- `src/manifest.rs` — re-export of `nebula-metadata::PluginManifest`, not a second manifest definition.
- `src/compiler.rs`, `src/compiler_validation.rs`, `src/plan.rs` — pure Graph-v1 compilation and checked recorded plans; `src/compatibility.rs` checks a plan against the exact frozen registry.
- `src/plugin_toml.rs` — existing marker parser; parsing is not signing, authentication, or process-isolation support.

## Conventions & never-do

- `impl Plugin` is the single runtime source of truth for what's registered. Do NOT duplicate `fn actions()`/`fn credentials()`/`fn resources()` in `plugin.toml` (spec theater).
- `PluginManifest` does NOT compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf.
- Registries are in-memory; persistence belongs behind storage ports. The existing `plugin.toml` parser must not grow a second runtime component list or imply signature enforcement. Process/WASM isolation is a non-goal (ADR-0091, canon §12.6).
- Compilation performs no persistence, tenant authorization, or binding resolution. First-party activation and exact execution consume frozen plans, but compiler success alone does not grant runtime admission.
- `PluginSetId` is an independent pin for the normalized registered surface
  (plugin/component/dependency keys and logical semver). The ID alone is not proof of schemas,
  runtime behavior, artifact authenticity, authorization, or a complete frozen registry.
  Artifact digest and runtime contract version are trusted composition-root inputs.
- Flavor fingerprint domains are persisted protocol versions: never change v1 field order, structural semver tags, normalization, or framing in place. Introduce a new domain version and golden vectors.
- Cross-plugin type references need a Cargo dependency; the runtime load/version edge also needs a `PluginManifest` dependency. Freeze validates the manifest graph. Neither dependency source substitutes for the other.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Registry snapshots and dependencies | [resolved_plugin](tests/resolved_plugin.rs), [frozen_registry](tests/frozen_registry.rs). |
| Compilation and persisted identities | [graph_plan](tests/graph_plan.rs), [effect_contract](tests/effect_contract.rs), [worker_flavor_record](tests/worker_flavor_record.rs). |
| Derive or marker parsing | [derive_plugin](tests/derive_plugin.rs), SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs), or [plugin_toml_parse](tests/plugin_toml_parse.rs), according to the changed surface. |

## See also

- `README.md` — full design · canon §3.5/§7.1/§13.1 · ADR-0018, ADR-0027 (in the ADR history) · [docs/INTEGRATION_MODEL.md](../../docs/INTEGRATION_MODEL.md) §7.
