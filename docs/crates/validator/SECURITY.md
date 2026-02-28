# Security

## Threat Model

- assets:
  - validation contract integrity
  - correctness of error signaling used by API/auth/routing layers
- trust boundaries:
  - untrusted external payloads enter via API/CLI/plugin interfaces
  - validator output is trusted by downstream decision logic
- attacker capabilities:
  - malformed, oversized, nested, adversarial inputs
  - regex/pathological patterns to trigger CPU spikes

## Security Controls

- authn/authz:
  - not owned here; enforced by caller crates.
- isolation/sandboxing:
  - validator logic is pure and does not require privileged side effects.
- secret handling:
  - validators should avoid embedding sensitive values in error messages.
  - sensitive param keys (`password`, `token`, `secret`, `api_key`, `credential`) must be redacted in diagnostics.
- input validation:
  - strict typed validators and explicit combinator semantics.

## Abuse Cases

- case: regex DoS via expensive patterns/input.
  - prevention: controlled regex usage and bounded input size in callers.
  - detection: latency metrics and error-rate alerts.
  - response: fallback rules + request throttling at API/runtime layer.
- case: nested payload causing huge error trees.
  - prevention: fail-fast options and nesting limits in caller policy.
  - detection: monitor error payload size and validation duration.
  - response: cap output and switch to summarized errors.
- case: information leakage through error messages.
  - prevention: standardized safe error text for sensitive fields.
  - detection: security review on new validators.
  - response: patch error code/message mapping.

## Security Requirements

- must-have:
  - deterministic validation with no hidden side effects
  - safe, non-secret-bearing error outputs
  - bounded behavior for adversarial inputs (by policy limits in consumers)
- should-have:
  - fuzz testing for parser-like validators
  - centralized security review checklist for new validator additions

## Security Test Plan

- static analysis:
  - clippy + lint checks + dependency audit.
- dynamic tests:
  - adversarial inputs for regex/json/path validators.
- contract tests:
  - `tests/contract/safe_diagnostics_test.rs` to ensure diagnostics do not leak secrets.
  - `tests/contract/error_tree_bounds_test.rs` to keep nested error structures bounded and parseable.
- fuzz/property tests:
  - property tests for combinator invariants and error tree shape limits.
