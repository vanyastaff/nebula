# Security

## Threat Model

- **Assets:** Idempotency keys; cached results (may contain sensitive data). Keys can reveal execution patterns.
- **Trust boundaries:** Keys generated server-side (node-level) or client-provided (request-level). Client keys must be validated/sanitized.
- **Attacker capabilities:** Replay attacks with stolen keys; key enumeration; cache poisoning.

## Security Controls

- **Authn/authz:** Idempotency does not replace auth; authenticated requests still require auth. Keys are dedup mechanism, not secret.
- **Isolation/sandboxing:** Cached results per key; no cross-tenant key reuse (tenant_id in key or storage scope).
- **Secret handling:** Cached results may contain PII; encrypt at rest for persistent storage; TTL to limit exposure.
- **Input validation:** User-provided keys: length limit (512 chars per schema); character set; no injection.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|------------|-----------|----------|
| Key enumeration | Rate limit; key format opaque | Monitor key patterns | Alert |
| Cache poisoning | Validate cached result schema; checksum | Audit cache writes | Invalidate |
| Replay with stolen key | Keys are not secrets; auth required | — | — |
| Excessive key creation | TTL; cleanup; storage limits | Monitor storage size | Evict |

## Security Requirements

- **Must-have:** No credentials in cached results; tenant isolation for keys.
- **Should-have:** Encrypt cached results at rest; key length limits.

## Security Test Plan

- **Static analysis:** No unsafe; key sanitization.
- **Dynamic tests:** Key length validation; tenant isolation.
- **Fuzz/property tests:** Key format; cache serialization.
