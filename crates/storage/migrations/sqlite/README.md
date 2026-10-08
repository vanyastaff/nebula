# SQLite migrations

This catalog creates the SQLite deployment schema. Production startup, file-backed
reconnects, and in-memory test databases use these same numbered SQL files. There is
no separate schema snapshot.

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
| `0008_platform.sql` | HTTP response replay cache; shared PostgreSQL GCRA tables have no SQLite counterpart |
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
ownership, and deletion rules. SQLite stores instants as integer microseconds since
the Unix epoch, documents as `TEXT` with JSON checks, flags as integers restricted to
zero or one, and opaque binary identities as `BLOB`. Adapters convert these values at
the storage boundary. Foreign keys must be enabled on connections and enforce tenant
ownership and permanent-purge cascades. An existence foreign key cannot prove a
parent is live; writers check soft-deletable parents inside their transaction.

Rate limits are process-local on SQLite. PostgreSQL defines shared `rate_limits` and
`rate_limit_reservations` inside its platform migration. Both catalogs still carry
every numbered version; backend-specific tables do not require reserved versions.

## Applying and extending the catalog

`nebula_storage::sqlite::init_schema` admits and applies this catalog using SQLite's
setup guard. The catalog ledger must be a canonical prefix; unknown versions, edited
checksums, failed migrations, and foreign unledgered schemas fail closed. File-backed
credential setup probes admission before opening a foreign database for writing.

After this baseline, append immutable `NNNN_description.sql` files to both backend
catalogs with the same version and slug. Review admission safety and record the review
before raising `migration_catalog::REVIEWED_HEAD`. Update both README inventories.
Never edit an applied migration to change the schema. Include dialect-specific DDL
inside the matching migration rather than leaving version gaps.

## Disposable development databases

The pre-baseline development catalog is incompatible and is rejected; there is no
automatic ledger adoption or migration-floor exception. Recreate disposable SQLite
databases with the current catalog. Stop all processes using a file database before
removing it and its sidecars; recreation destroys its data. In-memory test pools create
the schema fresh. `task db:reset` is the local PostgreSQL reset command, not a SQLite
file-reset command. Valuable data requires a separately designed migration plan.

## Backend parity

`schema_parity_postgres` compares the paired schemas, including foreign-key actions,
with explicit dialect exceptions. SQLite's JSON and boolean type checks have no
PostgreSQL counterparts because PostgreSQL column types enforce them. Identity tables
exist on both backends; schema parity does not claim that all durable authentication
adapters are implemented on SQLite.
