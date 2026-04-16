# nebula-plugin

Plugin trait, metadata, descriptors, and in-memory registry for the Nebula workflow engine.

**Layer:** Business
**Canon:** §3.5 (integration model), §7.1 (plugin packaging: `Cargo.toml` + `plugin.toml` + `impl Plugin`)

## Status

**Overall:** `implemented` — the trait + registry + descriptor types are the authoritative plugin registration surface.

**Works today:**

- `Plugin` trait — base trait every plugin implements; provides `actions()`, `credentials()`, `resources()`, `on_load()`, `on_unload()` with default no-ops
- `PluginMetadata` — static descriptor with builder API (human name, icon, categories, long description)
- `ActionDescriptor`, `CredentialDescriptor`, `ResourceDescriptor` — lightweight descriptors returned by the `Plugin` trait methods
- `PluginType` — enum wrapping a single plugin or a versioned set
- `PluginVersions` — multi-version container keyed by `u32`
- `PluginRegistry` — in-memory `PluginKey → PluginType` registry
- `PluginError` — typed error for plugin operations
- 7 unit test markers, **0 integration tests**

**Known gaps / deferred:**

- **Loading / isolation** is out of scope — see `nebula-sandbox`. This crate provides no I/O or FFI.
- **Cross-plugin dependency resolution** (canon §7.1 "plugin A references type from plugin B via `Cargo.toml`") — validated by loader/activation in `nebula-sandbox` + `nebula-plugin-sdk`, not here.
- **Signing / trust boundary** — `plugin.toml` parsing and signature verification are not in this crate; they belong to tooling that reads `plugin.toml` before compile (see canon §7.1).

## Architecture notes

- **Clean trait + registry separation.** Trait in `plugin.rs`, metadata in `metadata.rs`, descriptors in `descriptor.rs`, registry in `registry.rs`, versions in `versions.rs`. One concept per module.
- **Minimal deps.** `nebula-plugin-macros` (proc-macros), `nebula-core`, `nebula-error`. No upward dependencies.
- **`plugin_type.rs` + `versions.rs` could potentially merge** — `PluginType::Versioned(PluginVersions)` is the only consumer. Not a smell, but review when touching either.
- **No dead code or compat shims.**

## What this crate provides

| Type | Role |
| --- | --- |
| `Plugin` | Trait every plugin implements. |
| `PluginMetadata` | Static descriptor with builder API. |
| `PluginType` | Single plugin or versioned set. |
| `PluginVersions` | Multi-version container. |
| `PluginRegistry` | In-memory `PluginKey → PluginType`. |
| `ActionDescriptor`, `CredentialDescriptor`, `ResourceDescriptor` | Lightweight registration descriptors. |
| `PluginError` | Typed error. |

## Where the contract lives

- Source: `src/lib.rs`, `src/plugin.rs`, `src/metadata.rs`, `src/registry.rs`
- Canon: `docs/PRODUCT_CANON.md` §3.5, §7.1
- Satellite: `docs/PLUGIN_MODEL.md`
- Glossary: `docs/GLOSSARY.md` §7 (plugin)

## See also

- `nebula-plugin-sdk` — plugin authoring SDK
- `nebula-sandbox` — plugin loading and isolation
- `nebula-core` — `PluginKey` identity type
