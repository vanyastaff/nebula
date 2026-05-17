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

## Storage-port adapter schema (0027)

`0027_port_adapter_schema.sql` is **byte-identical** to
`crates/storage/src/sqlite/schema.sql`, which the spec-16 SQLite adapters
apply via `nebula_storage::sqlite::init_schema` for `:memory:` and test
pools. It is the canonical source for a rebuilt file-backed SQLite
database — the spec-16 port (execution + the atomic `TransitionBatch`,
control-queue outbox, idempotency, webhook activations,
workflows/versions, and the identity stores) persists through these
`port_*` tables. Keep this file and `schema.sql` in lockstep —
regenerate with `cp crates/storage/src/sqlite/schema.sql \
crates/storage/migrations/sqlite/0027_port_adapter_schema.sql` whenever
the port schema changes.

## Rebuilding the local dev database

A file-backed SQLite rebuild that applies these migrations destroys all
local dev data. `:memory:` test pools install the same schema fresh per
run via `init_schema`, so tests need no migration step.

## Schema parity

This directory and `../postgres/` must define logically identical tables.
Types differ by dialect; table/column names and constraints must match.
