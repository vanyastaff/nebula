# nebula-tenancy — design

| Field | Value |
|---|---|
| Status | Current |
| Reviewed | 2026-07-22 |
| Layer | Business |

## Responsibility

`nebula-tenancy` turns an authenticated actor plus org/workspace binding into the plain
`nebula_storage_port::Scope` required by storage ports. It also provides scope-substituting
decorators for general execution/workflow/resource stores.

`BindingScopeResolver` is fail-closed: a workspace binding is mandatory and malformed or absent
bindings never widen to organization/global authority. Scoped decorators substitute their bound
scope on every call; they do not accept a caller-supplied scope and then compare it after reading.
Wrong-scope and missing rows remain indistinguishable.

## Dependency boundary

The crate depends only on `nebula-core` and `nebula-storage-port`. It owns no `Scope` DTO, SQL,
backend, credential domain type, or deployment configuration.

## Credential relationship

Credential management no longer uses the former metadata-keyed `CredentialScopeLayer` or
credential-specific `ScopeResolver`. Those parallel types were removed rather than aliased.

On the supported authenticated HTTP management path, the credential bounded context's
`CredentialTenantAuthority` derives a mandatory `CredentialOwner`/`CredentialSelector` only after
one decision. `apps/server` implements the first-party authority by revalidating membership/role
for the credential operation and asking `BindingScopeResolver` to reproduce the exact authenticated
scope. This reuses tenancy projection policy without introducing an upward credential dependency
here. Technical service/runtime seams remain below that path until K3 closes the sole-writer and
operation-ledger design.

There is no optional owner, global credential decorator, or `None == admin` path.

## Invariants

- Resolve authenticated bindings fail-closed.
- Substitute scope before backend access; never post-filter a row.
- Do not reveal whether an ID exists in another tenant.
- Keep storage DTOs and backend implementation in their owning lower/exec crates.
- Do not reintroduce credential metadata as an authorization source.
