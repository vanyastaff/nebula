---
id: 0007
title: prefixed-ulid-identifiers
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [id, core, ulid, domain-key]
related: [crates/core/src/id/, crates/core/src/keys.rs, docs/GLOSSARY.md]
---

# 0007. Prefixed ULID identifiers (Stripe-style)

## Context

Every entity in Nebula needs a stable identifier. The choice affects storage
size, index performance, log / trace readability, URL shareability, compile-time
type safety, wire-format stability, and sort order for pagination. Getting it
wrong means either (a) UUIDs everywhere so `grep` across logs becomes useless,
(b) auto-increment integers that leak cardinality and enable enumeration, or
(c) custom schemes nobody understands.

Before this decision, `nebula-core` used a mixture of UUID v4 newtypes with no
prefix convention. Related outcomes downstream: `grep cred_...` against audit
logs was impossible, a DELETE request with a wrong-kind id returned a silent
`404` instead of a typed rejection, and the IDs were not time-sortable so
pagination by creation order required an extra `created_at` column.

The decision catalog below first appeared in the 2026-04-15 architecture review
(spec 06 in that session's arch-specs bundle). This ADR extracts the locked
parts from that spec; the full rationale and per-entity catalog live in
the archived spec file.

## Decision

**Prefixed ULID, Stripe-style**, as the single identifier convention for every
system-generated entity:

- 16 bytes binary in storage (`UUID` column in Postgres, `BLOB` in SQLite).
- Base32-encoded string on the wire with a typed prefix: `{prefix}_{ulid}`,
  e.g. `wf_01J9XYZABCDEF0123456789XYZA`.
- Typed newtype per entity kind, generated via a macro so prefix-mismatch
  becomes a compile-time (and parse-time) error.
- Monotonic ULID generator for hot append paths (journal, trigger inbox) to
  guarantee deterministic sort within a millisecond.

The catalog of prefixes (non-exhaustive, additions through explicit revision):
`org_`, `ws_`, `user_`, `sa_`, `sess_`, `pat_`, `wf_`, `wfv_`, `exec_`,
`cred_`, `res_`, `action_`, `plugin_`, `job_`, `nbl_`, `trig_`, `evt_`.

Separately, author-defined string keys (unstable, user-visible handles like
`NodeKey` in a workflow definition) use the **`*Key`** naming convention and
are not ULIDs — they are validated string newtypes backed by `domain_key::Key`.
System-generated IDs use the `*Id` convention and are ULID-backed. See also
§12.15 of the canon on the naming convention.

## Consequences

Positive:

- **Type safety on the wire.** Wrong-kind id reaches the handler as a typed
  parse error, not a silent `404`.
- **Debuggability.** `grep "cred_01J9X" logs/` finds every touch of one
  credential across the system; no collision with workflow ids.
- **Sortability.** ULID timestamp prefix means `ORDER BY id` ≈
  `ORDER BY created_at` — one less column needed for cursor pagination.
- **Storage compactness.** 16 bytes binary, same size as UUID column.
- **Support ergonomics.** A user pasting `cred_01J9X...` immediately tells
  the operator which entity failed — no schema archaeology to identify the
  kind.

Negative / accepted costs:

- Breaking change for any consumer of the previous UUID v4 newtypes
  (in-repo migration was absorbed during the ID-migration work).
- Prefix catalog must be kept in sync with new entity kinds; drift produces
  parse errors at runtime if a prefix is typo'd.

Follow-up:

- `ProjectId` → `WorkspaceId` rename is pending (still uses old naming in
  `crates/core/src/lib.rs`).
- Monotonic ULID generator is in place on hot-append seams as they land.

## Alternatives considered

- **UUID v4 (random).** Rejected: not sortable (pagination by id breaks),
  causes index fragmentation on insert, `grep` across logs is useless because
  every entity kind shares the same shape.
- **UUID v7 (timestamp).** Rejected at the time of decision: Rust crate
  ecosystem for v7 was less mature than `ulid`; no functional advantage
  over ULID once the prefix convention is adopted.
- **KSUID.** Rejected: 20 bytes does not fit the `UUID` column type in
  Postgres, forcing `BYTEA` everywhere and losing the canonical type.
- **NanoID.** Rejected: not sortable — kills pagination by id.
- **Auto-increment integer.** Rejected: cardinality leaks through the id
  itself (`user_7` tells you there are ≥7 users), and IDs collide across
  instances in multi-tenant deployments.

## Seam / verification

Seams:

- `crates/core/src/id/` — `PrefixedId` trait, `prefixed_id!` macro,
  per-entity newtypes.
- `crates/core/src/keys.rs` — `*Key` string newtypes (separate from
  ULID-backed `*Id`).
- `crates/core/src/lib.rs` re-exports the catalog.
- `docs/GLOSSARY.md` — canonical ID reference table.

Tests: `crates/core/benches/id_parse_serialize.rs` exercises the parse /
serialize round-trip; unit tests in `crates/core/src/id/` cover prefix
mismatch and malformed-body errors.

Related ADRs: 0001 (schema consolidation) depends on the `ExecutionId` /
`WorkflowId` shape defined here for per-execution proof-token lineage.
