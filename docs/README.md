# Nebula documentation map (agents)

**Stop.** Read this table before opening other paths under `docs/`.

## Tier 0 — read first

| Path | Role |
|------|------|
| [`AGENTS.md`](../AGENTS.md) | Repo rules, layout, commands, guard hooks |
| [`STRATEGY.md`](../STRATEGY.md) | Product direction and 2026 standard bar |
| [`README.md`](../README.md) | Product overview |
| [`docs/pitfalls.md`](./pitfalls.md) | Traps before touching hot paths |
| [`docs/MATURITY.md`](./MATURITY.md) | L0–L4 maturity |
| `crates/<crate>/README.md` | Crate you are editing |

## Tier 1 — normative

| Path | Role |
|------|------|
| [`docs/PRODUCT_CANON.md`](./PRODUCT_CANON.md) | Binding invariants |
| [`docs/INTEGRATION_MODEL.md`](./INTEGRATION_MODEL.md) | Integration mechanics (Resource, Credential, Action, Schema, Plugin) |
| [`docs/VISION.md`](./VISION.md) | Human charter draft — **agents skip** (use STRATEGY + canon) |
| [`docs/OBSERVABILITY.md`](./OBSERVABILITY.md) | Metrics / tracing boundaries |
| Design records (ADRs, roadmap, specs, research) | Maintained in the maintainers' private design vault — **not tracked in this public repository** |

**Conflict rule:** `PRODUCT_CANON.md` (invariants) → `INTEGRATION_MODEL.md` (mechanics) →
**accepted ADR** for a specific decision → `STRATEGY.md` (direction) → crate README.

**Compound Engineering / Cursor:** There is **no** separate “ADR skill”. `ce-plan` and
`ce-work` use **your** doc map. When an ADR amends canon, update canon or
INTEGRATION_MODEL with a **pointer**, not a second spec.

## Design history (not in this repo)

Design records (ADRs, roadmap, specs, research, audits, conference notes) are
maintained in the maintainers' private design vault and are **not** tracked in
this public repository. The canon docs above (PRODUCT_CANON, INTEGRATION_MODEL,
MATURITY, OBSERVABILITY) are the in-repo source of truth; ADRs are referenced by
**id** in prose for traceability, but their full text is not in-tree.

## Integration author path

1. `STRATEGY.md` → SDK / plugin layers
2. `crates/action`, `credential`, `resource`, `plugin` READMEs
3. `Schema` = configuration form for Action / Credential / Resource
4. `PRODUCT_CANON.md` for durability and operational honesty

## Legacy paths (wrong)

| Do not use | Use |
|------------|-----|
| Former `superpowers/` tree (removed) | Maintainers' private design vault — not in this repo |
| `docs/audits/**`, `docs/adr/**`, `docs/plans/**` (all removed) | Maintainers' private design vault — not in this repo |
| `C:/Users/.../RustroverProjects/docs/` as workspace canon | This repo’s `docs/` table |
| Bulk `glob docs/**` | This file + one crate README + cited ADR id |

## Code comments (Rust)

Write **behavior-first** `//!` / `//` text in `crates/**/*.rs`. Normative decisions live in
canon, INTEGRATION_MODEL, and ADRs — not as section pins on every hot path.

| Do | Don't |
|----|-------|
| Explain *what the code enforces* (lease reclaim, SKIP LOCKED, tenant scope, replay-oracle) | `ADR-0048 §3.2`, `canon §12.2`, `spec-16 §11.3` in inline comments |
| At most **one** module-level pointer per file to `crates/<crate>/README.md` or `docs/INTEGRATION_MODEL.md` when traceability helps | Scatter ADR numbers through `//` blocks inside functions |
| Keep ADR ids in **tests** when the test name/doc is explicitly proving a contract seam | Remove history from `deny.toml` layer comments or supersession tables |

`rg 'ADR-0|canon §' crates --glob '*.rs'` should trend to zero on merged work; crate READMEs may still cite ADRs for integrators.
