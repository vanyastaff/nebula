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

## Input schemas

Each input advertises a checked, cached record schema through `HasSchema`.
Construction failures propagate through plugin discovery; no input falls back
to an empty record or `Any`. The actions' genuinely arbitrary JSON outputs
advertise `Any`.

These input contracts use catalog interface version `2.0.0`; `core.delay`
retains `1.0.0` because its schema is unchanged. The core plugin bundle is
`2.0.0`. These entity versions are independent of the Rust package version.
Recorded plans keep their exact catalog revision and are not rebound to the
new schemas implicitly.

| Input | Declared structure |
|-------|--------------------|
| Aggregate | Object array, grouping keys, tagged aggregation records, error-policy choices. |
| Array | Arbitrary JSON array and tagged operations with bounded integer counts. |
| Dedupe / Sort | Object arrays and non-empty typed key lists; sort options and booleans. |
| Filter / If / Switch | Required condition objects; switch case records and ports. |
| Map / JSON Transform | Shared tagged transform records, string keys and key lists. |
| Set Fields | Assignment records with string names and arbitrary JSON values. |
| DateTime | Flattened operation tag, conditional fields, numeric bounds and duration choices. |
| Delay | Flattened wait mode and conditional duration or timestamp fields. |

These are **partial structural contracts**, not a claim of serde equivalence:

- Required data arrays permit `[]`. Root presence rules express this because
  form-style `Field::required()` also rejects empty collections. Non-empty key
  and aggregation lists use that stronger requirement intentionally.
- Optional means absent, not nullable. Object-or-null `data` fields stay
  explicitly dynamic; the existing action check validates their nullable
  shape. No null-to-object transformation is introduced.
- Recursive, key-sniffed `Condition` contents remain opaque inside a declared
  object. The real condition deserializer validates that recursive union.
- Operation records declare their tags and member types without pretending
  to be a different mode-envelope wire format. Serde still checks
  variant-specific missing members. Declared members are type-checked whenever
  present, including members serde would ignore for another selected variant.
- Empty JSON keys and null assignment values remain valid. Required nested
  string/value presence stays with serde where form-style requiredness would
  incorrectly reject those values.
- Serde still checks exact Rust integer representations, including nullable
  UTC offsets. Schema integer checks can accept integral JSON floats that
  serde rejects for integer fields. Timestamp parsing, conditional operation
  semantics and arithmetic overflow remain action responsibilities.

Consumers must complete schema resolution **and** decode the concrete input;
a schema proof alone does not guarantee that an opaque subtree is decodable.
Literal JSON ingress never interprets template strings or `$expr` objects as
programs. `tests/input_schema_contract.rs` exercises discovery, rejection,
serde round-trips, empty values and these explicit limits.

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
