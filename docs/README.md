# Nebula documentation map (agents)

**Stop.** Read this table before opening other paths under `docs/`.

## Tier 0 — read first

| Path | Role |
|------|------|
| [`CLAUDE.md`](../CLAUDE.md) | Repo rules, layout, commands, guard hooks |
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
| [`docs/VISION.md`](./VISION.md) | Long-horizon vision |
| [`docs/OBSERVABILITY.md`](./OBSERVABILITY.md) | Metrics / tracing boundaries |
| [`docs/adr/README.md`](./adr/README.md) | **Active ADRs (0042+)** — implement from these |
| [`docs/adr/HISTORICAL.md`](./adr/HISTORICAL.md) | Index only for 0001–0041 — open one file if cited |

**Conflict rule:** `PRODUCT_CANON.md` (invariants) → `INTEGRATION_MODEL.md` (mechanics) →
**accepted ADR** for a specific decision → `STRATEGY.md` (direction) → crate README.
Plans and `docs/ARCHIVE.md` trees are never normative.

**Compound Engineering / Cursor:** There is **no** separate “ADR skill”. `ce-plan` and
`ce-work` use **your** doc map; they do not auto-load all of `docs/adr/`. When an ADR
amends canon, update canon or INTEGRATION_MODEL with a **pointer**, not a second spec.

## Removed from repo (do not search here)

Plans, audits, conference notes, and superpowers drafts were **moved out** of
this repository. See [`docs/ARCHIVE.md`](./ARCHIVE.md). They are **not**
implementation specs.

## ADR layout

- **0042+** — current cascade (`docs/adr/0042-*.md` … `0065-*.md`).
- **0001–0041** — on disk for deep links from code; **excluded from Cursor
  index** (`.cursorignore`). Use [`docs/adr/HISTORICAL.md`](./adr/HISTORICAL.md)
  to pick the right file.

## Integration author path

1. `STRATEGY.md` → SDK / plugin layers  
2. `crates/action`, `credential`, `resource`, `plugin` READMEs  
3. `Schema` = configuration form for Action / Credential / Resource  
4. `PRODUCT_CANON.md` for durability and operational honesty  

## Legacy paths (wrong)

| Do not use | Use |
|------------|-----|
| Former `superpowers/` tree (removed) | [`ARCHIVE.md`](./ARCHIVE.md) |
| `docs/audits/**` | Removed — archive |
| `C:/Users/.../RustroverProjects/docs/` as workspace canon | This repo’s `docs/` table |
| Bulk `glob docs/**` | This file + one crate README + cited ADR |
