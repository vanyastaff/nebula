# Credential Systems API Survey

## n8n — Declarative Credential Types

- TS class implements `ICredentialType`: `name`, `properties[]`, `authenticate`, `test`
- `extends: ['oAuth2Api']` — base owns token refresh/redirect/storage
- `IAuthenticateGeneric` injects creds into headers/qs/body via templates; OR imperative fn
- Declarative `test: { request, rules }` — HTTP probe, no custom code
- Engine decrypts AES-256 blob, calls `authenticate()` on request; node never sees secrets

## Windmill — JSON Schema Resource Types

- `ResourceType { name, schema: Option<Value> }` — JSON Schema, created via API
- Variables (scalar secrets) vs Resources (structured creds) — separate concepts
- Runtime: function param type name matches resource type; framework injects
- Per-workspace AES-256 + salt; OAuth2 via platform-level `is_oauth` flag

## Temporal — No Credential System

- `PayloadCodec { encode, decode }` encrypts all workflow data (AES-GCM, key rotation)
- Transparent to workflow code; secrets management is app responsibility

## Key Takeaways for Nebula

1. `authenticate` separating credential-from-request = actions stay credential-agnostic
2. Declarative test-connection (n8n) >> custom test code
3. OAuth2 as inheritable base (n8n) > platform flag (Windmill)
4. Temporal's PayloadCodec is orthogonal — relevant for encryption layer only
