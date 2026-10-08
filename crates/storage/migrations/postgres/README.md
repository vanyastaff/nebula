# PostgreSQL migrations

This catalog creates the PostgreSQL deployment schema. Production startup, the
admitted database operator, and fresh test databases use these same numbered SQL
files. There is no separate schema snapshot.

## Catalog inventory

The eight baseline migrations follow aggregate dependency order. Runtime control
belongs to the executions and dispatch aggregates; it has no separate empty migration.

| File | Aggregate and relations |
|---|---|
| `0001_identity.sql` | Users, sessions, personal access tokens, verification tokens, OAuth states, external identities, MFA enrollment candidates |
| `0002_tenancy.sql` | Organizations, workspaces, organization memberships, workspace memberships |
| `0003_workflows.sql` | Workflows and immutable workflow versions |
| `0004_executions.sql` | Execution state, journal, revision catalogs and references, contract bundles, accepted turns, observation receipts, iteration checkpoints, resume tokens, start reservations, idempotency marks, operation ledger and protocol records |
| `0005_dispatch.sql` | Execution control queue, job dispatch queue, triggers, trigger start reservations, webhook activations |
| `0006_credentials.sql` | Credentials, refresh claims, refresh incidents, pending authorization states |
| `0007_resources.sql` | Resources, status heartbeats and snapshots, shared resources, subscriptions, source leases, events, deliveries, execution handoffs |
| `0008_platform.sql` | HTTP response replay cache and PostgreSQL shared GCRA rate-limit state |
| `0009_tenant_provisioning_receipts.sql` | Permanent provisioning request receipts and identity seals for existing organizations |

`credential_migration_catalog` checks this inventory against the SQL files, including
future additions. The baseline is a fixed named prefix, not the maximum catalog size.

Migration 0009 retains historical provisioning acceptance independently of tenant
purge. Stop older server/worker versions before applying it; older writers do not
participate in the receipt protocol. Existing organizations are sealed without
fabricating an original request; remove their `NEBULA_BOOTSTRAP_*` configuration
before starting the upgraded server.
Ordinary startup and existing tenant data remain unchanged. New accepted requests
replay from receipts and never restore revoked grants or deleted tenants.

## Dialect and ownership

The [database standard](../../docs/database-standard.md) defines names, constraints,
ownership, and deletion rules. PostgreSQL uses `TIMESTAMPTZ` for microsecond instants,
`JSONB` for documents, `BOOLEAN` for flags, and `BYTEA` for opaque binary identities.
Tenant entity identifiers use the types their adapters expose; they are not all binary.
Foreign keys enforce ownership and cascade on permanent purge. Soft-deletable parent
liveness is checked by the adapter in its transaction.

Shared GCRA state is deliberately PostgreSQL-only: `rate_limits` and
`rate_limit_reservations` allow limits across workers. Their nanosecond scheduling
coordinates and wrapping mutation identity retain the algorithm's exact representation.
These tables are defined in the shared platform migration version; they create no gaps
in the SQLite catalog. SQLite deployments use process-local rate limits.

## Applying and extending the catalog

Use `task db:migrate` for the admitted PostgreSQL operator. It checks that
`_sqlx_migrations` is a canonical catalog prefix before applying pending migrations,
under the setup lock, and verifies the resulting head. Unknown versions, edited
checksums, failed migrations, and foreign unledgered schemas fail closed.

After this baseline, append immutable `NNNN_description.sql` files to both backend
catalogs with the same version and slug. Review admission safety and record the review
before raising `migration_catalog::REVIEWED_HEAD`. Update both README inventories.
Backend-specific objects live inside their shared numbered migration; do not reserve
version gaps. Never edit an applied migration to change the schema.

## Disposable development databases

The pre-baseline development catalog is incompatible and is rejected; there is no
automatic ledger adoption or migration-floor exception. `task db:reset` drops and
recreates the local PostgreSQL database and applies the current catalog, destroying
its data. Use it only for disposable development databases. A database containing
valuable data requires a separately designed migration plan.

## Backend parity

The paired SQLite catalog defines the same shared logical relations and foreign-key
actions. `schema_parity_postgres` compares their physical schemas with explicit
dialect exceptions, including the PostgreSQL-only GCRA tables. Shared identity schema
does not imply that every authentication adapter already exists on SQLite.
