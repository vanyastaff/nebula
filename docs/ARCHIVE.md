# Archived documentation (outside this repo)

The following were **removed from the Nebula git tree** (including `docs/superpowers/` as of 2026-05-18) on 2026-05-17 to
shrink agent context. They remain on disk for humans; agents must not search
for them inside this repository.

**Archive root (outside this git repo):** a sibling documentation tree under the
maintainer’s local `RustroverProjects/docs/_nebula-repo-archive/2026-05-17/`.
Other machines: recover removed paths from git history (below) or ask the maintainer
for the archive export — do not assume a fixed absolute path.

| Moved path | Contents |
|------------|----------|
| `superpowers/` | Historical execution plans and design drafts |
| `audits/` | Point-in-time architecture audits |
| `brainstorms/` | Strategy brainstorm (superseded by `STRATEGY.md`) |
| `CONFERENCE-NOTES.md`, `CONFERENCE-DAY9.md` | Session notes |
| `adr-historical/` | Full copy of ADRs before flatten; includes alternate `0042-tool-provider-*` |

**In-repo substitutes**

| Need | Use |
|------|-----|
| Agent doc routing | [`docs/README.md`](./README.md) |
| ADR 0001–0041 index | [`docs/adr/HISTORICAL.md`](./adr/HISTORICAL.md) |
| ADR full text (0001–0041) | git history only — `git log -- docs/adr/<file>` (evicted from tree 2026-05-18) |
| Product direction | [`STRATEGY.md`](../STRATEGY.md) |

Recover deleted paths from git history, e.g. `git log -- docs/superpowers` or `git log -- docs/adr/0001-schema-consolidation.md`.
