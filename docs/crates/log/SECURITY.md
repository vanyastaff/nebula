# Security

## Threat Model

- **Assets:** Log output (may contain PII, workflow IDs, error details); telemetry export (traces, metrics)
- **Trust boundaries:** Application code → log crate; log crate → writers/OTLP/Sentry
- **Attacker capabilities:** Influence log content via application; read log files if permissions allow

## Security Controls

- **Authn/authz:** N/A; log is a library, not a service
- **Isolation/sandboxing:** Hooks run in process; panic isolation limits impact of malicious hooks
- **Secret handling:** Log crate does not redact secrets; callers must avoid logging credentials (see credential crate)
- **Input validation:** Filter strings validated at init; config deserialization via serde

## Abuse Cases

- **Sensitive data in logs:**
  - Prevention: Documentation and conventions; credential crate guidance
  - Detection: Static analysis for common patterns; manual review
  - Response: Fix call sites; add redaction if needed

- **Log injection (forged fields/messages):**
  - Prevention: Structured logging with typed fields; no raw concatenation in hot path
  - Detection: Audit log format
  - Response: Sanitize at ingestion if required

- **Hook DoS (slow/malicious hook):**
  - Prevention: Panic isolation; future hook budget (P-001)
  - Detection: Hook lag metrics
  - Response: Unregister hook; circuit breaker

## Security Requirements

- **Must-have:** No credential logging in examples/docs; panic isolation for hooks
- **Should-have:** Redaction guidance; hook execution limits

## Security Test Plan

- **Static analysis:** `cargo audit`; clippy security lints
- **Dynamic tests:** Init with malicious filter strings; panicking hook
- **Fuzz/property tests:** Config deserialization fuzz (optional)
