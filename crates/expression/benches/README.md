# Nebula Expression Benchmarks

Criterion benchmark suite for `nebula-expression`.

## Running

```bash
# All benchmark groups
cargo bench -p nebula-expression --bench baseline

# One group (prefix match)
cargo bench -p nebula-expression --bench baseline -- template
cargo bench -p nebula-expression --bench baseline -- engine
cargo bench -p nebula-expression --bench baseline -- context
cargo bench -p nebula-expression --bench baseline -- concurrent
cargo bench -p nebula-expression --bench baseline -- builtins
```

## Comparing runs

```bash
# Save a baseline before a change
cargo bench -p nebula-expression -- --save-baseline before

# After the change, compare against it
cargo bench -p nebula-expression -- --baseline before
```

Criterion writes reports to `target/criterion/`; open
`target/criterion/report/index.html` in a browser for the overview.

## Groups

| Group | Measures |
|-------|----------|
| `template/parse/*` | `Template::new` on simple, multi-expression, and complex HTML sources |
| `template/render/*` | Rendering parsed templates against a context |
| `template/clone` | `Template` cloning |
| `engine/evaluate_no_cache/*` | `ExpressionEngine::evaluate` with no cache |
| `engine/evaluate_with_cache/*` | Cache hit and miss paths |
| `context/operations/*` | `EvaluationContext` clone and variable lookup |
| `concurrent/access/*` | Shared-engine evaluation across 1-8 threads |
| `builtins/*` | Representative string, math, array, and conversion builtins |

## Profiling

```bash
# Flamegraph
cargo install flamegraph
cargo flamegraph -p nebula-expression --bench baseline -- --bench template

# CPU profile (Linux)
perf record -g cargo bench -p nebula-expression --bench baseline
perf report
```

## Adding a benchmark

1. Put it in the group that matches what it measures, or start a new group.
2. Pass inputs through `black_box`.
3. Record the observed numbers in the PR description, not in this file — stale
   targets in prose are worse than no targets.
