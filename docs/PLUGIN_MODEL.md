# Plugin Model

Scope: detailed companion to `docs/PRODUCT_CANON.md` §7.1.

## Three Layers

- `plugin.toml`: trust + compatibility boundary (SDK constraint, optional stable id, signing payload).
- `Cargo.toml`: Rust package identity and dependency graph.
- `impl Plugin` + `PluginMetadata`: runtime registration/content after load.

Do not duplicate registry contents in TOML when `impl Plugin` is already authoritative.

## Identity Rules

- If `[plugin].id` exists, it is the stable plugin id for discovery.
- If `[plugin].id` is omitted, effective id is `[package].name`.
- Loader/tooling key mapping must be deterministic and documented.

## Discovery and Load Lifecycle

1. Pre-compile discovery reads `Cargo.toml` + minimal `plugin.toml`.
2. Host resolves dependency closure (Cargo graph for Rust-native plugins).
3. Plugin loads and `impl Plugin` registers actions/resources/credentials/locales.
4. Runtime metadata from `PluginMetadata` becomes authoritative for display/catalog.

## Rust-Native vs FFI

- Rust-native plugins follow Cargo-first dependency and build semantics.
- Native plugin binaries are not implicitly ABI-stable across SDK/engine upgrades.
- Binary-stable ABI is an explicit FFI concern (for example, stabby path).

## Tooling Notes

- `cargo-nebula` and CLI flows should validate marker files early.
- Activation should fail fast on missing dependencies or unresolved cross-plugin types.
- Error messages should reference plugin id, package name, and missing provider dependency.