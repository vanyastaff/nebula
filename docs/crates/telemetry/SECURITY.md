# Security

## Threat Model

- **Assets:** Execution metadata (workflow_id, execution_id, node_id) in events; metric names and values. No credentials or PII in current schema.
- **Trust boundaries:** Telemetry is process-internal; events/metrics do not cross trust boundary in MVP. Export (Phase 3) will push to external endpoints.
- **Attacker capabilities:** If process compromised, attacker can read events/metrics; emit is from trusted engine/runtime only.

## Security Controls

- **Authn/authz:** N/A for in-process telemetry. Export endpoints (future) must be protected.
- **Isolation/sandboxing:** Telemetry runs in same process as engine; no isolation.
- **Secret handling:** ExecutionEvent does not carry secrets. Error strings in NodeFailed/Failed may contain sensitive info — ensure engine/runtime do not include credentials in error messages.
- **Input validation:** Event payloads are constructed by engine/runtime; no external input. Metric names from consumer code; consider sanitization if user-controlled (unlikely).

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|------------|-----------|----------|
| High-volume emit DoS | Fire-and-forget; no blocking. Subscribers must not block. | Monitor subscriber lag; EventBus capacity | Increase capacity; add backpressure (future) |
| Sensitive data in error strings | Engine/runtime must not include secrets in ExecutionEvent::NodeFailed/Failed | Code review; static analysis | Redact in emit layer if needed |
| Metric cardinality explosion | Document naming; avoid high-cardinality labels in registry | Monitor metric count | Bounded Histogram; label limits |

## Security Requirements

- **Must-have:** No credentials in event payloads; error strings must not leak secrets.
- **Should-have:** Export endpoints (future) use TLS; auth for scrape/push.

## Security Test Plan

- **Static analysis:** `cargo audit`; no unsafe in telemetry crate.
- **Dynamic tests:** Verify no panic paths in hot path; fuzz event serialization (optional).
- **Fuzz/property tests:** ExecutionEvent roundtrip; malformed input handling.
