# nebula database standard

The schema contract every migration, adapter and conformance test follows on both SQL
backends (PostgreSQL, SQLite). It is a standard, not a description of history: a new
table or column either follows it or changes it here first.

## Layout

One migration per aggregate in dependency order — `identity`, `tenancy`, `workflows`,
`executions`, `dispatch`, `credentials`, `resources`, `runtime_control`, `platform` — on
both backends; later migrations append. Table names are snake_case plural nouns without
technical prefixes.

## Names

Every constraint and index is named explicitly, identically on both backends:

| Object | Name |
|---|---|
| primary key | `pk_<table>` |
| unique | `uq_<table>__<cols>` |
| foreign key | `fk_<table>__<referenced table>` |
| check | `ck_<table>__<rule>` |
| index | `ix_<table>__<cols>` |

`<cols>` lists the key columns in order, joined by `_`. A partial index appends its
predicate as one word: `__live` (`deleted_at IS NULL`), `__unrevoked`, `__unconsumed`,
`__published`, … — `uq_workspaces__org_id_slug__live`, `uq_workspaces__org_id__live_default`.

A name is at most 63 bytes: PostgreSQL silently truncates longer identifiers, which
would drop the predicate suffix and split the two backends' catalogs. A name that
does not fit is shortened by these steps, in order, stopping as soon as it fits:

1. drop the tenant columns `org_id_workspace_id` from `<cols>`;
2. drop the `_id` suffix of every column in `<cols>`/`<rule>`;
3. write a length rule's `_length` as `_len`;
4. keep only the first column of `<cols>`.

The prefix, the table and the predicate suffix are never shortened —
`ix_execution_revision_references__worker_flavor__rollback`. The
`schema_object_names` test rejects a name over 63 bytes.

Type checks that PostgreSQL gets from its column types (`ck_<table>__<col>_json`,
boolean `ck_<table>__<col>` on flags) exist on SQLite only; every other name exists on both.
PostgreSQL reports violated constraints by name; SQLite reports UNIQUE violations by column
list, so constraint names are diagnostics on PostgreSQL.

## Types

| Meaning | PostgreSQL | SQLite |
|---|---|---|
| instant | `TIMESTAMPTZ` | `INTEGER` µs since the Unix epoch |
| duration | `BIGINT` `<name>_ms` | `INTEGER` `<name>_ms` |
| JSON document | `JSONB` | `TEXT` + `CHECK (json_valid(col))` |
| flag | `BOOLEAN` | `INTEGER` + `CHECK (col IN (0, 1))` |
| closed enum | `TEXT` + named `CHECK (col IN (...))` | same |
| bytes (digest, ciphertext, opaque key) | `BYTEA` + length `CHECK` | `BLOB` + length `CHECK` |
| entity id | `TEXT` (the port's canonical string form) | `TEXT` |

Structured values the database must reason about (identities, digests) are columns, not
keys inside a JSON document. Binary entity ids wait for typed ids in the port; existing
`BYTEA` ids stay until then.

### Instants

Instants are microsecond-precise on every backend; the in-memory backend truncates to
microseconds when it authors or stores one, so a row reads back identically everywhere.
Instants the backend authors (soft delete, grant time, provisioning) come from one clock per
backend: PostgreSQL's transaction clock (`now()`), the process clock on SQLite and in
memory. Caller-supplied instants are stored as given (truncated).

## Invariants

- Every table declares its primary key explicitly; every key column is `NOT NULL`.
- Tenant-owned rows carry `org_id, workspace_id` (hierarchy order). Keys and foreign keys
  include them, except where an id is unique across tenants (`workspaces.id`): its key is
  the id, and a `UNIQUE (org_id, id)` is the target children reference so every child row
  names its tenant.
- Optimistic concurrency: `version BIGINT NOT NULL CHECK (version >= 0)`.
- Soft delete: nullable `deleted_at` instant. A soft-deleted row is invisible to every read
  and write of its store, including the rows of its own aggregate beneath it (a deleted
  workflow's versions).
- Relationships the database can check are foreign keys. A foreign key proves existence,
  not liveness: an adapter that creates a row beneath a soft-deletable parent checks the
  parent is live in the same transaction (PostgreSQL: `FOR SHARE` on the parent row, so a
  concurrent soft delete serializes with it) and answers `NotFound` otherwise.
- Deletion is two-step. Soft delete is the archive: reversible, invisible to reads, no
  new children. Purge is the permanent step: it hard-deletes the aggregate and its
  contents through `ON DELETE CASCADE` foreign keys, so every foreign key from a child to
  its owning parent cascades. Purge, restore and workspace transfer are tracked in
  issue 1159.
- Ordered reads order by bytes (`COLLATE "C"` on PostgreSQL), so every backend returns the
  same order.

## Backends

- The SQL backends share one database per deployment and enforce everything above.
- The in-memory backend enforces every invariant **inside one aggregate** exactly as the
  schema does: keys, uniqueness among live rows, CAS, soft-delete visibility, and
  intra-aggregate references (a version needs its live workflow; a workspace grant needs the
  org grant and is removed with it). References **between aggregates** (a workflow's
  workspace) are foreign keys the in-memory stores do not check: they are composed
  independently, and the API resolves the live workspace through the tenant directory before
  any workspace-owned write — the foreign key is the backstop behind that resolution.
- Conformance encodes the split: `matrix!` cases run on all backends; cross-aggregate
  invariants are `relational_matrix!` cases (SQLite + PostgreSQL). An in-memory store that
  gains such a check moves its case into `matrix!`.

## Guards

- `schema_parity_postgres` compares both backends structurally; only listed
  PostgreSQL-only tables, the SQLite-only type checks above and dialect-specific indexes
  (each with a reason) may differ.
- `migration_catalog::REVIEWED_HEAD` and its review log gate every new migration.
