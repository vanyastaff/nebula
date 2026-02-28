# Security

## Threat Model

- assets:
  - credentials and connection material used by resources
  - tenant/workflow isolation boundaries
  - availability of resource acquisition path
- trust boundaries:
  - untrusted workflow inputs influence requested scope/resource usage
  - external systems (DB/API/queue) are outside trust boundary
  - optional credential provider boundary via feature-gated integration
- attacker capabilities:
  - attempt cross-tenant resource access via forged context
  - trigger pool exhaustion and degrade service availability
  - exploit logging/telemetry paths to expose secrets

## Security Controls

- authn/authz:
  - resource crate enforces scope compatibility, not user identity auth.
  - upper layers must bind `Context` to authenticated principal/tenant.
- isolation/sandboxing:
  - deny-by-default scope containment for incomplete parent chain.
  - quarantine blocks unhealthy resources from further acquisition.
- secret handling:
  - avoid storing raw secrets in events/errors; credential integration should return opaque references where possible.
  - docs and tests require redaction for DSN/token-like fields.
- input validation:
  - `Config::validate` and `PoolConfig::validate` enforce fail-fast constraints.

## Abuse Cases

- case: cross-tenant data access by scope spoofing.
  - prevention: strict containment check and parent chain consistency.
  - detection: audit hook + event anomalies by tenant/workflow dimensions.
  - response: block caller context, quarantine impacted resources, trigger incident workflow.
- case: resource exhaustion attack.
  - prevention: bounded `max_size`, acquire timeouts, caller-level rate limiting.
  - detection: monitor `PoolExhausted` and waiter growth trends.
  - response: degrade non-critical flows, raise capacity, or tighten admission control.
- case: secret leakage in logs/events.
  - prevention: redaction policy for config/error fields and hook guidance.
  - detection: log scanning with secret patterns.
  - response: credential rotation and post-incident patch.

## Security Requirements

- must-have:
  - no cross-tenant scope bypass.
  - no secret material in default event/error payloads.
  - deterministic blocking of quarantined resources.
- should-have:
  - integration with centralized policy engine for context provenance checks.
  - signed audit trail for critical resource lifecycle transitions.

## Security Test Plan

- static analysis:
  - clippy, dependency audit, forbidden-logging checks for secret patterns.
- dynamic tests:
  - scope mismatch/cross-tenant denial tests.
  - quarantine enforcement tests.
  - secret redaction regression tests.
- fuzz/property tests:
  - scope containment property tests.
  - malformed config and identifier fuzzing for validation robustness.
