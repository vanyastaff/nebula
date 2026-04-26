# temporalio/sdk-rust — Architectural Issues

Selected from 50+ open issues (as of 2026-04-26). Sorted by architectural significance.

---

## Open Architectural Issues

### #1144 — [Feature Request] Testing utilities
**URL:** https://github.com/temporalio/sdk-rust/issues/1144
**Labels:** enhancement, Rust SDK
**Body:** "No dedicated test framework. Need `TestWorkflowEnvironment`, time skipping, `ActivityEnvironment` for isolated activity testing, `WorkflowReplayer` for determinism verification, and dev server integration."
**Significance:** The Rust SDK has zero public testing utilities — no test harness, no time-skipping, no mocked environments. Every test requires a live Temporal server. This is a major gap relative to Java/Python/TypeScript SDKs.

---

### #1145 — [Feature Request] Nexus handler definitions
**URL:** https://github.com/temporalio/sdk-rust/issues/1145
**Labels:** enhancement, Rust SDK
**Body:** "Rust SDK can invoke Nexus operations from workflows but cannot implement/handle them. Need Nexus service definition, handler registration on workers, and operation context."
**Significance:** Nexus is Temporal's inter-workflow / cross-namespace operation protocol. The SDK supports calling Nexus ops from workflows but the server side (defining handlers) is not yet implemented.

---

### #1140 — [Feature Request] Activity interceptors
**URL:** https://github.com/temporalio/sdk-rust/issues/1140
**Labels:** enhancement, Rust SDK
**Body:** "No activity-level inbound/outbound interceptors."
**Comment from contributor (2026-03-18):** Proposes `ActivityInterceptor` trait with `on_activity_start` / `on_activity_finish` hooks. API sketch suggests adding `activity_interceptors: Vec<Arc<dyn ActivityInterceptor>>` to `WorkerOptions`.
**Significance:** Activities have no lifecycle hooks. Only a `WorkerInterceptor` (activation-level) exists. The Java SDK and others have full activity interceptors.

---

### #1139 — [Feature Request] Workflow interceptors
**URL:** https://github.com/temporalio/sdk-rust/issues/1139
**Labels:** enhancement, Rust SDK
**Body:** "No inbound/outbound workflow interceptors. The existing `WorkerInterceptor` operates at the activation level, not at the semantic level of individual workflow operations."
**Significance:** `WorkerInterceptor` at `sdk/src/interceptors.rs:18` only fires on `on_workflow_activation_completion` and `on_shutdown`, not on individual signals/queries/updates.

---

### #1138 — [Feature Request] Client-side interceptors
**URL:** https://github.com/temporalio/sdk-rust/issues/1138
**Labels:** enhancement, Rust SDK
**Significance:** No client-side interceptor middleware for adding headers, auth tokens, or logging on outgoing gRPC calls.

---

### #1137 — [Feature Request] Failure Converter implementation
**URL:** https://github.com/temporalio/sdk-rust/issues/1137
**Labels:** enhancement, Rust SDK
**Significance:** `DataConverter` includes `FailureConverter` but it is currently a stub (`#[allow(dead_code)]` in `crates/common/src/data_converters.rs:16`). No custom failure conversion implemented.

---

### #1213 — [Feature Request] Implement WorkflowHistory as a stream
**URL:** https://github.com/temporalio/sdk-rust/issues/1213
**Labels:** enhancement, good first issue
**Significance:** History fetching (for replay/debugging) currently requires loading the full history at once. Streaming is needed for large histories (millions of events).

---

### #1145 — Nexus handler definitions (see above)

---

### #687 — [Feature Request] Managing dependencies for the Rust SDK
**URL:** https://github.com/temporalio/sdk-rust/issues/687
**Labels:** enhancement, Rust SDK
**Body:** Contributor reports ~800 crates added to workspace when pulling in temporal-sdk. Identifies `mockall` (leaked to non-dev), `futures-retry 0.6.0` (old), `enum_dispatch`, `opentelemetry-prometheus` (pulls in `prometheus 0.13.3` + `protobuf 2.28.0` as non-optional deps) as notable bloat.
**Significance:** DX issue for teams embedding temporal-sdk in large workspaces. Signals the SDK's dependency hygiene is not yet production-ready.

---

### #692 — [Bug] Panic when failing to start worker due to being unauthorized
**URL:** https://github.com/temporalio/sdk-rust/issues/692
**Labels:** bug, Rust SDK
**Body:** Permission-denied gRPC error on startup causes a panic (`Activation processor channel not dropped`) rather than a clean `Err(...)` return. Fixed post-0.1 but indicative of the SDK's maturity level at the time.

---

### #1196 — [Feature Request] Remove task_types on WorkerOptions
**URL:** https://github.com/temporalio/sdk-rust/issues/1196
**Labels:** enhancement
**Significance:** `WorkerOptions` currently requires explicit `task_types` (workflow-only, activity-only, or both). Other SDK languages auto-detect based on registered functions.

---

## Summary of architectural gaps

| Gap | Severity | Notes |
|-----|----------|-------|
| No testing utilities (TestWorkflowEnvironment) | High | Issue #1144 — still open |
| No activity/workflow interceptors | High | Issues #1139, #1140 — design-only |
| No Nexus handler side | High | Issue #1145 — unimplemented |
| No FailureConverter | Medium | DataConverter stub |
| No client interceptors | Medium | Issue #1138 |
| Dependency bloat | Medium | Issue #687 — partially addressed in v0.3 |
| Missing streaming WorkflowHistory | Low | Issue #1213 |
