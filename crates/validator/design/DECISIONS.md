# nebula-validator — Architecture Decisions

## Versioning Policy

### Decision: minor releases are additive only

**Rule:** All minor version releases of nebula-validator must be
**additive-only minor evolution** — no existing validator behavior, error
message format, or public trait may change in a breaking way.

Specifically:
- New validator types may be added in minor releases.
- New `Rule` enum variants may be added in minor releases (the enum is
  `#[non_exhaustive]`).
- Existing `validate()` / `validate_value()` / `evaluate()` semantics are
  frozen until the next major version.

**Rationale:** nebula-validator is imported by many crates in the workspace.
A silent behavioral change would break consumer contracts across multiple
layers simultaneously.

## Rule Evaluation Semantics

### Decision: type mismatch is a silent pass, not an error

When a rule is applied to a JSON value whose type does not match the rule's
expectation (e.g. `MinLength` on a number), the rule **passes silently**.
This is intentional — rules are declarative schema constraints, not type
guards. Type validation is the parameter system's responsibility.

## Error Type Design

### Decision: ValidationError is stack-allocated (≤ 80 bytes)

`ValidationError` is deliberately kept to 80 bytes or fewer to avoid heap
allocation on every validation failure path. All string data uses `Cow<str>`.
