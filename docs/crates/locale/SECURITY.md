# Security

## Threat Model

- assets:
  - integrity of user-visible localized messages
  - confidentiality of interpolation data passed to translators
  - trust in locale negotiation and selection process
- trust boundaries:
  - request-provided locale headers/params
  - translation catalogs from filesystem or remote sources
  - interpolation parameters from runtime/action/validator errors
- attacker capabilities:
  - inject malformed locale tags or catalog content
  - exploit interpolation to leak sensitive data
  - trigger fallback abuse to hide malicious/misleading text

## Security Controls

- authn/authz:
  - locale selection should consume trusted user/profile context, not raw unauthenticated input alone.
- isolation/sandboxing:
  - catalog parsing isolated from business logic and validated before activation.
- secret handling:
  - never pass sensitive secrets directly as localization interpolation parameters.
- input validation:
  - strict locale tag validation and key/parameter schema checks.

## Abuse Cases

- case: malicious locale injection.
  - prevention: whitelist supported locales and normalize tags.
  - detection: invalid-locale metrics.
  - response: fallback to safe default and audit event.
- case: catalog tampering.
  - prevention: signed/verified bundles or trusted deployment pipeline.
  - detection: checksum/version mismatch alerts.
  - response: reject catalog and keep last known good version.
- case: sensitive data leakage via interpolation.
  - prevention: parameter allowlist/redaction policy.
  - detection: log scanning for sensitive patterns.
  - response: rotate secrets and patch message templates.

## Security Requirements

- must-have:
  - supported-locale allowlist enforcement.
  - catalog integrity validation before use.
  - no secret-bearing interpolation by default.
- should-have:
  - signed bundles and provenance metadata.
  - policy controls for strict render failure handling.

## Security Test Plan

- static analysis:
  - lint locale catalogs and key schemas.
- dynamic tests:
  - invalid-locale injection and catalog tamper scenarios.
- fuzz/property tests:
  - locale parser and interpolation validation fuzzing.
