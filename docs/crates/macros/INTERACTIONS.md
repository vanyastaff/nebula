# Interactions

## Ecosystem Map

**nebula-macros** provides proc-macros only. It depends on syn, quote, proc_macro; no nebula-* at compile time for macro crate itself. Generated code depends on nebula-action, nebula-resource, nebula-plugin, nebula-credential (and core) as specified by the author's Cargo.toml. Downstream: authors (and nebula-sdk which re-exports macros) use the derives; engine, runtime, plugin registry, credential manager consume the types that implement the traits.

### Upstream (macro crate build deps)

- **syn, quote, proc_macro** — parsing and code generation. No nebula-* in macro crate (generated code references them in author's crate).

### Downstream (consume macro output)

- **Action authors** — use derive(Action); generated type used by engine/runtime/action registry.
- **Resource/Plugin/Credential authors** — use derive(Resource), derive(Plugin), derive(Credential); generated types used by resource manager, plugin registry, credential manager.
- **nebula-sdk** — re-exports macros; authors often depend on sdk which depends on macros.

### Contract

- **action crate:** Generated impl Action must satisfy Action trait (metadata(), etc.). Action crate does not depend on macros; authors depend on both.
- **resource/plugin/credential:** Same: generated impl must satisfy respective trait.

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|--------------------|-----------|----------|------------|------------------|-------|
| authors → macros | in | TokenStream (source) | compile-time | compile_error | |
| macros → (generated) action/resource/plugin/credential | out | impl Trait | N/A | compile if wrong | generated code in author crate |
| sdk → macros | in | re-export | N/A | N/A | |

## Runtime Sequence

1. Author writes #[derive(Action)] etc.; compiler invokes macro; macro expands to impl and any helpers.
2. Author's crate compiles; type implements trait. Engine/runtime/registry load and use the type.
3. No runtime interaction in macro crate; all is compile-time.

## Cross-Crate Ownership

- **macros** owns: attribute grammar, expansion output shape, diagnostic messages.
- **action/resource/plugin/credential** own: trait definition and semantics; macro output must conform.

## Versioning and Compatibility

- Macro version X should document compatible versions of action, resource, plugin, credential (e.g. "macros 0.2 works with action 0.3"). Breaking trait in action etc. may require macro release (minor or major) to regenerate compatible code. Breaking attribute or output = macro major + MIGRATION.md.
