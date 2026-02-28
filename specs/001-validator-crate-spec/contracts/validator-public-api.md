# Contract: Validator Public API (Library Surface)

## Stable Public Interfaces

- `foundation::Validate<T>`
- `foundation::ValidateExt<T>`
- `foundation::Validatable`
- `foundation::ValidationError`
- `foundation::ValidationErrors`
- Built-in validators in `validators::*`
- Core combinators in `combinators::*`
- Prelude exports in `prelude::*`

## Behavioral Contract

- Validation is deterministic and side-effect free.
- `and` short-circuits on first failure.
- `or` succeeds on first successful branch.
- `not` inverts result semantics while preserving error contract boundaries.
- Typed and dynamic entry points (`validate` and `validate_any`) produce equivalent semantics for equivalent values.

## Compatibility Rules

- Minor release:
  - Additive validators/combinators only.
  - Existing error code meaning and field-path format remain stable.
- Major release:
  - Required for behavior-significant semantic changes.
  - Required for redefinition of existing error code meaning.
  - Required for field-path format contract changes.
- Deprecation:
  - Keep deprecated API for at least one minor release unless security-critical.
  - Publish migration mapping before removal.

## Required Contract Tests

- API signature and trait-bound compatibility checks.
- Cross-version fixtures for error code and field-path stability.
- Combinator semantics regression tests for short-circuit and deterministic outcomes.
- Integration fixtures for downstream mappings (`api`, `workflow`, `plugin`, `runtime`).
