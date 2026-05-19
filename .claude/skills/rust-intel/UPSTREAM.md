# rust-intel — vendoring provenance

Third-party agent skill vendored into this repo. **Not authored here.**

| Field | Value |
|-------|-------|
| Upstream | https://github.com/PHPCraftdream/rust-intel |
| Version | v0.2.2 |
| Commit | `3626f0c75d3ffe543dd0dcb4c1e96f7325fa181c` |
| License | MIT (see `LICENSE` in this directory) |
| Vendored on | 2026-05-19 |

## What this is

A behavioral spec defending against 26 categories of LLM-systematic Rust
mistakes (Tier A compile failures, Tier B silent-correctness bugs, Tier C
architecture). `SKILL.md` auto-loads on any Rust task; the three
`/rust-cc-{audit,fix,plan}` slash commands drive it.

## Path mapping (upstream → here)

| Upstream | Vendored path |
|----------|---------------|
| `rust-intel.md` | `.claude/skills/rust-intel/SKILL.md` |
| `docs/sources.md` | `.claude/skills/rust-intel/docs/sources.md` |
| `docs/roadmap.md` | `.claude/skills/rust-intel/docs/roadmap.md` |
| `LICENSE` | `.claude/skills/rust-intel/LICENSE` |
| `commands/rust-intel-cc/audit.md` | `.claude/commands/rust-cc-audit.md` |
| `commands/rust-intel-cc/fix.md` | `.claude/commands/rust-cc-fix.md` |
| `commands/rust-intel-cc/plan.md` | `.claude/commands/rust-cc-plan.md` |

`docs/` lives beside `SKILL.md` so its in-skill relative links
(`[docs/sources.md](docs/sources.md)`) resolve. Installer scripts
(`rust-cc-install.{sh,ps1,bat}`) are **deliberately not vendored** — no
executable third-party code enters the tree.

## Vendoring delta (Nebula adaptation)

Not byte-exact. Vendoring renamed files (`rust-intel.md` → `SKILL.md`;
`commands/rust-intel-cc/*` → flat `rust-cc-*`), which broke the
upstream's internal cross-references. A **small, mechanical, documented
delta** is applied so the skill actually functions in-tree — a
compatibility-shim file was rejected; the references are corrected at
source:

| Vendored file | Delta vs upstream |
|---|---|
| `SKILL.md` | `+ name: rust-intel` frontmatter line (Nebula skill convention) |
| `commands/rust-cc-{audit,fix,plan}.md` | literal `` `rust-intel.md` `` rule-source refs → `` the `rust-intel` skill (`SKILL.md`) `` |
| `commands/rust-cc-audit.md` | empty-arg default scope `src/**/*.rs` → Nebula workspace (`crates/*/src`, `examples/src`, `apps/*/src`) — no root `src/` here |
| `docs/roadmap.md` | command names `/rust-{audit,fix,plan}` → `/rust-cc-*`; links `../commands/rust-*.md` → `../../../commands/rust-cc-*.md` (correct depth + prefix) |
| `docs/sources.md` | prose `` `rust-intel.md` `` → `` the `rust-intel` skill (`SKILL.md`) `` |

Conceptual mentions of the upstream filename inside upstream prose (e.g.
`roadmap.md` "should `rust-intel.md` be split") are left as-is — only
navigational links and the command-doc rule-source references were
corrected. Everything outside this table is upstream-verbatim.

**Re-sync** = clone upstream at the new tag, re-copy the mapped files,
re-apply exactly the delta table above (mechanical find/replace), bump
the version/commit table, re-run `/rust-cc-audit` on the changed
surface, commit as `chore(claude): sync rust-intel <ver>`. Any further
Nebula-only divergence is added to the delta table here — never left
undocumented in the vendored files.

## Structural-budget note (ADR-0083)

Vendoring a ~760-line third-party spec plus command docs is an
intentional bulk import, not re-implemented workspace utility code. The
Layer-2 duplicate-symbol gate matches `pub fn` examples inside
`SKILL.md` code fences (a false positive on documentation). The escape
token below satisfies the deterministic structural-budget gate on the
vendor commit and on every re-sync commit (which re-touches this file):

    // budget-justified: vendored third-party rust-intel spec (SKILL.md + docs + rust-cc-* commands) — reviewed bulk import; the only pub-fn collisions are markdown code-fence examples, not duplicate utilities

## Nebula-specific caveats (read before trusting blindly)

- **Advisory, not hook-enforced.** Nebula's mechanical gate is the
  `.claude/hooks/*` D10 core (`edit-guard`/`stop-gate`/`intent-gate`):
  it catches `unwrap`/`panic`/TODO/lint-suppression/structural-budget.
  rust-intel's Tier-B (§B2 Mutex-across-`.await`, §B3 cancel-safety,
  §B11 blocking-executor, §B13 TOCTOU, §B14 unbounded-channel) is
  **outside** that gate and outside clippy — it is checklist
  discipline, weaker than a hook. Treat it as a force-multiplier on
  review, not a guarantee.
- **§B3 tension.** rust-intel closes cancel-safety with doc-comments
  (`/// cancel-safe: NO`). Nebula's bar prefers structural/type
  enforcement over "remember to annotate" discipline. The comment
  approach is the upstream best-effort (cancel-safety is not
  type-expressible) — acceptable as a signal, not as the Nebula
  Definition-of-Done substitute.
- Layer/canon boundaries, `nebula-eventbus`-only cross-crate comms, and
  observability-as-DoD remain governed by `CLAUDE.md`, which outranks
  this skill on any conflict.
