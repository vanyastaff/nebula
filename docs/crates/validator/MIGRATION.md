# nebula-validator — Migration Guide

This document provides explicit **Old Behavior → New Behavior** mappings for
all breaking changes made to nebula-validator across major versions.

## v1 (current)

No breaking migrations. v1 is the initial stable release.

## Breaking Change Template

All breaking changes must include an entry in this table:

| Contract Surface | Old Behavior | New Behavior | Consumer Impact | Mitigation |
|---|---|---|---|---|
| _(example)_ `evaluate()` signature | Accepts `&Value` | Accepts `&HashMap<String, Value>` | Must update call sites | Pass `value.as_object().unwrap()` |

## Guidance for Contributors

When introducing a breaking change to nebula-validator:

1. Add an entry to the table above with **explicit old -> new mapping**.
2. Bump the major version in `Cargo.toml`.
3. Update all call sites in the workspace.
4. Add a migration note to `CHANGELOG.md`.

The old -> new mapping must be explicit enough that a consumer can mechanically
update their code without requiring domain knowledge of the change rationale.
