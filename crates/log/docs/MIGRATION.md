# Migration

## Versioning intent

`nebula-log` evolves with additive changes whenever possible.
Breaking changes must include migration notes and replacement guidance.

## Contracts to preserve

- initialization APIs (`auto_init`, `init`, `init_with`)
- startup precedence semantics
- documented writer failure policies
- config schema compatibility expectations

## Deprecation approach

1. Deprecate old API/config path.
2. Add replacement API/config path.
3. Keep transition window.
4. Remove only after migration guidance is published.

## Upgrade checklist

1. Review `crates/log/CHANGELOG.md`.
2. Re-run crate tests and clippy gates.
3. Validate startup with real deployment env.
4. Validate log ingestion and telemetry in staging.
5. Confirm no breaking changes in structured field names consumed by downstream systems.
