# Security

## Threat Model

### Assets

- Event payloads (may contain execution_id, workflow_id, resource_id, error messages)
- Event bus process memory (bounded buffer)
- Subscriber processes/tasks receiving events

### Trust Boundaries

- Eventbus is in-process; all emitters and subscribers are trusted (same process)
- Events are not persisted by eventbus; no durable storage
- Distributed eventbus (Phase 4) would introduce network boundary — separate threat model

### Attacker Capabilities

- **In-process:** Malicious or compromised code could emit forged events or subscribe to harvest data
- **Out-of-process:** N/A for Phase 1–3 (single process)

## Security Controls

### Authn/Authz

- **Current:** None; eventbus is internal transport. Emitters and subscribers are same-process components.
- **Future (distributed):** Phase 4 would need authentication for remote subscribers; TLS; optional encryption.

### Isolation/Sandboxing

- Eventbus does not execute user code. Event handlers run in subscriber tasks; sandboxing is responsibility of engine/runtime.
- No eval, no deserialization of untrusted data in eventbus itself.

### Secret Handling

- **Rule:** Event payloads must NOT contain secrets (passwords, tokens, API keys).
- Domain crates (telemetry, resource) must ensure events carry only non-sensitive identifiers and metadata.
- Error messages in events: sanitize to avoid leaking internal details.

### Input Validation

- Event type is generic; validation is responsibility of emitter. Eventbus does not validate payload content.
- Buffer size and policy are configured at construction; no runtime injection.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|-------------|------------|----------|
| Event flood (DoS) | Bounded buffer; BackPressurePolicy; DropOldest/DropNewest | EventBusStats.dropped; metrics | Alert on high drop rate; scale buffer or add back-pressure |
| Forged events | In-process; trusted emitters | N/A | Code review; access control to emitter code |
| Sensitive data in events | Design guideline; code review | Audit event schemas | Remove sensitive fields; document |
| Subscriber lag (resource exhaustion) | Bounded buffer; Lagged skip | subscriber_count; recv latency | Scale subscribers; optimize handlers |

## Security Requirements

### Must-Have

- Events must not contain secrets
- Bounded buffer to prevent unbounded memory growth
- No execution of untrusted code

### Should-Have

- Event schema documentation (what fields are safe to log)
- Sanitization guidance for error messages in events

## Security Test Plan

- **Static analysis:** cargo audit; no unsafe in eventbus
- **Dynamic tests:** Emit flood; verify bounded memory; no panic
- **Fuzz/property tests:** Optional; event payload fuzz for Clone/serialization
