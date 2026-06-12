# Quality gates

How Nebula mechanically enforces code quality in the toolchain. Normative
product rules live in [`docs/PRODUCT_CANON.md`](./PRODUCT_CANON.md); the
agent-facing discipline contract is the **Enforced Discipline** table in
[`AGENTS.md`](../AGENTS.md). This file is only about *how* the knobs work and
*why* some Clippy lints are intentionally `allow`. The knobs themselves live in
`Cargo.toml` (`[workspace.lints]`), `clippy.toml`, `deny.toml`, and
`.claude/hooks/` — extend them in place; do not duplicate them elsewhere.

Layer order (strongest first): rustc / Clippy (`-D warnings` in CI) →
`cargo deny` (`deny.toml`) → committed guard hooks (`.claude/hooks/`, the D10
no-cheat core + the ADR-0083 Layer-2 budget) → human review.

## Mechanized junior markers

What is enforced today, observable in the repo (the `Cargo.toml`
`[workspace.lints.clippy]` block carries the citations referenced here):

| Marker | Mechanization (current) |
|--------|-------------------------|
| Pedantic / nursery / cargo lint families | `[workspace.lints.clippy]` at **warn**; CI `cargo clippy … -- -D warnings` turns every warn into a hard failure (`.github/workflows/ci.yml`). |
| `std::mem::forget` misuse | `mem_forget = "deny"` (`Cargo.toml`). |
| `Rc<Mutex>` / non-`Send`/`Sync` `Arc` footguns | `rc_mutex` / `arc_with_non_send_sync` = **warn** — cites [C-SEND-SYNC](https://rust-lang.github.io/api-guidelines/interoperability.html#c-send-sync). |
| `dbg!` shipped in non-test code | `dbg_macro = "warn"` (tests exempt via `clippy.toml`). |
| `unwrap()` / `expect()` / `panic!()` in library code | **Enforced**, no escape, by `.claude/hooks/edit-guard.sh` (AGENTS.md "Enforced Discipline" / D10) — not via `clippy::unwrap_used`, which would need a workspace-wide burn-down first. |
| `unsafe` without local reasoning | `undocumented_unsafe_blocks` + `clippy.toml` (`accept-comment-above-statement/attributes`); convention `// SAFETY:` above the block. |
| Function bloat / cognitive complexity / nesting | `clippy.toml` thresholds (`too-many-lines = 100`, `cognitive-complexity = 25`, `excessive-nesting = 5`) are **inert workspace-wide** (the lints are `allow` — see next section) but enforced **diff-scoped on new code** by `.claude/hooks/intent-gate.sh` (ADR-0083). |
| Duplicate utility / oversized / file-sprawling turns | `.claude/hooks/intent-gate.sh` net-LoC / new-file / large-blob / duplicate-public-symbol budgets (ADR-0083), with a `// budget-justified:` escape. |

Still review-only (honest list — no full mechanization yet): `Box<dyn Error>`
at public API boundaries; duplicate *stable type* names across crates (the
duplicate-*symbol* heuristic in `intent-gate.sh` is a partial, conservative
proxy, not a full check); ADR front-matter ↔ code traceability.

## Intentionally allowed Clippy

Several lints in `Cargo.toml` `[workspace.lints.clippy]` are set to `allow`
**not** because Nebula rejects what they encourage, but because `warn` would
force large or noisy churn across existing code (style taste, macro sites, API
shape, a generic-heavy workspace, or legacy patterns). This is a universal
policy, not a per-feature exception.

**Rule for agents and reviewers:** on **new** and **heavily-touched** code,
follow the *spirit* of those lints where it improves clarity, safety, or
alignment with the Rust API Guidelines and Reference — even when CI is green.
CI passing does not mean "ignore the lint's intent" on new code.

That spirit is no longer review-only: it is mechanized **diff-scoped** by the
ADR-0083 Layer-2 gate (`.claude/hooks/intent-gate.sh`). The inert `clippy.toml`
complexity thresholds, plus net-LoC / new-file / duplicate-symbol budgets, are
enforced on the turn's changed code while legacy stays grandfathered.

**Mechanization path for any such `allow`:** workspace `warn` only after an
explicit burn-down, or `warn` in crates that opt in, or a targeted
`dylint`/lint crate on changed paths. The sequenced legacy structural-debt
burn-down workstream (ADR-0083 § Follow-up) reconciles the
`cognitive_complexity` / `too_many_lines` allowance crate-by-crate; until then
`intent-gate.sh` holds the line on new code.

## Diff-scoped structural budget (ADR-0083)

The `cognitive_complexity` / `too_many_lines` workspace `allow` stays — flipping
them on 36 crates is thousands of legacy warnings. `.claude/hooks/intent-gate.sh`
holds new code to a diff-scoped budget instead: the **large-blob proxy** is
derived from the `clippy.toml` `too-many-lines = 100` threshold; the **net-LoC
(400)**, **new-file (5)** and **duplicate-symbol** caps are the gate's own
independent budgets (not `clippy.toml` thresholds). All carry a
`// budget-justified:` escape. Legacy is grandfathered; the separate legacy
burn-down workstream reconciles the inert clippy thresholds crate-by-crate.
