
## Diff-scoped structural budget (ADR-0083)

The `cognitive_complexity` / `too_many_lines` workspace `allow` stays — flipping
them on 36 crates is thousands of legacy warnings. New code is instead held to
the `clippy.toml` thresholds **diff-scoped** by `.claude/hooks/intent-gate.sh`
(net-LoC, new-file, large-blob proxy, duplicate-symbol), with a
`// budget-justified:` escape. Legacy is grandfathered; the separate legacy
burn-down workstream reconciles it crate-by-crate.
