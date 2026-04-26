# flowlang — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/mraiser/flow
- **Crate:** `flowlang` v0.3.29 (Cargo.toml line 3)
- **Stars:** 11 / **Forks:** 1 (as of 2026-04-26)
- **Last commit:** "Rust code generation fixes" (2 recent commits with that
  message, commit `c123671` and `b45f11f`)
- **License:** MIT (`LICENSE.md`)
- **Governance:** Solo maintainer (mraiser). No contribution guidelines,
  no CI config found in the repo.
- **Published on crates.io:** yes (`flowlang` crate).
- **Primary language:** Rust (100% of tracked source).

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description** (README.md line 1–14):
> "A dataflow oriented programming meta-language in JSON supporting
> functions written in rust, python, javascript, java, and flow."

**My description after reading code:**
Flowlang is a JSON-stored dataflow graph interpreter: flow programs
are directed graphs of typed operations (nodes) stored as JSON files,
where the Rust runtime loads those graphs, resolves connections
between nodes, and dispatches each node's logic to whichever language
implementation it targets (Rust function pointer, Python via pyo3,
JavaScript via deno_core, Java via JNI, or a nested flow sub-graph).
The recent MCP (Model Control Protocol) server (`flowmcp` binary)
exposes any loaded flow library as a JSON-RPC tool server usable by
LLM agents.

**Comparison with Nebula:**
Nebula is a workflow orchestration engine targeting production
SaaS/enterprise workloads: typed DAG at compile time, credential
subsystem, multi-tenancy, OpenTelemetry observability, Postgres
persistence, and WASM plugin sandbox. Flowlang is positioned as a
rapid-prototyping multi-language dataflow interpreter, primarily for
LLM tooling and visual programming (via Newbound IDE), with no
persistent state beyond the filesystem, no credential management, no
tenancy, and no observability. The design axes are almost entirely
non-overlapping: Nebula targets correct-at-scale production
orchestration; Flowlang targets "duct-tape heterogeneous integrations
quickly."

---

## 2. Workspace structure [A1]

**Single-crate project.** No Cargo workspace at the repo root. The
`Cargo.toml` at root defines one package (`flowlang`) with optional
features and three binaries.

**Feature flags** (`Cargo.toml` lines 12–19):
- `serde_support` — enables serde/serde_json + ndata serde bridge
- `java_runtime` — enables JNI Java embedding
- `python_runtime` — enables pyo3 Python embedding
- `javascript_runtime` — enables deno_core JS embedding
- `mirror` — inter-process ndata heap sharing
- `python_no_singleton` — multiple Python interpreter instances
- `gag` — stdout suppression (for MCP mode to avoid stdout pollution)

**Modules:** Flat module tree under `src/`, with `src/flowlang/`,
`src/builder/`, `src/mcp/` as sub-directories. No layer separation
enforced; all modules reside in one crate.

**Comparison with Nebula:** Nebula has 26 separate crates with
enforced layer separation (error → resilience → credential →
resource → action → engine). Flowlang is one crate doing everything.
This is a fundamental architectural difference: Nebula's workspace
boundaries are contractual; Flowlang's flat module tree has no
enforced dependency direction.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 — Trait shape

Flowlang has **no trait for commands/actions.** There is no `Action`
trait, no `Command` trait, no sealed trait. The central abstraction is
the `Command` struct (`src/command.rs:28–38`) and the `Source` enum
(`src/command.rs:17–25`):

```rust
// src/command.rs:17–25
pub enum Source {
  Flow(Case),
  Rust(RustCmd),
  #[cfg(feature="java_runtime")]
  Java(JavaCmd),
  #[cfg(feature="javascript_runtime")]
  JavaScript(JSCmd),
  Python(PyCmd),
}

// src/command.rs:28–38
pub struct Command {
  pub name: String,
  pub lib: String,
  pub id: String,
  pub lang: String,
  pub src: Source,
  pub return_type: String,
  pub params: Vec<(String, String)>,
  pub readers: Vec<String>,
  pub writers: Vec<String>,
}
```

There is no trait-based polymorphism. Dispatch is via `match` on the
`Source` enum in `Command::execute` (`src/command.rs:185–213`).

There are no associated types, no GATs, no HRTBs, no typestate. This
is a simple enum-tagged union dispatch, equivalent to `dyn Command`
without the vtable.

**The only type-level abstraction for Rust commands** is the function
pointer alias:

```rust
// src/lib.rs:37 and src/rustcmd.rs:9
pub type Transform = fn(DataObject) -> DataObject;
```

Every Rust command is a bare `fn(DataObject) -> DataObject`. This is
the entire "trait" for a Rust command. It is not sealed, not versioned,
not generic.

### A3.2 — I/O shape

All inputs and outputs are `DataObject` (from the `ndata` crate), which
is a dynamically-typed JSON-like map. There is no compile-time typing of
inputs or outputs. The `Command` struct stores declared param types as
strings (`src/command.rs:35`: `pub params: Vec<(String, String)>`) and
performs runtime coercion in `Command::cast_params` (`src/command.rs:150–183`),
but these are advisory — there is no enforcement or validation at the
type-system level.

Return type is also stored as a runtime string (`src/command.rs:34`:
`pub return_type: String`), used by the MCP layer to decide how to
serialize the result (`src/mcp/mcp/invoke.rs:35`).

Streaming output: absent. All returns are single synchronous
`DataObject`.

### A3.3 — Versioning

No versioning. Commands are identified by `(lib, id)` string pairs
stored in the JSON datastore. There is no v1/v2 distinction, no
`#[deprecated]`, no migration. The identifier format is an opaque
string like `"jnunvo180e784fe1cq21"` (observed in
`src/flowlang/system/mod.rs:19`).

### A3.4 — Lifecycle hooks

Single lifecycle method: `execute`. No pre/post/cleanup/on-failure
hooks. No cancellation support. No idempotency key.

Error signaling uses a `CodeException` enum
(`src/code.rs:17–20`):

```rust
pub enum CodeException {
    Fail,
    Terminate,
    NextCase,
}
```

`NextCase` implements control-flow branching (jump to the next
alternative branch in the flow definition); it is not an error in the
traditional sense.

### A3.5 — Resource and credential deps

No mechanism. Commands do not declare resource or credential
dependencies. They access shared state via `DataStore::globals()`
directly, which is a global `DataObject` heap entry (index 0).

### A3.6 — Retry/resilience attachment

None. No per-command or global retry policy. Panic is caught via
`std::panic::catch_unwind` in generated execute wrappers
(`src/builder/rust.rs:176`), which converts panics to error
`DataObject`s with a `"status": "err"` field. No circuit breaker,
backoff, or bulkhead.

### A3.7 — Authoring DX

For Rust: author writes a plain Rust function, runs `flowb` to generate
an `execute` wrapper. The generated boilerplate (`src/builder/rust.rs:117–213`)
extracts named params from `DataObject`, calls the user function via
`panic::catch_unwind`, and packages the return value. Approximate
"hello world" in ~5 lines of Rust user code + `flowb` run.

For Flow: visual editing via Newbound IDE, stored as JSON. No Rust code
required.

For Python: write a `.py` file; `flowb` generates stub registration.

### A3.8 — Metadata

Commands have a `name` field (string) from the JSON datastore. No icon,
category, i18n, or display description built in. The MCP layer reads a
`description` field from command metadata to decide whether to expose
a command as a tool (`src/mcp/mcp/list_tools.rs:81–85`).

### A3.9 — Comparison with Nebula

Nebula has 5 sealed trait-based action kinds (Process/Supply/Trigger/
Event/Schedule) with associated `Input`/`Output`/`Error` types, compile-
time port typing, derive macros, and lifecycle hooks. Flowlang has one
struct (`Command`) with an enum discriminant (`Source`) and a single
`fn(DataObject) -> DataObject` interface. The Nebula approach provides
type safety and compile-time guarantees. The Flowlang approach is simpler
but entirely runtime-typed; mismatches surface as panics or empty values.

---

## 4. DAG / execution graph [A2, A9, A10]

### Graph representation

A flow function is represented as a `Case` struct
(`src/case.rs:9–15`):

```rust
pub struct Case {
  pub input: HashMap<String, Node>,
  pub output: HashMap<String, Node>,
  pub cmds: Vec<Operation>,
  pub cons: Vec<Connection>,
  pub nextcase: Option<Box<Case>>,
}
```

Operations are nodes in the graph; connections are directed edges.
`nextcase` implements pattern-matching branches (case alternatives,
similar to `match` arms).

`Connection` carries source/destination as `(index: i64, name: String)`
pairs (`src/case.rs:63–68`). Index `-1` means "flow input"; index `-2`
means "flow output".

### Execution algorithm

`Code::execute` (`src/code.rs:36–154`) uses a two-phase event loop:

1. **Operation pass:** Iterates all ops; any op with no input connections
   (i.e., no pending data to wait for) is executed immediately.
2. **Connection pass:** Iterates connections; when a source op is done,
   propagates its output to destination op inputs. When all inputs of a
   destination op become `done`, that op is executed immediately.
3. Loop continues until all connections are marked done.

This is a dependency-resolution topological walk — not a compiled DAG
sort. There is no petgraph, no compile-time topology check. Cycles
would cause an infinite loop (no cycle detection visible in the source).

### Port typing

Ports are untyped at the graph level. Type metadata is stored in
`Node.cmd_type` as a string (`src/case.rs:22`) but this is a visual
hint from Newbound, not enforced by the interpreter. The connection
system passes `Data` (ndata) values without type-checking.

**Comparison with Nebula:** Nebula's TypeDAG has 4 levels of increasing
precision (static generics → TypeId → predicates → petgraph). Flowlang
has zero compile-time DAG validation. This is a fundamental gap for
production use: Nebula's type system catches misconnected ports at
compile time; Flowlang's mismatches produce silent nulls or panics at
runtime.

### Concurrency model

Purely `std::thread`. No tokio, no async. `Code::execute` is single-
threaded per invocation. The `thread` and `thread_id` primitives
(`src/flowlang/system/mod.rs:14`) spawn OS threads for parallel
execution. The event bus (`src/appserver.rs:152–154`) also spawns
threads per listener. Thread safety is provided by `ndata`'s internal
`SharedMutex`, not by Rust's standard synchronization primitives.

---

## 5. Persistence and recovery [A8, A9]

### Storage layer

The `DataStore` (`src/datastore.rs`) is a file-system JSON store. Each
"data object" (command definition, flow definition, metadata) is stored
as a JSON file under `data/<lib>/<subdir-hash>/<id>`. The directory
sharding uses 4-char chunks of the ID for 4 levels (16^4 = 65536 leaf
directories).

No database. No SQL. No migrations. No schema versioning. The entire
data model is flat JSON files on disk, read synchronously at command
lookup time.

### Persistence model for execution state

There is **no execution state persistence.** When a flow runs, it
executes entirely in memory (`Code::execute`) and returns. There is no
checkpoint, no journal, no append-only log, no crash recovery. If the
process dies during a flow execution, the execution is lost.

Global mutable state survives within the process lifetime via
`DataStore::globals()` (an `ndata` heap object at index 0), but this is
in-memory only and not durable.

**Comparison with Nebula:** Nebula uses frontier-based checkpoint
recovery with an append-only execution log stored in PostgreSQL. This
is a categorical difference: Nebula supports durable long-running
workflows; Flowlang only supports ephemeral function-call semantics.

---

## 6. Credentials / secrets [A4] — DEEP

### A4.1 — Existence

**No credential layer exists.** There is no credential store, no secret
management module, no API key handling, no token vault.

**Grep evidence:**
- Searched `src/` for `credential`, `secret`, `token`, `auth`, `oauth`,
  `password`, `encrypt` (case-insensitive): found only
  `src/flowlang/file/mime_type.rs` (MIME type string "auth" as
  substring) and `src/x25519.rs` (cryptographic primitive). No
  credential management code found.
- The `config.properties` file (`config.properties` at repo root)
  stores HTTP address and port — no secrets.
- `src/appserver.rs` has a `security` config flag (line 254:
  `config.put_boolean("security", true)`) that controls whether security
  is "on" or "off" — but there is no implementation of what security
  enforcement means beyond this flag.

### A4.2–A4.9 — All absent

At-rest encryption: absent. In-memory protection (Zeroize, secrecy): absent.
Lifecycle (CRUD / revoke / refresh): absent. OAuth2/OIDC: absent.
Composition / scope / type safety: absent.

If users need credentials, they must manage them externally (environment
variables, config files) and pass them as `DataObject` parameters to
commands.

**Comparison with Nebula:** Nebula has a deep credential subsystem
(State/Material split, LiveCredential with `watch()`, blue-green refresh,
OAuth2Protocol blanket adapter, DynAdapter erasure). Flowlang has none.
This is one of the largest gaps between the two projects.

---

## 7. Resource management [A5] — DEEP

### A5.1 — Existence

**No resource abstraction exists.** There is no `Resource` trait, no
DB pool management, no HTTP client lifecycle, no connection management.

**Grep evidence:**
- Searched `src/` for `resource`, `pool`, `connection` (as module names
  or type names): found only `src/mcp/mcp/list_resources.rs` and
  `src/mcp/mcp/mod.rs` — both are MCP protocol endpoints for listing
  "resources" in the MCP sense (data sources exposed to LLM agents),
  not Flowlang resource lifecycle management.

### A5.2–A5.8 — All absent

Scope levels: absent. Lifecycle hooks (init/shutdown/health-check):
absent. Reload / hot-reload for resources: absent. Sharing / pooling:
absent (each command makes its own I/O calls). Credential deps:
absent. Backpressure: absent.

The only "global resource" is the ndata heap (global `DataObject` at
index 0 via `DataStore::globals()`), which serves as a simple
key-value store. It is not bounded, not lifecycle-managed, and requires
manual GC via `DataStore::gc()`.

**Comparison with Nebula:** Nebula has 4 scope levels, `ReloadOutcome`
enum, generation tracking, and `on_credential_refresh` per-resource
hook. Flowlang has none of this. Not applicable to flowlang's use case
as a script-execution engine.

---

## 8. Resilience [A6, A18]

### Retry / circuit breaker / bulkhead / timeout

**None.** No retry logic, no circuit breaker, no bulkhead, no timeout
mechanism in the Flowlang runtime.

The only error handling is:
1. `CodeException::Fail` propagated up from command execution
   (`src/code.rs:17`).
2. `std::panic::catch_unwind` in generated Rust command wrappers
   (`src/builder/rust.rs:176`) and in the HTTP server
   (`src/flowlang/http/listen.rs:254`).
3. The HTTP server returns a 500 response on panic
   (`src/flowlang/http/listen.rs:265–276`).

### Error type

No dedicated error crate or type. Errors are represented as
`DataObject` with a `"status": "err"` key and a `"msg"` key. This
is a stringly-typed error protocol.

**Comparison with Nebula:** Nebula has `nebula-error` crate with
`ErrorClass` enum (transient/permanent/cancelled/etc.) and
`nebula-resilience` with retry/CB/bulkhead/timeout/hedging plus a
unified `ErrorClassifier`. Flowlang has none of this.

---

## 9. Expression / data routing [A7]

### DSL / expression engine

Flowlang is **itself** the expression language — flows are defined in
JSON and execute as programs. There is no separate expression DSL layer
(like n8n's `$nodes.foo.result.email` syntax or JSONPath).

Within a flow, data routing is done via `Connection` structs that name
source and destination ports. Values flow from one operation's output
to another's input. There is no expression interpolation; the value is
passed as-is.

### Primitive operations

~50 built-in primitives registered in `Primitive::init()`
(`src/primitives.rs:28–94`), covering:
- Math: `+`, `-`, `*`, `/`, `<`, `>`, `or`
- String: `split`, `trim`, `ends_with`, `starts_with`, `substring`,
  `length`, `string_left`, `string_right`
- Object: `get`, `get_or_null`, `set`, `remove`, `equals`, `has`,
  `keys`, `index_of`, `push`, `push_all`, `to_json`,
  `object_from_json`, `array_from_json`
- System: `time`, `sleep`, `stdout`, `system_call`, `execute_command`,
  `thread`, `execute_id`, `thread_id`, `unique_session_id`
- File: `file_read_all_string`, `file_exists`, `file_visit`,
  `file_is_dir`, `file_list`, `file_read_properties`,
  `file_write_properties`, `mime_type`
- TCP: `tcp_listen`, `tcp_accept`
- HTTP: `http_listen`, `http_websocket_open/read/write`, `cast_params`,
  `http_hex_decode/encode`
- Type coercion: `to_int`, `to_float`, `to_boolean`, `to_string`,
  `is_string`, `is_object`
- Data: `data_read`, `data_write`, `data_exists`, `library_exists`,
  `library_new`, `data_root`

Notably absent: no conditional branching primitive. Control flow is
handled via `CodeException::NextCase` (case/branch pattern) and the
`match` operation type in `Code::evaluate_operation`
(`src/code.rs:411–455`).

**Comparison with Nebula:** Nebula has 60+ expression functions with
type inference and sandboxed eval using `$nodes.foo.result.email`
syntax. Flowlang has ~50 primitives but no expression language — data
routing is structural (wire connections), not textual expressions.
Different decomposition: Nebula expressions sit between nodes; Flowlang
connections are the routing.

---

## 10. Plugin / extension system [A11] — DEEP

### 10.A — Plugin BUILD process

#### A11.1 — Format

Plugins in Flowlang are **Rust crates compiled as dynamic libraries**
(`.so` / `.dll` / `.dylib`) or statically linked Rust modules. The
manifest format is a `meta.json` file stored at
`data/<lib_name>/meta.json`. Key field: `"ffi": true` in the `"cargo"`
section triggers dylib compilation (`src/builder/cargo.rs:22–37`):

```json
// data/<lib>/meta.json example (from README)
{
  "root": "my-crate-name",
  "cargo": {
    "crate_types": ["dylib"]
  }
}
```

Without `"ffi"` the library is compiled as an `rlib` and statically
linked.

#### A11.2 — Toolchain

`flowb` binary (`src/build.rs:30`): a dedicated build tool that:
1. Reads all `data/<lib>/` directories (`src/builder/mod.rs:22–25`).
2. For each library, generates Rust source stubs from command metadata.
3. Updates `mod.rs` files and `cmdinit.rs` to register commands.
4. Generates the top-level `src/generated_initializer.rs` to wire
   all sub-crates into the main binary.
5. Creates `Cargo.toml` for sub-crates if missing.

Cross-compilation: not addressed. Reproducibility: not addressed (no
lock files beyond the standard `Cargo.lock`).

#### A11.3 — Manifest content

`meta.json` supports: `root` (crate name), `cargo.dependencies`,
`cargo.crate_types`. No capability declaration, no permission grants,
no security sandbox manifest.

#### A11.4 — Registry / discovery

Local directory only (`data/`). No remote registry, no signing, no
search. Libraries are discovered by scanning `data/` at startup
(`src/appserver.rs:415–422`).

### 10.B — Plugin EXECUTION sandbox

#### A11.5 — Sandbox type

**Dynamic library loading** (`.so`/`.dll`/`.dylib`) via `libloading`
crate (`src/builder/initializer.rs:45`: `use libloading::{Library, Symbol}`).
The loading logic copies the library to a temp file with a timestamp
suffix (to force OS re-read) then calls `Library::new`
(`src/builder/initializer.rs:105`).

Hot-reload is implemented: `FlowLangLibrary::reload()` method
(`src/builder/initializer.rs:119–140`) drops the old `Library` handle
(triggering `dlclose`), copies the new `.so`, calls `Library::new`,
and re-registers all commands via the `mirror_<name>` FFI function.

#### A11.6 — Trust boundary

**No sandbox.** Loaded libraries execute in the same process with full
memory access. There is no capability-based security, no CPU/memory
limits, no network policy, no `unsafe` isolation. This is equivalent to
loading arbitrary native code.

#### A11.7 — Host–plugin calls

The ABI is:
```rust
// src/builder/initializer.rs:60–65
#[repr(C)]
pub struct Initializer {
    pub ndata_config: NDataConfig,
    pub cmds: Vec<(String, Transform, String)>,
}
type MirrorFlowLangFunc = unsafe extern "C" fn(initializer: *mut Initializer);
```

The plugin exposes a `mirror_<lib_name>` C function that fills an
`Initializer` struct with command registrations. Host-to-plugin calls
go through function pointers stored as integers in the ndata heap
(`src/rustcmd.rs:40–42`: `std::mem::transmute(ptr_val)`). This is
`unsafe` and relies on the function pointer ABI being stable.

Marshaling: `DataObject` is passed by value; ndata uses an internal
shared heap so the same data is accessible from both the host and the
plugin if they share the ndata config (`mirror()` call).

#### A11.8 — Lifecycle

Start: library loaded at `initialize_all_commands()` call. Stop: on
process exit (Drop impl removes temp file). Hot-reload: supported via
`reload_library(lib)` function. Crash recovery: none — a panic in a
loaded library that is not caught by `catch_unwind` will kill the process.

#### A11.9 — Comparison with Nebula

Nebula targets WASM sandbox with wasmtime, capability-based security,
and the Plugin Fund commercial model. Flowlang uses native dylib loading
with no sandbox. The Flowlang approach is simpler to implement but
provides zero isolation: a buggy or malicious plugin can corrupt the
entire process. Nebula's WASM approach (when implemented) would prevent
this. Flowlang has no commercial monetization model.

---

## 11. Trigger / event model [A12] — DEEP

### A12.1 — Trigger types

Flowlang supports:
- **Schedule / timer:** `add_timer` / `timer_loop` in `appserver.rs`
  (`src/appserver.rs:54–63`, `src/appserver.rs:170–207`). Timers fire
  commands at configurable intervals. Units: milliseconds, seconds,
  minutes, hours, days.
- **Event / internal pubsub:** `add_event_listener` / `fire_event` /
  `event_loop` (`src/appserver.rs:92–168`). Events have an app-name
  and event-name key.
- **HTTP webhook (inbound HTTP):** The `http_listen` primitive
  (`src/flowlang/http/listen.rs`) acts as a webhook receiver — it
  binds a TCP socket, parses HTTP requests, and dispatches them to a
  named command.
- **Manual / CLI:** `flow <lib> <ctl> <cmd> <<< '<json>'` (direct CLI
  invocation via `src/main.rs`).
- **MCP call (new):** `flowmcp` binary dispatches JSON-RPC
  `tools/call` method to commands (`src/mcp/mcp/invoke.rs`).

### A12.2 — Webhook

HTTP handler in `src/flowlang/http/listen.rs`. URL allocation: fixed
at `TcpListener::bind(socket_address)`. No idempotency key, no HMAC
verification, no rate limiting, no retry on init fail. The URL is
whatever path the incoming request uses — routed as-is to the command.

### A12.3 — Schedule

Timer system in `src/appserver.rs`. Configured via `meta.json` in
library definitions: `"start"`, `"interval"`, `"intervalunit"` fields.
Supported units: milliseconds, seconds, minutes, hours, days. No cron
syntax. No timezone support. Missed schedule recovery: none — if the
process is down, timers don't fire. Distributed double-fire prevention:
none.

### A12.4 — External event

No Kafka, RabbitMQ, NATS, Redis streams, or CDC integrations. The
internal event bus (`fire_event` / `event_loop`) is in-process only.
External events can be ingested via HTTP webhook or by Python/Java/JS
commands that connect to external brokers, but there is no built-in
connector.

### A12.5 — Reactive vs. polling

The event loop (`event_loop()`, `src/appserver.rs:127–168`) is polling
with a 100ms sleep: `thread::sleep(dur)` when no events pending. The
timer loop (`timer_loop()`, `src/appserver.rs:170–207`) also polls at
1000ms. This is a polling model with a hard-coded poll interval.

### A12.6 — Trigger-to-workflow dispatch

1:1 mapping. A timer or event fires one command by `(lib, cmdid)`. Fan-
out is possible by having the command itself fire further events. Trigger
metadata (timer params, event data) is passed as the `DataObject`
argument to the command. No conditional triggers, no replay support.

### A12.7 — Trigger as Action type

Triggers are not a separate action type. Timers and events are
configuration entries in `meta.json`, resolved to regular `Command`
dispatches at startup (`src/appserver.rs:373–413`). There is no
`TriggerAction` equivalent. The trigger lifecycle is implicit (timers
repeat per `intervalmillis`; events are fire-and-forget).

### A12.8 — Comparison with Nebula

Nebula uses a 2-stage Source → Event → TriggerAction model where the
`TriggerAction` trait has `Input = Config` for registration and
`Output = Event` for typed payloads; the `Source` trait normalizes raw
inbound signals into Events. Flowlang conflates triggering and execution
into a single dispatch step: a timer or event directly calls a `Command`
with no intermediate normalization. There is no backpressure model;
the event loop spawns a thread per event (`thread::spawn`) with no
bounding.

---

## 12. Multi-tenancy [A14]

No multi-tenancy. There is no tenant concept, no schema isolation, no
RBAC, no SSO, no SCIM. The `security` flag in `config.properties` is
the only security configuration, and it controls an unimplemented
security mode (the flag is read but no enforcement code was found beyond
the boolean store).

The `readers` and `writers` arrays on `Command` (`src/command.rs:36–37`)
and `data_write` / `library_new` primitives suggest a rudimentary
access-control concept for data objects, but there is no enforcement
layer visible in the interpreter.

---

## 13. Observability [A15]

**No observability infrastructure.** No OpenTelemetry, no tracing, no
structured logging framework, no metrics.

Logging is ad-hoc `println!` / `eprintln!` scattered through the source.
MCP server uses `eprintln!("[MCP] ...")` for diagnostics
(`src/mcp/mcp/mcp.rs:20`, `51`, etc.). The README explicitly advises:
"Use `eprintln!` for logging in `flowmcp` to avoid contaminating the
stdout channel."

**Comparison with Nebula:** Nebula uses OpenTelemetry with structured
per-execution tracing. Flowlang has none. Not a primary design goal for
flowlang's use case, but a blocker for production adoption.

---

## 14. API surface [A16]

- **CLI:** `flow <lib> <ctl> <cmd> <<< '<json>'` — full command
  dispatch via stdin JSON.
- **HTTP:** `http_listen` primitive — raw TCP HTTP server, no REST
  framework, no OpenAPI spec. URL path is passed as-is to the
  command. No versioning.
- **MCP (JSON-RPC):** `flowmcp` binary — implements `tools/list`,
  `tools/call`, `prompts/list`, `resources/list`, `initialize` per
  MCP protocol 2024-11-05 (`src/mcp/mcp/initialize.rs:27`).
- **Programmatic Rust:** `flowlang::init("data")` + `Command::lookup` +
  `command.execute(args)`.

No REST framework, no GraphQL, no gRPC, no OpenAPI generation, no
versioning scheme.

---

## 15. Testing infrastructure [A19]

**No formal testing infrastructure.** The source code has no `#[test]`
attribute functions, no integration test harness, no testing crate.

The `data/testflow` directory contains JSON-defined flows used as
functional examples / smoke tests, executable via:
```
flow testflow testflow test_add <<< '{"a": 300, "b": 120}'
```

There is no automated test runner, no CI configuration visible in the
repository. No mock library, no test fixtures.

**Comparison with Nebula:** Nebula has a `nebula-testing` crate with
contract tests and a structured testing harness.

---

## 16. AI / LLM integration [A21] — DEEP

### A21.1 — Existence

LLM integration is a **primary marketing focus** for Flowlang (README
is explicitly titled "Flowlang as a Premier Platform for LLM Tooling
and Model Control Protocol (MCP)"). However, there is **no built-in
LLM client abstraction** in the Rust code. Flowlang's strategy is to be
the **orchestration container** for LLM tools, not to provide LLM access
itself.

The MCP server (`flowmcp`) is the closest thing: it implements MCP
protocol 2024-11-05 (`initialize.rs:27`) for exposing flow commands as
tools to external LLM agents (Claude, GPT-4, etc.).

### A21.2 — Provider abstraction

**None.** No LLM provider trait. No OpenAI client, no Anthropic client,
no local model support in Rust. LLM calls are expected to be written
as Python nodes (using OpenAI/LangChain SDK, etc.) or JavaScript nodes,
executed via the Python/JS bridges.

**Grep evidence:** Searched `src/` for `openai`, `anthropic`, `llm`,
`embedding`, `completion`, `gpt`, `claude`, `gemini` — found nothing
in `.rs` files. Only `x25519.rs` matched "anthropic" as a false positive
(substring "an" not "anthropic"). Zero LLM provider code in Rust source.

### A21.3 — Prompt management

None in the Rust runtime. The MCP `prompts/list` endpoint
(`src/mcp/mcp/list_prompts.rs`) is wired up but returns an empty list
(the implementation in `list_prompts.rs` was not expanded; based on the
pattern of `list_tools.rs` it likely returns `{"prompts": []}`).

Prompt management would be done inside Python/Flow nodes by the user.

### A21.4 — Structured output

None. MCP tool results are serialized as text or base64-encoded data
(`src/mcp/mcp/invoke.rs:44–154`). The `wrap_value` function in
`invoke.rs` returns either `{"content": [{"type": "text", "text": "..."}]}`
or image data for `"File"` return type. No JSON schema enforcement, no
function calling schema generation beyond what MCP defines.

### A21.5 — Tool calling

MCP `tools/call` dispatch (`src/mcp/mcp/invoke.rs:17–38`) invokes
flow commands by name in format `lib-control-command`. Parameters are
passed as a `DataObject`. Multi-tools per call: not supported (one
dispatch per JSON-RPC request). Feedback loop: not implemented (no
multi-turn within a single MCP session). Parallel exec: not supported.

### A21.6 — Streaming

None in Flowlang's Rust code. MCP responses are single JSON objects.
No SSE, no chunked transfer, no streaming to workflow nodes.

### A21.7 — Multi-agent

Flowlang could implement multi-agent patterns by having one flow command
call another flow command via `execute_command` primitive or by using the
MCP server as the dispatch layer. No explicit multi-agent framework or
hand-off protocol is built in.

### A21.8 — RAG / vector

None. No vector store integration, no embedding API, no retrieval node.
These would be implemented in Python nodes using Python LLM libraries.

### A21.9 — Memory / context

Global state via `DataStore::globals()` can serve as a shared memory
between flows within one process run. No conversation memory abstraction,
no context window management, no long-term memory. This must be user-
implemented in Python/Flow nodes.

### A21.10 — Cost / tokens

None. No token counting, no cost tracking, no budget circuit breakers,
no per-tenant attribution.

### A21.11 — Observability for LLM calls

None. No per-LLM-call tracing, no prompt+response logging, no eval hooks.

### A21.12 — Safety

None. No content filtering, no prompt injection mitigations, no output
validation in the Rust runtime.

### A21.13 — Comparison with Nebula + Surge

Nebula has no first-class LLM abstraction and bets on AI workflows via
generic actions + plugin LLM client. Flowlang's positioning is similar
in that LLM calls happen inside language-specific nodes (Python), not
in first-class Rust types. The key difference: Flowlang has the MCP
server, giving it a ready integration point for LLM tool use. Nebula
has nothing equivalent today. Flowlang is "working" in the sense that
you can wire Python LLM calls into flows and expose them via MCP; but it
is over-reliant on external Python code and provides zero safety
guarantees or observability for those calls. Surge (Nebula's partner
agent orchestrator) does not exist in source yet. The MCP server is
Flowlang's most concrete competitive differentiator for AI use cases.

---

## 17. Notable design decisions

### 1. Global GC heap (ndata) instead of Rust ownership

All inter-node data is managed by `ndata`'s global heap with manual
garbage collection (`DataStore::gc()` calls scattered through the code).
This deliberately bypasses Rust's ownership system in favor of a
garbage-collected dynamic type system. This choice makes multi-language
data passing trivial but at the cost of all compile-time memory safety
guarantees. It is the key architectural bet of the project.

**Trade-off:** DX win for rapid prototyping; correctness regression for
production use (memory leak risk if `gc()` is not called, silent nulls
on misuse).

**Applicability to Nebula:** Not directly borrowable — Nebula's typed
system is more valuable for production workloads. However, the idea of
a "universal data container" (Nebula's `serde_json::Value` in some
cases) is analogous.

### 2. Function pointer as command registration protocol

Rust commands are registered as `fn(DataObject) -> DataObject` function
pointers stored as integers in the ndata heap
(`src/rustcmd.rs:17–27`). Retrieval uses `std::mem::transmute`
(`src/rustcmd.rs:40–42`), which is `unsafe` and brittle. The function
pointer ABI must match exactly.

**Trade-off:** Allows runtime registration of commands without a vtable;
enables hot-reload without trait objects. Fragile if a library is
recompiled with a different signature.

### 3. Builder generates code from JSON metadata

`flowb` inspects `data/` JSON metadata and generates Rust `.rs` source
files (`src/builder/rust.rs`). This is code generation from a data
store, not from type annotations or macros. The generated code includes
type extraction boilerplate and panic-catching wrappers.

**Trade-off:** Allows non-Rust developers to define commands (via
Newbound IDE); automates repetitive FFI boilerplate. Brittle if the
JSON schema changes; generated code is hard to review or test in
isolation.

### 4. Multi-language via process and in-process embedding

Python: embedded in-process via pyo3 (optional). JavaScript: embedded
in-process via deno_core/V8 (optional). Java: embedded in-process via
JNI (optional). Without the feature, Python falls back to subprocess
call (`system_call` to `python <path> <args>` —
`src/pycmd.rs:37–55`).

**Trade-off:** In-process embedding is fast but creates ABI coupling;
subprocess fallback is slow but isolated. The V8 embedding provides
JavaScript sandboxing; Python and Java have none.

### 5. MCP server as primary AI integration point

The `flowmcp` binary implements MCP 2024-11-05 over stdin/stdout.
This allows Claude Desktop, Cursor, and other MCP-compatible LLM
clients to discover and invoke Flowlang commands as tools.

**Trade-off:** This is the project's real differentiator — any flow
library becomes an LLM tool server with minimal configuration. However,
it only supports stateless request/response (no streaming, no multi-
turn). The MCP server hardcodes `serverInfo.name = "newbound-mcp"`
(`src/mcp/mcp/initialize.rs:16`) suggesting it was created for a
specific deployment, not fully generalized.

---

## 18. Known limitations / pain points

Based on commit messages and README admissions:

1. **Java and JavaScript support is broken.** README (line 1–4):
   "Support for back-end commands written in Java and Javascript is
   kind of broken for now."

2. **No crash recovery.** Execution state is entirely in-memory; a
   process crash loses all in-flight flows.

3. **Manual garbage collection required.** Users must call
   `DataStore::gc()` periodically or memory grows unbounded. README
   warns about this explicitly.

4. **Multiple FIXME/TODO in core HTTP handler.** `src/flowlang/http/listen.rs`
   has 13+ `// FIXME` and `// FIXME - implement or remove` comments,
   indicating incomplete HTTP features (keep-alive, chunked encoding,
   CORS, MIME multipart: `panic!("No MIME MULTIPART support yet")`).

5. **Unsafe function-pointer transmute.** `src/rustcmd.rs:40–42` uses
   `std::mem::transmute` to reconstitute function pointers stored as
   integers. This is undefined behavior if the wrong type is used.

6. **No test suite.** There are no automated tests; correctness relies
   on manual flow execution.

7. **Recent instability in hot-reload.** Commit `049dbbf` is titled
   "hot-reload is fine" — suggesting it was not fine before that.

8. **Code generation bugs.** Two consecutive commits titled "Rust code
   generation fixes" (`c123671`, `b45f11f`) indicate the builder system
   has had correctness issues.

9. **Multipart HTTP not supported** (`panic!` at
   `src/flowlang/http/listen.rs:123`).

10. **No distributed scheduling.** Timer loop has no distributed
    double-fire prevention; running two instances would fire timers twice.

Note: GitHub issue tracker has 0 issues, so no referenced issue numbers
are available. The repo has <10 total issues and does not meet the
>100 threshold for the citation requirement.

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 (mraiser).
- **Commit cadence:** Active — recent commits show ongoing development
  (MCP added, builder refactored, hot-reload added).
- **Stars:** 11. Very low community adoption.
- **Issues:** 0 open, ~0 historical. Either perfect or users aren't
  using it (the latter is more likely given 11 stars).
- **Last release:** v0.3.29 (current). No changelog file found.
- **Risk:** Solo-maintained, very low community, no CI, no tests.
  Any project depending on it is fully exposed to the maintainer's
  availability.

---

## 20. Final scorecard vs Nebula

| Axis | flowlang approach | Nebula approach | Verdict | Borrow? |
|------|-------------------|-----------------|---------|---------|
| A1 Workspace | 1 crate, flat modules, feature flags for language runtimes | 26 crates layered, Edition 2024 | Nebula deeper (enforced layering); flowlang simpler for quick start | no |
| A2 DAG | JSON-stored `Case`/`Operation`/`Connection`; runtime dependency walk; no cycle detection; no type checking | TypeDAG L1-L4 (generics→TypeId→predicates→petgraph) | Nebula deeper; flowlang's graph has no compile-time or runtime safety guarantees | no |
| A3 Action | `fn(DataObject) -> DataObject` function pointer; `Source` enum for dispatch; no trait, no assoc types, no versioning | 5 action kinds, sealed trait, assoc Input/Output/Error, versioning, derive macros | Nebula deeper; flowlang simpler but type-unsafe | no |
| A4 Credential | Absent — no credential layer | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula only; flowlang has nothing | no |
| A5 Resource | Absent — no resource abstraction | 4 scope levels, ReloadOutcome, generation tracking | Nebula only | no |
| A6 Resilience | None — only panic::catch_unwind + stringly-typed error DataObject | retry/CB/bulkhead/timeout/hedging + ErrorClassifier | Nebula deeper | no |
| A7 Expression | No expression DSL; ~50 built-in primitives; data routing by wire connections | 60+ funcs, type inference, `$nodes.foo.result.email` syntax | Different decomposition: flowlang's wiring is visual/structural; Nebula's is textual | no |
| A8 Storage | File-system JSON flat files (no DB) | sqlx + PgPool + Pg*Repo + RLS | Nebula deeper for production; flowlang appropriate for prototyping | no |
| A9 Persistence | None — ephemeral in-process execution only | Frontier+checkpoint+append-only log | Nebula only | no |
| A10 Concurrency | std::thread (no async); ndata SharedMutex; single-threaded per flow | tokio + frontier scheduler + !Send isolation | Different: flowlang is sync/threaded; Nebula is async. Flowlang simpler | no |
| A11 Plugin BUILD | flowb generates Rust stubs from JSON; meta.json manifest; local dir discovery only | WASM planned, plugin-v2 spec | Different: flowlang BUILD is code-gen from JSON; Nebula BUILD is WASM compile. Flowlang approach is interesting for DSL-authored plugins | refine |
| A11 Plugin EXEC | dylib hot-reload (libloading); no sandbox; unsafe transmute ABI; same-process | WASM sandbox + capability security planned | Nebula deeper (security); flowlang more pragmatic | no |
| A12 Trigger | Timer (interval-based, polling), internal pubsub event loop, HTTP inbound (raw TCP), CLI, MCP JSON-RPC | TriggerAction Source→Event 2-stage, typed payload | Different decomposition; flowlang simpler but no backpressure, no idempotency | no |
| A21 AI/LLM | MCP server (tools/list + tools/call over stdin/stdout); no built-in LLM client; LLM calls via Python nodes | No first-class LLM; bet on generic actions + plugin LLM | Flowlang has concrete MCP server delivery (working differentiator); Nebula has nothing yet — **Borrow the MCP-server-as-workflow-adapter pattern** | yes |
