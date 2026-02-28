# Security

## Threat Model

- assets:
  - correctness and integrity of active runtime configuration
  - confidentiality of sensitive values from env/files
- trust boundaries:
  - untrusted source files/overrides/env can enter process
  - remote source variants may cross network trust boundaries
- attacker capabilities:
  - tamper config files/env
  - inject malformed/hostile values
  - trigger frequent reload churn

## Security Controls

- authn/authz:
  - not owned directly by config core; remote adapters must enforce auth.
- isolation/sandboxing:
  - config loading and parse/validation should remain side-effect minimal.
- secret handling:
  - env loader masks sensitive keys in logs by default.
- input validation:
  - validator pipeline gate before activation/reload success.

## Abuse Cases

- case: secrets exposed in logs.
  - prevention: sensitive key detection + redaction.
  - detection: log scanning and secret-leak checks.
  - response: rotate credentials and patch patterns.
- case: malicious file/env override.
  - prevention: strict validation and source precedence discipline.
  - detection: source metadata and change-event auditing.
  - response: rollback to last-valid config, incident review.
- case: config reload DoS.
  - prevention: debounce/backoff strategy at watcher/runtime level.
  - detection: reload frequency/latency alerts.
  - response: disable hot reload temporarily and investigate.

## Security Requirements

- must-have:
  - no activation of invalid config
  - sensitive value redaction in operational logs
  - clear source provenance for loaded data
- should-have:
  - signed remote config support before remote sources are GA
  - policy-based allowlist for overridable keys

## Security Test Plan

- static analysis:
  - clippy/audit checks and dependency review.
- dynamic tests:
  - sensitive logging behavior tests.
  - invalid/malicious input rejection tests.
- fuzz/property tests:
  - parser and path traversal robustness tests.
