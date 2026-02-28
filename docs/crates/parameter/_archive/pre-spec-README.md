# nebula-parameter

`nebula-parameter` defines node input schemas for Nebula.

It describes:
- parameter kinds and capabilities
- metadata (name/key/hints/required/sensitive)
- declarative validation rules
- conditional display logic
- runtime value container and diff/snapshot utilities

## Role in Platform

For a Rust n8n-like platform, this crate is the canonical schema layer between:
- action/plugin/credential parameter declarations
- UI form rendering
- engine-side validation before execution

## Main Surface

- `ParameterDef` (tagged enum for all parameter types)
- `ParameterKind` and `ParameterCapability`
- `ParameterMetadata`
- `ParameterCollection` (`validate`, lookup, mutation, iteration)
- `ParameterValues` (flat key->JSON value map with snapshot/diff)
- `ValidationRule` (declarative constraints)
- display APIs (`ParameterDisplay`, `DisplayRuleSet`, `DisplayCondition`)

## Dependencies

- `nebula-validator` for rule execution in collection validation

## Document Set

- [ARCHITECTURE.md](../ARCHITECTURE.md)
- [API.md](../API.md)
- [DECISIONS.md](../DECISIONS.md)
- [ROADMAP.md](../ROADMAP.md)
- [PROPOSALS.md](../PROPOSALS.md)
