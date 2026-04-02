# Scope Enforcement

Multi-tenant credential isolation via `ScopeId` (hierarchical string format: `org:acme/team:eng`).

## Methods That Enforce Scope

| Method | Scope Required | Behavior |
|--------|----------------|----------|
| `retrieve_scoped(id, context)` | Yes | Returns credential only if `context.scope_id` matches `metadata.scope` (exact or hierarchical). Returns `Err(ScopeRequired)` if context has no scope. Returns `Ok(None)` for unscoped credentials or scope mismatch. |
| `list_scoped(context)` | Yes | Returns only credential IDs whose `metadata.scope` matches context (exact or hierarchical). Returns `Err(ScopeRequired)` if context has no scope. Excludes unscoped credentials. |

### Hierarchical Matching

- **Exact**: `org:acme` matches `org:acme`
- **Prefix (parent accesses child)**: `org:acme` matches `org:acme/team:eng` — parent scope can access child credentials
- **No match**: `org:other` does not match `org:acme`

## Methods That Do NOT Enforce Scope

| Method | Notes |
|--------|-------|
| `retrieve(id, context)` | Returns credential by ID regardless of scope. Use when caller has already verified scope at a higher layer (e.g. API resolved ID from scoped list). |
| `list(context)` | Returns all credential IDs from storage. No scope filtering. |
| `list_ids(filter, context)` | Delegates to `StorageProvider::list`; no scope filtering at manager level. |
| `store`, `delete`, `validate`, `get_metadata` | Accept context but do not enforce scope for access control. |

## Storage Providers

**None of the built-in providers enforce scope.** They receive `CredentialContext` in the trait but:

- `MockStorageProvider` — stores/retrieves by ID only; ignores context
- `LocalStorageProvider` — keyed by ID; context not used for access control
- `AwsSecretsManagerProvider`, `HashiCorpVaultProvider`, `KubernetesSecretsProvider` — same

Scope is stored in `CredentialMetadata.scope` when the manager copies `context.scope_id` at store time. Enforcement happens at the **manager** layer in `retrieve_scoped` and `list_scoped`.

## Cache

`CacheLayer` is keyed by `CredentialId` only. A cache hit returns the credential regardless of the caller's scope. For scope-enforced access, use `retrieve_scoped` (which bypasses cache and always fetches from storage to check metadata.scope).

## When to Use Which

- **Multi-tenant API**: Use `list_scoped` for `GET /credentials`, `retrieve_scoped` for `GET /credentials/:id`. Require scope in API context.
- **Internal/single-tenant**: `retrieve` and `list` are fine when scope is not a concern.
- **Workflow execution**: Engine typically resolves credential ID from a scoped list before calling `retrieve`; scope already verified.

## Tests

See `crates/credential/tests/scope_enforcement.rs` for:

- `retrieve_scoped` requires scope in context
- `retrieve_scoped` returns None for scope mismatch
- `retrieve_scoped` returns credential for exact/hierarchical match
- `list_scoped` requires scope, filters by scope
- `retrieve` does not enforce scope (documented behavior)
