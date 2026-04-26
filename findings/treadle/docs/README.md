# Treadle

[![][build-badge]][build]
[![][crate-badge]][crate]
[![][tag-badge]][tag]
[![][docs-badge]][docs]
[![License](https://img.shields.io/crates/l/treadle.svg)](LICENSE-MIT)

[![][logo]][logo-large]

*A persistent, resumable, human-in-the-loop workflow engine backed by a petgraph DAG*

> **Status: v1 Complete** — The core engine is implemented and tested.
> A v2 design adding quality gates, retry budgets, and review policies is
> in progress. Contributions and feedback are welcome.

---

## What Is Treadle?

Treadle is a lightweight workflow engine for Rust that tracks **work items**
as they progress through a **directed acyclic graph (DAG) of stages**, with
**persistent state**, **human review gates**, and **fan-out with per-subtask
visibility**.

It fills a specific gap in the Rust ecosystem: the space between single-shot
DAG executors (define stages, run once, get results) and heavyweight
distributed workflow engines (durable execution, external runtime servers,
replay journals). Treadle is designed for **local, single-process pipelines**
where you need the pipeline to survive restarts, pause for human decisions,
and show you exactly where every item stands.

The name comes from the **treadle** — the foot-operated lever that drives a
loom, spinning wheel, or lathe. The machine has stages and mechanisms, but
without the human pressing the treadle, nothing moves. This captures the core
design: a pipeline engine where human judgement gates the flow.

## Why Treadle?

If you're building a CLI tool or local service that processes items through
multiple stages — and you need persistence, resumability, and human review —
your current options in Rust are:

- **Single-shot DAG executors** (dagrs, dagx, async_dag): Great for
  "define tasks, run them in parallel, get results." But they have no
  persistent state, no pause/resume, no concept of work items progressing
  over time. If your process crashes, you start over.

- **Distributed workflow engines** (Restate, Temporal, Flawless): Powerful
  durable execution with journaled replay. But they require an external
  runtime server, are designed for distributed microservices, and are
  enormous overkill for a personal CLI tool or local pipeline.

- **DAG data structures** (daggy, petgraph): Excellent building blocks, but
  they're data structures, not execution engines. You still need to build
  the state tracking, execution logic, and review workflow yourself.

Treadle occupies the middle ground: a **library** (not a service) that gives
you persistent, resumable, inspectable DAG execution with human-in-the-loop
gates, without requiring any external infrastructure.

## Core Concepts

### Work Items

A work item is anything flowing through your pipeline. It could be a file to
process, a record to enrich, an image to transform — anything that needs to
pass through multiple stages. You define what a work item is by implementing
the `WorkItem` trait:

```rust
pub trait WorkItem: Debug + Send + Sync {
    fn id(&self) -> &str;
}
```

The trait is object-safe, so you can use `&dyn WorkItem` for dynamic dispatch
across heterogeneous work item types.

### Stages

A stage is a single step in the pipeline. You implement the `Stage` trait to
define what happens at each step:

```rust
#[async_trait]
pub trait Stage: Debug + Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, item: &dyn WorkItem, ctx: &mut StageContext) -> Result<StageOutcome>;

    // Optional hooks
    async fn before_execute(&self, item: &dyn WorkItem, ctx: &StageContext) -> Result<()> { Ok(()) }
    async fn after_execute(&self, item: &dyn WorkItem, ctx: &StageContext, outcome: &StageOutcome) -> Result<()> { Ok(()) }
}
```

Stages return a `StageOutcome` indicating what happened:

- **`Complete`** — Stage succeeded. Dependents can now run.
- **`NeedsReview`** — Stage produced results that need human approval before
  the pipeline continues.
- **`Failed`** — Stage failed permanently.
- **`Retry`** — Stage failed and should be retried.
- **`FanOut(Vec<SubTask>)`** — Stage spawned multiple concurrent subtasks
  (e.g., fetching from several APIs). Each subtask is tracked independently.

### The DAG

Stages are connected in a directed acyclic graph using petgraph. This gives
you topological ordering (stages run in dependency order), cycle detection at
build time, and an inspectable graph structure for status display:

```rust
let workflow = Workflow::builder()
    .stage("scan", ScanStage)
    .stage("enrich", EnrichStage)
    .stage("review", ReviewStage)
    .stage("export", ExportStage)
    .dependency("enrich", "scan")
    .dependency("review", "enrich")
    .dependency("export", "review")
    .build()?;
```

### Persistent State

Every work item's progress through the DAG is tracked in a durable state
store. The default implementation uses SQLite, but the `StateStore` trait can
be implemented for any backend:

```rust
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn save_stage_state(&mut self, item_id: &str, stage: &str, state: &StageState) -> Result<()>;
    async fn get_stage_state(&self, item_id: &str, stage: &str) -> Result<Option<StageState>>;
    async fn get_all_stage_states(&self, item_id: &str) -> Result<HashMap<String, StageState>>;
    async fn save_work_item_data(&mut self, item_id: &str, data: &JsonValue) -> Result<()>;
    async fn get_work_item_data(&self, item_id: &str) -> Result<Option<JsonValue>>;
    async fn delete_work_item(&mut self, item_id: &str) -> Result<()>;
    async fn list_work_items(&self) -> Result<Vec<String>>;
}
```

Two implementations are provided:

- **`MemoryStateStore`** — thread-safe, in-memory store for testing and
  development.
- **`SqliteStateStore`** — persistent SQLite-backed store with automatic
  schema migration (enabled via the `sqlite` feature, on by default).

This means:

- If the process crashes, you resume from where you left off.
- You can query the full state of any work item at any time.
- Pipeline status can be displayed in your CLI or TUI.

### Human-in-the-Loop Review Gates

When a stage returns `StageOutcome::NeedsReview`, the pipeline pauses for
that work item. The item sits in review until a human explicitly approves or
rejects it via `workflow.approve_review()` or `workflow.reject_review()`.
This is first-class in the engine, not a workaround.

### Fan-Out with Per-Subtask Tracking

A stage can fan out into multiple concurrent subtasks — for example, enriching
a record from five different APIs simultaneously. Each subtask is tracked
independently in the state store with its own status, retry count, and error
history. If three of five sources succeed and two fail, you retry only the
two that failed.

### Event Stream

The workflow engine emits structured events via a tokio broadcast channel.
Your TUI, CLI, or logging layer subscribes to these events for real-time
visibility:

```rust
#[non_exhaustive]
pub enum WorkflowEvent {
    StageStarted { item_id: String, stage: String },
    StageCompleted { item_id: String, stage: String },
    StageFailed { item_id: String, stage: String, error: String },
    ReviewRequired { item_id: String, stage: String, data: ReviewData },
    StageSkipped { item_id: String, stage: String },
    StageRetried { item_id: String, stage: String },
    FanOutStarted { item_id: String, stage: String, subtasks: Vec<String> },
    SubTaskStarted { item_id: String, stage: String, subtask: String },
    SubTaskCompleted { item_id: String, stage: String, subtask: String },
    SubTaskFailed { item_id: String, stage: String, subtask: String, error: String },
    WorkflowCompleted { item_id: String },
}
```

### Pipeline Status

The `PipelineStatus` type provides a complete snapshot of a work item's
progress through the workflow, including per-stage status, timing, retry
counts, and subtask details. It supports progress percentage calculation and
has a built-in `Display` implementation for quick inspection.

## Design Principles

1. **Library, not a service.** Treadle is a crate you embed in your
   application. No external runtime, no server process, no Docker container.
   Add it to your `Cargo.toml` and go.

2. **The human is part of the pipeline.** Review gates are a first-class
   concept, not an afterthought. The engine is designed around the assumption
   that some stages need human judgement.

3. **Visibility over magic.** Every piece of state is inspectable. You can
   always answer "where is this item in the pipeline, what happened at each
   stage, and why did this fail?" The event stream makes real-time
   observation trivial.

4. **Resilience is explicit.** The engine provides manual retry support
   (`retry_stage()`) and tracks retry counts per stage. Stage implementations
   can use whatever internal resilience strategy fits. v2 will add
   configurable retry budgets and quality gates for automatic retry loops.

5. **Stages are the unit of abstraction.** Implementing a new stage is
   implementing a trait. Adding a stage to the pipeline is adding a node and
   an edge. The engine handles ordering, state, and concurrency.

6. **Incremental by nature.** The pipeline processes items one at a time (or
   in batches), tracking each independently. New items can enter the pipeline
   at any time. Items at different stages coexist naturally.

## Architecture

```
┌──────────────────────────────────────────────────┐
│            Your Application (CLI, TUI, HTTP)     │
│                    ^ subscribes to events        │
└────────────────────┼─────────────────────────────┘
                     │
┌────────────────────┼─────────────────────────────┐
│  Treadle Engine    │                             │
│                    │                             │
│  ┌─────────────────v──────────────────────────┐  │
│  │  Event Stream (tokio broadcast channel)    │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │  Workflow (petgraph DAG of Stages)         │  │
│  │                                            │  │
│  │  scan ──> identify ──> enrich ──> review   │  │
│  │                          │                 │  │
│  │                   ┌──────┴───────┐         │  │
│  │                   │   fan-out    │         │  │
│  │                   │ src1 src2 …  │         │  │
│  │                   └──────────────┘         │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │  StateStore (SQLite / in-memory / custom)  │  │
│  │  item × stage × subtask → status           │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

## Quick Start

Add Treadle to your `Cargo.toml`:

```toml
[dependencies]
treadle = "0.2"
```

## Usage Example

```rust
use treadle::{
    Workflow, Stage, StageOutcome, StageContext, WorkItem,
    MemoryStateStore, ReviewData, Result,
};
use async_trait::async_trait;

// Define a work item
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Document {
    id: String,
    path: String,
}

impl WorkItem for Document {
    fn id(&self) -> &str { &self.id }
}

// Implement stages
#[derive(Debug)]
struct ParseStage;

#[async_trait]
impl Stage for ParseStage {
    fn name(&self) -> &str { "parse" }
    async fn execute(&self, _item: &dyn WorkItem, _ctx: &mut StageContext) -> Result<StageOutcome> {
        // ... parse the document ...
        Ok(StageOutcome::Complete)
    }
}

#[derive(Debug)]
struct ReviewStage;

#[async_trait]
impl Stage for ReviewStage {
    fn name(&self) -> &str { "review" }
    async fn execute(&self, item: &dyn WorkItem, _ctx: &mut StageContext) -> Result<StageOutcome> {
        Ok(StageOutcome::NeedsReview)
    }
}

#[derive(Debug)]
struct ExportStage;

#[async_trait]
impl Stage for ExportStage {
    fn name(&self) -> &str { "export" }
    async fn execute(&self, _item: &dyn WorkItem, _ctx: &mut StageContext) -> Result<StageOutcome> {
        Ok(StageOutcome::Complete)
    }
}

// Build and run
#[tokio::main]
async fn main() -> Result<()> {
    let mut store = MemoryStateStore::new();

    let workflow = Workflow::builder()
        .stage("parse", ParseStage)
        .stage("review", ReviewStage)
        .stage("export", ExportStage)
        .dependency("review", "parse")
        .dependency("export", "review")
        .build()?;

    // Subscribe to events for your TUI/CLI
    let mut events = workflow.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            println!("{event:?}");
        }
    });

    // Process an item — advances through all eligible stages
    let doc = Document { id: "doc-1".into(), path: "report.pdf".into() };
    workflow.advance(&doc, &mut store).await?;
    // parse completes, review pauses for human judgement

    // Later: approve the review and continue
    workflow.approve_review(doc.id(), "review", &mut store).await?;
    workflow.advance(&doc, &mut store).await?;
    // export completes

    Ok(())
}
```

See [`examples/basic_pipeline.rs`](examples/basic_pipeline.rs) for a more
complete example with event streaming, status display, and fan-out stages.

## Target Use Cases

- **Media processing pipelines** — scan files, identify metadata, enrich
  from external sources, review, export. (This is the motivating use case:
  [tessitura](https://github.com/TODO/tessitura), a musicological library
  cataloguing tool.)
- **AI agent pipelines** — LLM-driven document processing with quality gates
  and human review for confidence thresholds.
- **Data migration / ETL tools** — extract records, transform, validate
  with human review, load.
- **Document processing** — parse, classify, review, archive.
- **Content moderation pipelines** — ingest, auto-classify, flag for human
  review, publish or reject.
- **Any CLI tool** where items flow through stages, some stages need human
  judgement, and you need the pipeline to survive restarts.

## Related Projects

For why this project was created and a brief overview of related projects in
the Rust ecosystem, be sure to check out:

- [Rust DAG/workflow/pipeline Projects](./docs/related-projects.md)

## Roadmap

### v1 (Complete)

- [x] Core traits: `WorkItem`, `Stage`, `StageOutcome`, `StateStore`
- [x] petgraph-backed `Workflow` with builder pattern and DAG validation
- [x] SQLite `StateStore` implementation
- [x] In-memory `StateStore` for testing
- [x] Workflow executor with topological stage ordering
- [x] Fan-out with per-subtask state tracking
- [x] Event stream via tokio broadcast channel
- [x] Pipeline status and visualisation helpers
- [x] Human review gates with approve/reject
- [x] Manual retry support for failed stages
- [x] Documentation and examples

### v2 (In Progress)

- [ ] `Artefact` trait and typed artefact passing between stages
- [ ] `QualityGate` trait for automated output evaluation
- [ ] `RetryBudget` with configurable attempts, delays, and timeouts
- [ ] `ReviewPolicy` (Never, Always, OnEscalation, OnUncertain,
  OnEscalationOrUncertain)
- [ ] `ReviewOutcome` with ApproveWithEdits support
- [ ] Automatic retry loop with quality feedback threading
- [ ] Attempt history and structured quality feedback
- [ ] Integration tests and updated documentation

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

[//]: ---Named-Links---

[logo]: assets/images/logo/v1-x250.png
[logo-large]: assets/images/logo/v1.png
[build]: https://github.com/oxur/treadle/actions/workflows/ci.yml
[build-badge]: https://github.com/oxur/treadle/actions/workflows/ci.yml/badge.svg
[crate]: https://crates.io/crates/treadle
[crate-badge]: https://img.shields.io/crates/v/treadle.svg
[docs]: https://docs.rs/treadle/
[docs-badge]: https://img.shields.io/badge/rust-documentation-blue.svg
[tag-badge]: https://img.shields.io/github/tag/oxur/treadle.svg
[tag]: https://github.com/oxur/treadle/tags
