# DeepWiki Findings — aqueducts-utils

## Query 1 — Core trait hierarchy for actions/nodes/activities

Question: "What is the core trait hierarchy for actions/nodes/activities? How is a pipeline step defined as a Rust type?"

Response summary: The core trait hierarchy in Aqueducts is not a trait hierarchy in the OOP sense. Pipeline steps (stages) are plain `Stage` structs containing a `name` and `query` field. The `ProgressTracker` is the only substantive trait; everything else is enum-dispatched data. There is no `Action`, `Node`, or `Activity` trait. Sources and destinations are plain enums (`Source`, `Destination`). Confirmed: the framework uses data-oriented pipeline config structs, not trait objects.

## Query 4 — Plugins or extensions (WASM/dynamic/static)

Question: "How are plugins or extensions implemented (WASM/dynamic/static)? Where do plugins compile and where do they execute?"

Response summary: Extensions are statically linked Rust crates, conditionally included via Cargo feature flags. `aqueducts-delta` and `aqueducts-odbc` are optional provider crates enabled at build time. No WASM, no dynamic library loading, no subprocess IPC. The `wasm-bindgen` transitive dependency found in Cargo.lock comes from indirect dependencies (likely DataFusion), not from Aqueducts' own extension mechanism.

## Query 7 — LLM or AI agent integration

Question: "Is there built-in LLM or AI agent integration? What providers and abstractions are supported?"

Response summary: No built-in LLM or AI agent integration. The `.claude/` entry in `.dockerignore` is a development artifact, not an integrated feature. The framework focuses purely on ETL: Sources → SQL Stages → Destinations.

## Query 9 — Known limitations and planned redesigns

Question: "What known limitations or planned redesigns are documented?"

Response summary: Roadmap items from README.md:
- Web server for pipeline management/orchestration (TODO)
- Apache Iceberg support (TODO)
- Data catalog for Delta Lake and Iceberg (TODO)

Completed:
- Docs, ODBC, parallel stage processing, remote execution, memory management

Key implicit limitations: single executor concurrency (one pipeline at a time by design), no workflow state persistence/recovery, no trigger system, no credential lifecycle management.
