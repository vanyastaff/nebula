# Issues — ebi_bpmn

## GitHub issue data

Queried via: `gh issue list --repo BPM-Research-Group/Ebi_BPMN --state all --limit 30`

Result: **0 issues** (open or closed). The repository has no issue tracker activity as of 2026-04-26.

This is consistent with the project age (~7 weeks since creation on 2026-02-24) and the single-maintainer academic context. There is no community engagement via GitHub issues.

## Known pain points (from README and commit history instead)

1. Sub-process support incomplete — `marking.rs:78` and `marking.rs:125` return explicit "not supported" errors for sub-process markings.
2. OR join semantics under active development — commit e56578d "expand support for OR joins in loops" and earlier partial-order run commits marked "[does not compile]" (e0b5f54, 7d4b717, 22aa62e).
3. `bitvec` 64-bit limit on inclusive gateway outgoing flows — documented in README.
4. No BPMNDI (layout) support — documented limitation.
5. Stochastic sub-process and multi-pool limitations — documented in README Stochastic section.
