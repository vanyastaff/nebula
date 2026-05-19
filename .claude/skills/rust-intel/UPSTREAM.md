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

## Maintenance rules

- **Verbatim. Do not hand-edit `SKILL.md` / `docs/` / `rust-cc-*.md`.**
  They are byte-exact upstream copies; hand-edits silently fork the spec
  and break re-sync. Frontmatter (`name:` absent — skill name derives
  from the directory) is upstream-as-is by design.
- **Re-sync** = clone upstream at the new tag, re-copy the mapped files,
  bump the version/commit table above, re-run `/rust-cc-audit` on the
  changed surface, commit as `chore(claude): sync rust-intel <ver>`.
- Local divergence, if ever required, goes in **this file** as an
  override note — never inline in the vendored files.

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
