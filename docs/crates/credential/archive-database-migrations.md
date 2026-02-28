# Archived From "docs/archive/database-migrations.md"

# Database Migrations

Full conventions, troubleshooting, and contribution guidelines for Nebula's PostgreSQL migrations managed by [sqlx](https://github.com/launchbadge/sqlx).

## Quick Start

```bash
# Set the connection string
export DATABASE_URL="postgres://nebula:nebula@localhost:5432/nebula"

# Run all pending migrations
sqlx migrate run

# Check migration status
sqlx migrate info

# Revert the last migration (if down scripts exist)
sqlx migrate revert
```

## Creating a New Migration

```bash
# From the repo root
sqlx migrate add descriptive_name
# Edit the generated file in migrations/
```

Migrations are prefixed with a UTC timestamp (`YYYYMMDDHHmmss`) that sqlx uses for ordering. Keep names lowercase and use underscores.

## Migration Dependency Graph

Migrations run in timestamp order. The logical dependency chain is:

```
000  extensions
  |
001  organizations
  |
002  users
  |
003  memberships         (org_members, api_keys, sessions)
  |
004  audit_log
  |
005  tenants
  |
006  tenant_config       (settings, variables)
  |
007  credentials         (types + encrypted storage)
  |
008  resources           (managed connections, health checks)
  |
009  workflows
  |
010  workflow_versions   (versions, triggers, circular FK)
  |
011  workflow_sharing    (cross-org sharing)
  |
012  executions
  |
013  execution_nodes     (node_runs, logs, 90-day cleanup)
  |
014  execution_lifecycle (idempotency, approvals)
  |
015  registry            (action_defs, node_defs, packages)
  |
016  cluster             (nodes, workers, locks, queue)
  |
017  roles               (custom RBAC roles)
  |
018  projects            (projects, folders, project_members)
  |
019  teams               (teams, team_members, backfill FK)
  |
020  sharing_acl         (shared_workflows, shared_credentials, acl_entries)
  |
021  service_accounts    (service_accounts, keys, project_roles)
  |
022  sso                 (SSO providers, sessions)
  |
023  scim                (tokens, external_identities, group_mappings)
  |
024  tags                (tags, workflow_tags)
  |
025  mfa                 (TOTP, WebAuthn, backup codes)
  |
026  invitations
  |
027  project_variables
  |
028  permission_cache    (cache table + invalidation triggers)
```

## Conventions

### Naming

- Files: `YYYYMMDDHHmmss_<name>.sql` (sqlx default)
- Logical names: `NNN_<domain>` where NNN is a zero-padded sequence
- Use singular domain names (e.g., `workflow`, not `workflows`) except where the plural reads more naturally

### File Size

Migrations are kept focused and well under 100 lines each. Split large domain areas into separate files (e.g., `009_workflows` + `010_workflow_versions` + `011_workflow_sharing`) rather than creating monolithic files.

### Style

- All DDL in `UP` direction only (no down scripts in production)
- Use `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`
- Define foreign keys inline with the column where possible
- Add `ON DELETE CASCADE` or `ON DELETE SET NULL` explicitly — never rely on defaults
- Use `gen_random_uuid()` (from pgcrypto) for UUID primary keys
- Use `TIMESTAMPTZ` for all timestamps; store UTC
- Add a `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()` column to every table

### Indexes

- Index every foreign key column
- Add partial indexes for soft-delete patterns (`WHERE deleted_at IS NULL`)
- Use `GIN` indexes for JSONB columns queried with `@>` or `?`
- Name indexes: `idx_<table>_<column(s)>`

### Sensitive Data

- Never store plaintext secrets; use `bytea` with application-level encryption (AES-256-GCM)
- Credential values live in `encrypted_credentials.encrypted_value` (bytea)

## Troubleshooting

### Migration fails mid-run

sqlx wraps each migration in a transaction. A failure rolls back the entire migration; fix the SQL and re-run `sqlx migrate run`.

### "already applied" errors

If a migration was applied outside sqlx (e.g., manually), insert a row into `_sqlx_migrations` to mark it as applied:

```sql
INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
VALUES (<timestamp>, '<description>', NOW(), true, '\x00', 0);
```

### Checksum mismatch

sqlx rejects a migration if the file content changed after it was applied. To fix:
1. Create a new migration that alters the schema
2. Update the checksum in `_sqlx_migrations` only in development (never in production)

## Per-Crate Options

Individual crates may embed their own migrations using `sqlx::migrate!` macro pointing to a subdirectory. Check each crate's `build.rs` or `lib.rs` for embedded migration paths.

| Crate | Embedded migrations |
|-------|---------------------|
| `nebula-storage` | None — uses repo-root `migrations/` |
| `nebula-credential` | None — uses repo-root `migrations/` |

