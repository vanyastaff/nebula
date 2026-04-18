<!-- This template is normative for crates/*/README.md and, in reduced form, crates/*/src/lib.rs //!. See docs/PRODUCT_CANON.md §15 and docs/superpowers/specs/2026-04-17-docs-architecture-redesign-design.md §6. -->

---
name: nebula-<crate>
role: <named pattern from docs/GLOSSARY.md — e.g. "Transactional Outbox", "Idempotent Receiver", "Bulkhead Pool", "Stability Pipeline">
status: frontier | stable | partial
last-reviewed: YYYY-MM-DD
canon-invariants: [L2-11.1, L2-12.3, ...]   # optional; empty list if none
related: [nebula-core, nebula-error, ...]    # sibling crates and satellite docs
---

# nebula-<crate>

## Purpose

One paragraph. What this crate is for, framed as a problem it solves in the engine.

## Role

Named architectural pattern (see `docs/GLOSSARY.md` Architectural Patterns section) with a one-line book reference if applicable.

Example: *Transactional Outbox (DDIA ch 11; EIP "Guaranteed Delivery"). Persists control-plane signals atomically with state transitions.*

## Public API

Key types / traits / functions. Use rustdoc-style links where useful. Do NOT duplicate rustdoc in prose — keep this section a catalog, one line per item.

Example:

- [`ExecutionRepo`] — repository trait, seam for §11.1 CAS transitions.
- [`ExecutionControlQueue`] — durable outbox for cancel/dispatch signals (§12.2).
- [`ExecutionJournal`] — append-only event log.

## Contract

Invariants this crate enforces. Each invariant cites the canon layer (L1/L2/L3) and points to its seam test. Do NOT duplicate invariants' full text — reference canon section.

Example:

- **[L2-§11.1]** State transitions use CAS on `version`. Seam: `ExecutionRepo::transition`. Test: `crates/execution/tests/authority.rs::transition_cas`.
- **[L2-§12.2]** Outbox writes share the same transaction as state transitions. Seam: `ExecutionRepo::transition_with_signal`. Test: `crates/execution/tests/outbox_atomicity.rs`.

## Non-goals

What this crate deliberately does NOT do. Point to the crate that does if there is one.

Example:
- Not an expression evaluator — see `nebula-expression`.
- Not a retry pipeline — see `nebula-resilience`.

## Maturity

See `docs/MATURITY.md` row for this crate. Short summary here:

- API stability: stable | frontier | partial
- One sentence on what is still moving, if anything.

## Related

- Canon: `docs/PRODUCT_CANON.md` sections touched.
- Satellite docs: `docs/INTEGRATION_MODEL.md`, `docs/OBSERVABILITY.md`, …
- Siblings: list crates this one depends on or is depended on by.
