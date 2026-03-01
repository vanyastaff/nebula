# Desktop App Architecture

## Primary approach

Use **Feature-First** structure at the top level.
Each feature may use **DDD-light layers** internally.

This is not "features inside domain". It is "domain inside each feature".

## Target structure

```text
src/
  app/                # app bootstrap, routing, providers
  features/
    connection/
      domain/         # feature rules, types, invariants
      application/    # use-cases, orchestrators, hooks
      infrastructure/ # API/storage/adapters
      ui/             # React components for this feature
    auth/
      domain/
      application/
      infrastructure/
      ui/
  shared/             # reusable technical modules (ui-kit, http, utils)
```

## Dependency rules

- A feature `ui/` can depend on its own `application/` and `domain/`.
- A feature `application/` can depend on its own `domain/` and `infrastructure/`.
- A feature `infrastructure/` can depend on its own `domain/`.
- A feature `domain/` must be framework-agnostic and depend only on itself.
- Cross-feature imports should go through explicit public APIs, not deep internal paths.
- Shared technical code lives in `shared/` and must not contain business rules.

## Practical guidance

- Start simple; use DDD tactics only where business complexity exists.
- Do not introduce heavy DDD ceremony for trivial UI state.
- Keep feature boundaries explicit (connection, auth, workflows, etc.).
- Prefer one HTTP entry point per feature or shared API client abstraction.

## Auth policy

- OAuth is required for all targets (`local`, `remote-selfhosted`, `remote-saas`).
- Supported providers: `Google`, `GitHub`.
- API access must be denied when auth status is not `signed_in`.
- Workspace visibility and permissions are always enforced by backend membership checks.
- Desktop callback deep-link format:
  - `nebula://auth/callback?access_token=<token>&provider=google|github`
  - `nebula://auth/callback?code=<oauth_code>&provider=google|github` (desktop exchanges code via `/auth/oauth/callback`).

## Current state

Current code is organized by global layers (`domain/`, `application/`, `infrastructure/`, `ui/`).
This is acceptable as an intermediate step and should be gradually migrated to `features/*`.
