# Security

## Threat Model

- **Assets:** execution safety (avoid cascading failures); policy configuration integrity; observability data (no secrets).
- **Trust boundaries:** resilience is a library; callers (engine/runtime) are trusted. External config sources (file, env) may be untrusted.
- **Attacker capabilities:** malformed policy config; timing/DoS via retry storms; observability data exfiltration (low risk).

## Security Controls

- **Authn/authz:** N/A — resilience does not authenticate or authorize. Callers enforce access control.
- **Isolation/sandboxing:** resilience runs in-process; sandbox is enforced by `nebula-sandbox` at action boundary.
- **Secret handling:** resilience does not handle secrets; no credential storage or logging of sensitive data.
- **Input validation:** policy config validated via `ResiliencePolicy::validate()`, `RetryPolicyConfig::validate()`; invalid config returns `ConfigError`.

## Abuse Cases

- **Retry storm:** attacker triggers repeated retries to exhaust resources.
  - **Prevention:** retry limits (`max_attempts`), circuit breaker, rate limiter; backpressure via bulkhead.
  - **Detection:** observability hooks; metrics for retry/circuit events.
  - **Response:** circuit opens; bulkhead rejects; operator adjusts limits.

- **Malformed policy config:** attacker supplies invalid policy (e.g., `max_attempts: 0`, negative timeouts).
  - **Prevention:** validation at load time; `ConfigError` on invalid values.
  - **Detection:** config load failures; startup validation.
  - **Response:** reject config; fallback to defaults or fail startup.

- **Timing side channels:** resilience timing (backoff, circuit reset) could leak information.
  - **Prevention:** jitter on backoff; no secret-dependent timing.
  - **Detection:** N/A for typical workflow use.
  - **Response:** N/A.

## Security Requirements

- **Must-have:** no secrets in logs/metrics; validate all policy config; bounded retries and concurrency.
- **Should-have:** rate limiting to prevent abuse; circuit breaker to fail fast under attack.

## Security Test Plan

- **Static analysis:** `cargo audit`; no `unsafe` in resilience (deny unsafe_code).
- **Dynamic tests:** policy validation tests; malformed config rejection.
- **Fuzz/property tests:** optional; fuzz policy deserialization for robustness.
