# CLAUDE.md

**Derived from:** `docs/AGENT_PROTOCOL.md` (meta-protocol); `docs/PRODUCT_CANON.md` §15 (document map); `rust-toolchain.toml` (toolchain pin); `.github/workflows/ci.yml` and `.github/workflows/test-matrix.yml` (CI commands); `.cursor/rules/*.mdc` (modular agent rules).

This file is the **entry point** for Claude Code. Domain rules live under **`.cursor/rules/`** (MDC). The canonical agent text is **`docs/AGENT_PROTOCOL.md`** — read **Universal principles** first, then the verbatim rules. House style and Rust mindset are **`docs/STYLE.md`** (§0 universal, §§1–2 idioms/antipatterns). For **language-level** rule IDs and a Rust 1.95+ LLM contract (UB, `unsafe`, patterns), load **`docs/RUST_EXPERT_STYLE_GUIDE.md`** / **`docs/guidelines/README.md`** — Nebula docs still win on conflicts.

## Non-negotiable

1. Follow **`docs/AGENT_PROTOCOL.md`** (evidence before assertion, re-exports, macros, migration state, doc↔code gaps, inspect/implement, erosion triggers, **`docs/IDIOM_REVIEW_CHECKLIST.md`** when required).
2. Follow **`docs/PRODUCT_CANON.md`** for product invariants; use **`docs/INTEGRATION_MODEL.md`** for integration mechanics.
3. Use **`.cursor/rules/`** — rules auto-attach by glob; `00-meta-protocol.mdc` and `11-product-canon-core.mdc` are always on.

## Session read order (from `docs/PRODUCT_CANON.md` §15)

Normative core and satellites are listed in the **§15** table — load `PRODUCT_CANON`, `INTEGRATION_MODEL`, `STYLE`, `GLOSSARY`, `MATURITY`, `OBSERVABILITY`, `ENGINE_GUARANTEES`, `UPGRADE_COMPAT`, `pitfalls`, and `adr/README.md` as the task requires.

## Toolchain and verification (**observable**)

- **Pinned toolchain:** `channel = "1.95.0"` — `rust-toolchain.toml` lines 16–18.
- **Formatting (CI):** `cargo +nightly fmt --all -- --check` — `.github/workflows/ci.yml` lines 60–66 (`fmt` job uses nightly rustfmt per comment lines 56–59).
- **Clippy (CI):** `cargo clippy --workspace -- -D warnings` — `.github/workflows/ci.yml` lines 87–88.
- **Tests (matrix):** `cargo nextest run -p … --profile ci --no-tests=pass` — `.github/workflows/test-matrix.yml` lines 160–164.

For a local gate similar to historical dev practice, run fmt (nightly) + clippy + nextest; align changed crates with `lefthook.yml` / CI when touching those paths.

## Where rules came from

| Rule file | Primary `/docs` source |
|-----------|-------------------------|
| `00-meta-protocol.mdc` | `docs/AGENT_PROTOCOL.md` |
| `10-workspace-layout.mdc` | `docs/PRODUCT_CANON.md` §5 + `deny.toml` + `docs/AGENT_PROTOCOL.md` (layers, SRP/SOLID vs canon) |
| `11-product-canon-core.mdc` | `docs/PRODUCT_CANON.md` §4.5, §12.2, §12.5, §17 |
| `20-glossary-maturity.mdc` | `docs/GLOSSARY.md`, `docs/MATURITY.md`, `docs/STYLE.md` §3 |
| `30-style.mdc` | `docs/STYLE.md` (§0 universal mindset, §§1–2 idioms/antipatterns) |
| `40-engine-guarantees.mdc` | `docs/ENGINE_GUARANTEES.md` |
| `50-integration.mdc` | `docs/INTEGRATION_MODEL.md` |
| `60-pitfalls.mdc` | `docs/pitfalls.md` |
| `65-observability.mdc` | `docs/OBSERVABILITY.md` |
| `66-upgrade-compat.mdc` | `docs/UPGRADE_COMPAT.md` |

## Generic agents

See **`AGENTS.md`** for the same pointers without Claude-specific framing.
