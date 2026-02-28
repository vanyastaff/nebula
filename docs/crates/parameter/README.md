# nebula-parameter

Node input schema definition for Nebula workflow platform.

## Scope

- **In scope:**
  - Parameter kinds and capabilities (19 variants: Text, Number, Select, Object, List, Mode, etc.)
  - Metadata (key, name, hints, required, sensitive)
  - Declarative validation rules (min, max, pattern, OneOf, etc.)
  - Conditional display logic (show_when, hide_when)
  - Runtime value container (`ParameterValues`) with snapshot/diff utilities
  - Schema collection and validation pipeline
- **Out of scope:**
  - UI widget dimensions, styling, layout
  - Expression resolution (handled by `nebula-expression`)
  - Credential resolution (handled by `nebula-credential`)
  - Workflow orchestration

## Current State

- **Maturity:** Stable schema layer; used by action, credential, engine, macros, sdk
- **Key strengths:** JSON-serializable schema, capability-based kind semantics, error aggregation, recursive containers
- **Key risks:** Raw JSON values push type mismatch detection late; display rule cycles not preflighted

## Target State

- **Production criteria:** Stable error codes, deterministic validation order, schema lint pass, typed value layer (optional)
- **Compatibility guarantees:** Patch/minor preserve API; breaking changes in MIGRATION.md

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
