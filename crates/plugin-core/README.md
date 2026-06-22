# nebula-plugin-core

The first-party **`core`** plugin: a set of pure, in-process actions that cover
the everyday building blocks of a workflow — filtering, sorting, aggregation,
reshaping, branching, batching, time arithmetic, and a durable timer wait.

Every action is **pure**: no I/O, no credentials, no resources. Inputs and
outputs are plain JSON (`serde_json::Value`), and errors are typed
(`ActionError::Fatal`) — there are no `unwrap`/`panic` on reachable paths.

## Actions

| Key | Name | What it does |
|-----|------|--------------|
| `core.filter` | Filter | Filter an array of JSON objects by a condition. |
| `core.sort` | Sort | Sort an array of objects by one or more fields (asc/desc), with per-key null placement and case-insensitive options. |
| `core.aggregate` | Aggregate | Reduce an array of objects to grouped/scalar summaries (sum/count/avg/min/max/collect/join). |
| `core.dedupe` | Dedupe | Remove duplicate array elements by one or more key fields (first occurrence wins). |
| `core.map` | Map | Reshape each element of an array (per-element pick/omit/rename/flatten). |
| `core.array` | Array | Shape a JSON array with chunk/flatten/take/skip operations applied left-to-right. |
| `core.json_transform` | JSON Transform | Apply a sequence of pick/omit/rename/flatten operations to a single JSON object. |
| `core.set_fields` | Set Fields | Merge a list of named field assignments onto a JSON object. |
| `core.datetime` | DateTime | Offset-aware RFC3339 timestamp formatting, parsing, arithmetic, and diff (millisecond-precise). |
| `core.delay` | Delay | Park the execution for a fixed duration (down to milliseconds) or until a timestamp, then resume. |
| `core.if` | If | Route execution to the `true` or `false` port based on a field condition. |
| `core.switch` | Switch | Route execution to the first matching case port, or `default` if none match. |

Numeric comparisons (`sort`, `if`/`switch` ordered ops, `aggregate` min/max)
compare integers **exactly** — large 64-bit IDs are not collapsed through `f64`.

## Runnable examples

Each example wires a real `WorkflowEngine` with this plugin and drives a workflow
end to end, asserting the result so it doubles as a smoke test. They live in the
root [`examples/`](../../examples) workspace member.

| Example | Demonstrates |
|---------|--------------|
| `workflow_data_pipeline` | `filter` → `sort` → `aggregate` over records |
| `workflow_batch_etl` | `dedupe` → `map` → `array` (dedup, project, batch) |
| `workflow_json_reshape` | `json_transform` object reshaping (flatten/omit/rename/pick) |
| `workflow_datetime_schedule` | `datetime` parse → add (ms interval) → format |
| `workflow_conditional_routing` | `if` binary branching with skip semantics |
| `workflow_switch_router` | `switch` multi-way routing incl. the `default` port |
| `workflow_delay_resume` | `delay` durable timer park → resume (wait-state) |

Run any of them with:

```sh
cargo run -p nebula-examples --example workflow_data_pipeline
```

## Wiring

The engine is wired by registering the plugin; the `core.*` actions then dispatch
through the normal node spine:

```rust,ignore
let engine = WorkflowEngine::new(runtime, metrics)?;
let core = Arc::new(ResolvedPlugin::from(CorePlugin::try_new()?)?);
let engine = engine.with_plugin(core)?;
```

See `workflow_data_pipeline` for the full standalone setup, or
`crates/plugin-core/tests/plugin_wiring_e2e.rs` for the wiring contract.
