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

## Schema parity

This directory and `../postgres/` must define logically identical tables.
Types differ by dialect; table/column names and constraints must match.
