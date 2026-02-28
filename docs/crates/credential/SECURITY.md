# Security

## Threat Model

- **assets:** Encrypted credentials at rest; decrypted secrets in memory; encryption keys; audit trail
- **trust boundaries:** Untrusted: network, storage backends, user input, OAuth2/API providers. Trusted: OS, Rust std, crypto libs, HSM/KMS
- **attacker capabilities:** Storage access; memory dump; MITM; privilege escalation; DoS; log exposure; supply chain

## Security Controls

- **authn/authz:** Context-driven scope isolation; `CredentialContext` with owner/scope; no implicit trust
- **isolation/sandboxing:** Scope enforcement on every operation; tenant isolation via `ScopeId`
- **secret handling:** `SecretString` with zeroization; redaction in Debug/Display; AES-256-GCM at rest
- **input validation:** `CredentialId` validation (path traversal, empty); schema validation via `ParameterCollection`

## Defense-in-Depth Layers

1. **Encryption at rest:** AES-256-GCM; unique nonces; key separation from data
2. **Access control:** `CredentialContext` + scope; least privilege; ownership model
3. **Memory protection:** `zeroize` on drop; `SecretString`; minimal secret lifetime
4. **Audit logging:** Credential lifecycle events; rotation outcomes; scope violations
5. **Network:** TLS 1.3 for provider backends (AWS, Vault); mTLS where supported

## Abuse Cases

- **credential theft from storage:** Prevention: encryption at rest; key separation. Detection: decryption failure alerts. Response: key rotation; re-encrypt
- **scope violation / cross-tenant access:** Prevention: strict scope check on every retrieve. Detection: `ScopeViolation` errors; audit log. Response: fail-fast; alert
- **log exposure of secrets:** Prevention: `SecretString` redaction; no secrets in error messages. Detection: security review. Response: patch; rotate exposed credentials
- **DoS via credential fetch storm:** Prevention: cache; rate limits (P-004). Detection: latency/throughput metrics. Response: backpressure; circuit breaker
- **encryption key compromise:** Prevention: HSM/KMS; key rotation. Detection: key access anomalies. Response: immediate rotation; re-encrypt all; revoke key

## Security Requirements

- **must-have:** Encryption enabled by default; scope enforced on all operations; `SecretString` for in-memory secrets; `#![forbid(unsafe_code)]`
- **should-have:** Constant-time comparison for secrets (`subtle`); rate limiting; audit event coverage; dependency audit in CI

## Security Test Plan

- **static analysis:** `cargo audit`; clippy; no unsafe code
- **dynamic tests:** Scope enforcement; decryption failure paths; invalid ID rejection
- **fuzz/property tests:** CredentialId validation; encrypted payload round-trip

## Comparative Analysis (n8n, Node-RED, etc.)

- **Adopt:** Encrypted storage; scope/tenant isolation; provider abstraction; OAuth2 flows
- **Reject:** Plaintext credential storage; global credential namespace
- **Defer:** HSM integration (Phase 2); rate limiting (P-004)
