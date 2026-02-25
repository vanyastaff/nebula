# Database Migrations

SQL migrations for Nebula's PostgreSQL database, managed by [sqlx](https://github.com/launchbadge/sqlx).

For full conventions, troubleshooting, and contribution guidelines, see [docs/database-migrations.md](../docs/database-migrations.md).

## Quick Start

```bash
# Set the connection string (or use deploy/.env)
export DATABASE_URL="postgres://nebula:nebula@localhost:5432/nebula"

# Run all pending migrations
sqlx migrate run

# Check migration status
sqlx migrate info
```

## Creating a New Migration

```bash
sqlx migrate add descriptive_name
# Edit the generated file in this directory
```

## Migration Index

| Migration | Domain |
|-----------|--------|
| `000000_extensions` | PostgreSQL extensions (uuid-ossp, pgcrypto, pg_trgm) |
| `000001_users_organizations` | Organizations, users, org members, API keys, sessions, audit log |
| `000002_tenants` | Multi-tenancy: tenants, settings, variables |
| `000003_credentials` | Credential types, seed data, encrypted credentials |
| `000004_resources` | Resources, health checks, cleanup function |
| `000005_workflows` | Workflows, versions, triggers, cross-tenant sharing |
| `000006_executions` | Executions, node runs, logs, idempotency, approvals |
| `000007_registry` | Action/node definitions, packages |
| `000008_cluster` | Cluster nodes, workers, distributed locks, work queue |
| `000009_rbac_projects` | RBAC: roles, projects, folders, project members, teams |
| `000010_sharing_acl` | Shared workflows/credentials, object-level ACL |
| `000011_service_accounts` | Service accounts, API keys, project roles |
| `000012_sso_scim` | SSO providers (SAML/OIDC/LDAP), SCIM provisioning |
| `000013_tags_mfa_invitations` | Tags, MFA methods, invitations, project variables |
| `000014_permission_cache` | Permission cache, cache invalidation triggers |
