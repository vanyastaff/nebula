# Security

## Threat Model

### Assets

- Stored data (workflows, executions, binary blobs)
- Keys (may reveal structure: workflow:*, execution:*)
- Connection credentials (DB, Redis, S3)

### Trust Boundaries

- Storage is in-process; backend connections are out-of-process
- Backend credentials (DATABASE_URL, Redis URL, S3 credentials) are trusted
- Stored values are not encrypted by storage; encryption is consumer responsibility (e.g. credential)

### Attacker Capabilities

- **SQL injection:** If key/value used in raw SQL; use parameterized queries
- **Access to backend:** Compromised DB/Redis/S3 exposes all data
- **Key enumeration:** List/scan could leak key patterns

## Security Controls

### Authn/Authz

- Storage does not perform authn/authz. Backend (Postgres, Redis, S3) has its own access control. Application configures connection with appropriate credentials.
- Consumers responsible for scoping (e.g. tenant prefix in key).

### Isolation/Sandboxing

- Storage is a library; no sandboxing. Backend connections use configured credentials.
- Multi-tenant: key namespace (e.g. `tenant:{id}:workflow:{id}`) enforced by consumer.

### Secret Handling

- **Rule:** Storage does not encrypt values. For secrets, use nebula-credential (StorageProvider with encryption).
- Connection strings and API keys: environment variables or secret manager; never in code.

### Input Validation

- Key/value not validated by storage. Consumer validates. For SQL backends, use parameterized queries to prevent injection.
- Large values: consider size limits to prevent DoS.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|-------------|------------|----------|
| Key injection (SQL) | Parameterized queries | — | — |
| Unbounded list | Pagination; limit | — | — |
| Connection exhaustion | Pool limits | Pool metrics | Alert |
| Sensitive data in plaintext | Consumer uses credential crate | Audit | Encrypt |

## Security Requirements

### Must-Have

- Parameterized queries for Postgres
- No plaintext secrets in storage (use credential for secrets)
- Connection credentials from env/config

### Should-Have

- Key size limit
- Value size limit (configurable)

## Security Test Plan

- **Static analysis:** cargo audit; no unsafe
- **Dynamic tests:** Verify parameterized queries; no raw string interpolation
- **Fuzz:** Optional; key/value fuzz for injection
