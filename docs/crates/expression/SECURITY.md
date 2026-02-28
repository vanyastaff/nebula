# Security

## Threat Model

- assets:
  - integrity of workflow data transformations
  - availability of runtime evaluation path
  - confidentiality of context data referenced in expressions
- trust boundaries:
  - untrusted expression/template input from users/config
  - evaluation context sourced from runtime state
  - function registry and regex execution paths
- attacker capabilities:
  - craft pathological expressions for CPU/memory abuse
  - trigger unsafe regex backtracking patterns
  - probe context variables/functions for unauthorized data access

## Security Controls

- authn/authz:
  - expression crate does not perform user auth; caller must provide authorized context only.
- isolation/sandboxing:
  - recursion depth cap and template expression count cap.
  - regex safety checks to mitigate ReDoS patterns.
- secret handling:
  - engine should not log raw sensitive values by default.
  - callers must avoid exposing secret-bearing context fields unnecessarily.
- input validation:
  - strict parse/type/function validation before producing results.

## Abuse Cases

- case: regex ReDoS payload.
  - prevention: pattern length and nested-quantifier safety checks.
  - detection: regex error telemetry.
  - response: reject expression and alert.
- case: deep recursive/complex expression DoS.
  - prevention: recursion depth limits and potential cost budget controls.
  - detection: eval error spikes/timeouts in telemetry.
  - response: fail-fast and throttle offending source.
- case: unauthorized context data access.
  - prevention: caller-provided context scoping and function policy controls.
  - detection: audit failed variable/function lookups when policy-enabled.
  - response: deny and investigate context provisioning.

## Security Requirements

- must-have:
  - evaluator safety guardrails enabled in production builds.
  - deterministic rejection for malformed/unsafe expressions.
  - no implicit exposure of hidden context fields.
- should-have:
  - policy-driven function allowlist.
  - configurable expression cost budget.

## Security Test Plan

- static analysis:
  - clippy/lint checks and dependency audits.
- dynamic tests:
  - regex abuse tests, recursion/limit tests, context isolation tests.
- fuzz/property tests:
  - parser fuzzing and randomized expression/evaluator invariants.
