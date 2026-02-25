# Execution View & Tracing — Industry Research

> Research document: how leading workflow/durable execution systems implement Execution View, Tracing, and observability. Intended as a reference for Nebula's future execution visibility features.

## Overview

Systems like Inngest, Temporal, Arize Phoenix, n8n, and Prefect provide rich execution visibility. This document summarizes their approaches and patterns applicable to Nebula.

---

## Inngest

**Docs:** [Traces](https://www.inngest.com/docs/platform/monitor/traces), [Inspecting Function Runs](https://www.inngest.com/docs/platform/monitor/inspecting-function-runs)

### Three Levels of Tracing

| Level | Purpose | Data Captured |
|-------|---------|---------------|
| **Built-in Traces** | Default, no config | Retry attempts, logs, event data, step execution timing, function timeline |
| **AI Traces** | AI Inference steps | Input prompt, output response (side-by-side), token counts, model name/version |
| **Extended Traces** | OpenTelemetry | HTTP requests, DB queries, third-party calls, distributed context |

### Run Traces View

- **Waterfall visualization** — Sequence and timing of steps, including parallel execution; inspired by OpenTelemetry
- **Interactive drill-down** — Expand steps to see input, output, errors
- **Expanded run details** — Granular view without losing workflow context

### Function Run Details Panel

- **Timeline** — Steps execution order with timing
- **Event payload** — Trigger event that started the run
- **Trigger details** — For support/debugging
- **Step expansion** — Retries, errors, timings per step

### Search & Filtering

- CEL expressions over `event.data`, `output`, `event.name`, `event.id`, `event.ts`
- Error search: `output.name == "UserNotFoundError" && output.message == "Failed to import data"`
- Custom error types improve searchability

### Actions

- **Replay** — Rerun failed function
- **Send to Dev Server** — Reproduce locally with same trigger event

---

## Temporal

**Docs:** [Web UI](https://docs.temporal.io/web-ui), [Events and Event History](https://docs.temporal.io/workflow-execution/event)

### Event History

- Append-only log of ~40 event types
- Activity lifecycle: `ActivityTaskScheduled` → `ActivityTaskStarted` → `ActivityTaskCompleted` / `ActivityTaskFailed` / `ActivityTaskTimedOut` / `ActivityTaskCanceled`
- Used for durable execution, replay, and audit

### Web UI — Workflow Execution View

**History tab views:**

| View | Description |
|------|-------------|
| **JSON** | Full JSON event history |
| **Compact** | Logical grouping (Activities, Signals, Timers) |
| **All** | All events |
| **Timeline** | Chronological/reverse-chronological with summary; click event for details |

**Other sections:**

- **Input and Results** — Function arguments and return values (available when workflow finishes)
- **Relationships** — Parent/child workflow tree
- **Call Stack** — Where workflow is waiting (via `__stack_trace` Query)
- **Pending Activities** — Active and waiting activities
- **Metadata** — User metadata, static/dynamic summary and details
- **Workers** — Workers polling the task queue

### Additional Features

- **Download Event History** — Export as JSON
- **Codec Server** — Decode encrypted input/output in UI
- **Saved Views** — Saved filter queries
- **Task Failures View** — Workflows with task failures
- **Workflow Actions** — Cancel, Signal, Update, Reset, Terminate from UI

---

## Arize Phoenix

**Docs:** [Tracing](https://arize.com/docs/phoenix/tracing/how-to-tracing)

### Focus

AI/LLM application tracing and observability.

### Features

- **Spans** — Metadata, tags, custom attributes
- **Sessions** — Group related traces (e.g. conversations)
- **Projects** — Organize traces by application
- **Span Chat** — Analyze and evaluate specific spans
- **Evaluations** — Quality metrics (correctness, relevance)
- **Integrations** — OpenAI, LangChain, LlamaIndex, Anthropic

### Use Case

Phoenix is complementary: execution visibility + quality evaluation for AI workflows.

---

## n8n

**Docs:** [Debug and re-run past executions](https://docs.n8n.io/workflows/executions/debug)

### Execution View

- **All executions** — Across workflows
- **Workflow-level executions** — Per workflow
- **Filters** — Status, workflow name, start time, saved custom data

### Input/Output

- **Copy to editor** — Successful runs: load data into editor
- **Debug in editor** — Failed runs: load data and fix workflow, then re-run

### Limitations

- No real-time progress in production; node details available after completion
- Manual test mode shows nodes running in editor

---

## Prefect

**Docs:** [Logging](https://orion-docs.prefect.io/guides/logs), [Flow Runs](https://orion-docs.prefect.io/ui/flow-runs)

### Features

- **Logs** — Flow and task run logs in UI
- **CLI** — `prefect flow-run logs` with `--head`, `--tail`, `--num-logs`, `--reverse`
- **REST API** — Download logs endpoint
- **Log levels** — CRITICAL, ERROR, WARNING, INFO, DEBUG
- **Scoping** — Flow/task logs vs worker/agent logs

---

## Patterns for Nebula

| Pattern | Source | Recommendation |
|---------|--------|----------------|
| **Waterfall / Timeline** | Inngest, Temporal | Visualize step sequence and timing; show parallel execution |
| **Input/Output per step** | Inngest, Temporal | Expandable step details: input, output, errors |
| **Event History** | Temporal | Append-only event log for replay, audit, recovery |
| **Search by data** | Inngest | Query by event payload, output, error fields |
| **Replay / Rerun** | Inngest, Temporal | Re-run execution with same input |
| **OpenTelemetry** | Inngest | Integrate with external observability stack |
| **Call Stack / Pending** | Temporal | Show where execution is waiting; pending nodes |
| **Copy/Debug in editor** | n8n | Load execution data for debugging |

---

## Nebula Execution Crate Context

The `nebula-execution` crate already provides building blocks:

- **`JournalEntry`** — Audit log of execution events (analogous to Event History)
- **`NodeOutput`** — Node output with metadata
- **`NodeAttempt`** — Per-attempt tracking (retries)
- **`ExecutionState`** / **`NodeExecutionState`** — State tracking
- **`ExecutionContext`** — Runtime context

Future Execution View features can build on these types to support:

1. Timeline/waterfall visualization
2. Step-level input/output inspection
3. Search and filtering over journal/event data
4. Replay with same trigger/input
5. OpenTelemetry span integration (via `nebula-telemetry`)

---

## References

- [Inngest Traces](https://www.inngest.com/docs/platform/monitor/traces)
- [Inngest Inspecting Function Runs](https://www.inngest.com/docs/platform/monitor/inspecting-function-runs)
- [Inngest Enhanced Observability (Waterfall)](https://www.inngest.com/blog/enhanced-observability-traces-and-metrics)
- [Temporal Web UI](https://docs.temporal.io/web-ui)
- [Temporal Events and Event History](https://docs.temporal.io/workflow-execution/event)
- [Arize Phoenix Tracing](https://arize.com/docs/phoenix/tracing/how-to-tracing)
- [n8n Debug Executions](https://docs.n8n.io/workflows/executions/debug)
- [Prefect Logging](https://orion-docs.prefect.io/guides/logs)
