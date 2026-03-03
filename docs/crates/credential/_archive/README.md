# Archive

Legacy and reference material. Key design ideas have been incorporated into the main docs (ARCHITECTURE.md, SECURITY.md, PROTOCOLS.md, etc.).

## Contents (after cleanup)

| Folder | Purpose |
|--------|---------|
| **Examples/** | Step-by-step examples (OAuth2, API Key, Database, LDAP, mTLS, etc.) |
| **How-To/** | Store, retrieve, rotate credentials; configure caching; enable audit logging |
| **Integrations/** | AWS Secrets Manager, Azure Key Vault, HashiCorp Vault, Kubernetes Secrets, Local Storage; Provider comparison; Migration guide |
| **Troubleshooting/** | Common errors, decryption failures, OAuth2 issues, rotation failures, provider connectivity, debugging checklist, scope violations |
| **Getting-Started/** | Quick start, core concepts, installation |
| **Research/** | Kubernetes Secrets best practices |

## Removed (incorporated into main docs)

- **Meta/** — Architecture, technical design, security spec, data model → ARCHITECTURE.md, SECURITY.md
- **Advanced/** — Threat model, rotation policies, compliance, performance → SECURITY.md, RELIABILITY.md, PROPOSALS.md
- **Reference/** — API reference, traits, storage backends → API.md, PROTOCOLS.md
- **Top-level** — Architecture.md, ROADMAP.md, protocol design drafts → ARCHITECTURE.md, ROADMAP.md, PROTOCOLS.md
