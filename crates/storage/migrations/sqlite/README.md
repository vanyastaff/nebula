# SQLite Migrations

Spec-16 compliant schema for Nebula's SQLite backend (local-first / dev / tests).

## Dialect notes

- IDs: `BLOB` (16-byte ULID, prefixed on wire)
- JSON: `TEXT` (validated by application; sqlite `json1` extension for querying)
- Timestamps: `TEXT` (ISO 8601 format)
- IP addresses: `TEXT`
- Arrays: `TEXT` (JSON array)
- Booleans: `INTEGER` (0/1)
- CAS: `INTEGER` version column on all mutable entities
- No `ALTER TABLE ADD CONSTRAINT` for foreign keys (enforced at app level where needed)
- No partial indexes with `NOW()` (SQLite requires constant expressions)

## Migration order

Same structure as `../postgres/` — see that README for the table index.
Migration `0020_add_resume_result_persistence.sql` lands in both dialects
(ADR-0009 resume persistence schema).

Migration `0021_add_control_queue_reclaim_count.sql` lands in both dialects
in parity with ADR-0017 (control-queue reclaim policy, ADR-0008 B1 follow-up).

Migration `0026_execution_control_queue_w3c_trace_context.sql` adds nullable
`w3c_trace_context` to `execution_control_queue` in both dialects (M3.5).

Migration `0040_credential_refresh_retry_gate.sql` adds the structural
credential refresh-retry admission gate in parity with PostgreSQL.

## Storage-port adapter schema (0027)

`crates/storage/src/sqlite/schema.sql` is the **cumulative** `port_*` schema,
applied in one shot by `nebula_storage::sqlite::init_schema` for `:memory:`
and test pools. The spec-16 port (execution + the atomic `TransitionBatch`,
control-queue outbox, idempotency, webhook activations, workflows/versions,
and the identity stores) persists through those `port_*` tables.

`0027_port_adapter_schema.sql` was that schema *at the time it was written*.
It is now a historical migration and **must not be regenerated from
`schema.sql`** — the two have legitimately diverged because later port
changes landed as their own migrations (0032 webhook activation fields,
0033 spec link, 0034 control-queue resume target, 0035 resume tokens).
Copying the cumulative schema over 0027 would rewrite an already-applied
migration and duplicate what those later ones already do.

When the port schema changes, do both:

1. add a **new** numbered migration for the delta, and
2. edit `schema.sql` so the one-shot install stays current.

The invariant to hold is *end-state equality*, not file equality: replaying
`0001..NNNN` into a file-backed database must produce the same tables,
columns, types and constraints as `init_schema` builds in one shot.

Verify across a migrated database and a freshly `init_schema`-built one:

1. the `port_%` object sets in `sqlite_master` match (tables **and** indexes
   and triggers — a migration that forgets an index still passes a
   column-only comparison);
2. `pragma table_info(<table>)` matches per table (names, declared types,
   `NOT NULL`, defaults, primary-key position);
3. `pragma index_list` / `index_info` and `pragma foreign_key_list` match per
   table, which is what covers uniqueness and referential constraints.

`CHECK` constraints live only in the stored `CREATE TABLE` text, so compare
the `sqlite_master.sql` bodies too — but normalize whitespace and strip `--`
comments first: `ALTER TABLE` rewrites that text, so a migrated table and an
`init_schema` one legitimately differ in comments and column order while
being semantically identical.

## Rebuilding the local dev database

A file-backed SQLite rebuild that applies these migrations destroys all
local dev data. `:memory:` test pools install the same schema fresh per
run via `init_schema`, so tests need no migration step.

## Schema parity

Where both dialects define a table, they must define it *logically
identically*: types differ by dialect, but table/column names and
constraints must match.

Parity is **not** total, and the gaps are deliberate — these tables are
PostgreSQL-only because their adapters are (`crates/storage/src/pg/`, with
no SQLite counterpart):

| PostgreSQL migration | Why it has no SQLite counterpart |
|----------------------|----------------------------------|
| `0029_external_identities` | `external_identities` — OAuth identity linking, `pg/external_identity.rs` only |
| `0036_plane_a_oauth_state_cleanup_index` | partial index over `NOW()`; SQLite requires constant expressions (see *Dialect notes*) |
| `0037_mfa_enrollment_candidates` | `mfa_enrollment_candidates` — `pg/mfa_enrollment.rs` only |
| `0038_identity_secret_authority` | `pg/identity_secret.rs` only |

A fresh SQLite database therefore has two fewer tables than a fresh
PostgreSQL one (`external_identities`, `mfa_enrollment_candidates`). Adding
a SQLite adapter for any of those features means adding its migration here
at the same time.
