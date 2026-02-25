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
| `000_extensions` | PostgreSQL extensions (uuid-ossp, pgcrypto, pg_trgm) |
| `001_organizations` | Organizations |
| `002_users` | Users |
| `003_memberships` | Org members, API keys, sessions |
| `004_audit_log` | Audit log |
| `005_tenants` | Multi-tenancy: tenants |
| `006_tenant_config` | Tenant settings and variables |
| `007_credentials` | Credential types, seed data, encrypted credentials |
| `008_resources` | Resources, health checks, cleanup function |
| `009_workflows` | Workflows |
| `010_workflow_versions` | Workflow versions and triggers |
| `011_workflow_sharing` | Cross-organization workflow sharing |
| `012_executions` | Executions |
| `013_execution_nodes` | Node runs and execution logs (with 90-day cleanup) |
| `014_execution_lifecycle` | Idempotency keys and approvals |
| `015_registry` | Action/node definitions, packages |
| `016_cluster` | Cluster nodes, workers, distributed locks, work queue |
| `017_roles` | Custom RBAC roles |
| `018_projects` | Projects and folders |
| `019_teams` | Teams and team members |
| `020_sharing_acl` | Shared workflows/credentials, object-level ACL |
| `021_service_accounts` | Service accounts and their API keys |
| `022_sso` | SSO providers (SAML/OIDC/LDAP) and sessions |
| `023_scim` | SCIM provisioning, external identities |
| `024_tags` | Tags and workflow-tag assignments |
| `025_mfa` | MFA methods (TOTP, WebAuthn, backup codes) |
| `026_invitations` | Pending email invitations |
| `027_project_variables` | Per-project environment variables |
| `028_permission_cache` | Permission cache and auto-provisioning triggers |
