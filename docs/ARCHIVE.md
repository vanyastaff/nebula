# Archived documentation (outside this repo)

The following were **removed from the Nebula git tree** on 2026-05-17 to
shrink agent context. They remain on disk for humans; agents must not search
for them inside this repository.

**Archive root (sibling tree, not in workspace):**

`C:/Users/vanya/RustroverProjects/docs/_nebula-repo-archive/2026-05-17/`

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
| ADR full text (0001–0041) | `docs/adr/NNNN-*.md` (indexed out in `.cursorignore` / `.claudeignore`) |
| Product direction | [`STRATEGY.md`](../STRATEGY.md) |

Recover deleted paths from git history: `git log -- docs/superpowers`.
