# nebula-component — Agent orientation
> Local guide for `crates/component/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** The contract shared by every integration component: `Component` (`Kind`, `Config`, `KEY`, `metadata()`), the closed set of kinds, the typed `key_of`, and the shared catalog admission `admit`.
**Layer:** Core — depends only downward (`nebula-core`, `nebula-metadata`, `nebula-schema`) and on `domain-key`, `serde`, `tracing`; no upward deps.

## Key files

- `src/component.rs` — `Component`, `KeyOf`, `key_of`, `admit`
- `src/kind.rs` — sealed `ComponentKind` and the `Action` / `Resource` / `Credential` markers

## Conventions & never-do

- Contracts only. No runtime, no storage, no registry lives here; a type that needs a runtime handle belongs in the crate that owns that runtime.
- `ComponentKind` is sealed. Adding a kind is an architecture change, not a refactor.
- Do not add a kind-specific item to `Component`. If only actions need it, it goes on `Action`.
- `Component::metadata()` returns the shared `nebula_metadata::Metadata`. Do not introduce per-kind metadata types; a fact that is part of a kind's contract (ports, auth pattern) is declared on the kind trait and recorded in that kind's definition.
- `key_of` keeps its `const` assertion: an invalid `KEY` literal must stay a build error.
- `src/lib.rs` includes this crate's README as rustdoc, so README examples are doctests.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| `Component`, `key_of`, `admit` | [contract](tests/contract.rs) and the README doctest. |

## See also

- `docs/INTEGRATION_MODEL.md` — how the kinds connect
- `crates/metadata/AGENTS.md` — `Metadata`, `BaseDefinition`, admission budgets
