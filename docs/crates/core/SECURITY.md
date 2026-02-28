# Security

## Threat Model

- **Assets:** ID integrity (no ID spoofing), scope semantics (correct lifecycle/access boundaries), validated keys (no injection via PluginKey)
- **Trust boundaries:** Core is a library; trust boundary is at API/storage/runtime layers. Core provides primitives; consumers enforce policy.
- **Attacker capabilities:** Malicious input (invalid UUIDs, oversized keys), crafted serialized data; core validates structure, not business policy.

## Security Controls

- **Authn/authz:** None in core. Core provides `UserId`, `TenantId`, `RoleScope`; enforcement is in api/credential/runtime.
- **Isolation/sandboxing:** None. Core is synchronous, in-process.
- **Secret handling:** None. Core does not handle secrets; `CredentialId` is an opaque reference.
- **Input validation:** `PluginKey` normalizes and validates (length, charset); `Id::parse` rejects invalid UUIDs; `utils::is_valid_identifier` for identifiers.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|------------|-----------|----------|
| PluginKey injection (e.g., path traversal) | `PluginKey` allows only `a-z` and `_`; max 64 chars | N/A | Reject at parse |
| ID confusion (wrong type) | Typed IDs; compile-time separation | N/A | Compile error |
| Scope bypass (access wrong tenant) | `ScopeLevel` hierarchy; consumers must enforce `is_contained_in` | N/A | Consumer responsibility |
| Oversized constants abuse | Constants are fixed; no user input | N/A | N/A |

## Security Requirements

- **Must-have:** PluginKey validation; ID type safety; no secrets in core types
- **Should-have:** Scope containment strictness (P-002); snapshot tests for serialized forms to detect schema drift

## Security Test Plan

- **Static analysis:** `cargo audit`; clippy security lints
- **Dynamic tests:** PluginKey rejection of invalid input; ID parse rejection
- **Fuzz/property tests:** PluginKey normalization idempotency; ID round-trip properties
