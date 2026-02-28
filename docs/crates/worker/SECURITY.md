# Security

## Security Goals

- isolate untrusted action execution
- enforce tenant and credential boundaries
- prevent secret leakage in logs/telemetry
- provide auditable execution traces

## Threat Model

- malicious action/plugin code attempting sandbox escape
- credential exfiltration via process env/files/network
- noisy neighbor resource exhaustion across tenants
- forged or replayed task messages

## Controls

- sandbox policy:
  - deny-by-default network/filesystem/syscalls
  - per-task uid/gid/process namespace isolation
- credential handling:
  - short-lived scoped retrieval
  - redaction at error/log boundary
- queue integrity:
  - signed/authenticated task channel
  - lease ownership verification on ack/nack
- resource containment:
  - memory/cpu/io hard caps + watchdog kill on violation

## Security Requirements

- worker must fail-closed when sandbox policy cannot be applied.
- secrets must never be persisted in plaintext worker logs.
- crash dumps and diagnostics must redact tenant-sensitive payloads.
- every execution must carry actor/tenant trace metadata for audits.

## Security Testing

- sandbox escape attempt suite
- secret leakage assertions on logs/errors/traces
- unauthorized lease ack/nack rejection tests
- DoS resilience tests for quota enforcement
