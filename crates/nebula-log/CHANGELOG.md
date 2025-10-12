# Changelog

All notable changes to `nebula-log` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added - Observability System (Sprint 1-4)

#### Core Infrastructure
- **Unified Observability Framework** - Complete observability system with events, hooks, and metrics
- **Lock-Free Event Registry** - Using `arc-swap` for zero-contention event emission across threads
- **Panic Safety** - All hooks wrapped in `catch_unwind` to prevent system crashes from user code
- **Event Filtering** - Prefix, exact, set, and custom predicate filters with AND/OR/NOT combinators

#### Metrics Support (Sprint 1)
- **Standard `metrics` Integration** - Added `metrics` crate v0.23 support
- **Timing Utilities** - `timed_block()` and `timed_block_async()` for automatic duration recording
- **Helper Macros** - Re-exported `counter!`, `gauge!`, `histogram!` macros
- **Feature Flag** - New `observability` feature for optional metrics support

#### Event System (Sprint 2)
- **`ObservabilityEvent` Trait** - Unified event interface for all crates
- **`ObservabilityHook` Trait** - Extensible hook system for event processing
- **Built-in Hooks**:
  - `LoggingHook` - Emit events as tracing logs
  - `MetricsHook` - Record events as metrics counters
- **Global Registry** - Thread-safe hook registration and event emission
- **Lifecycle Events** - `OperationStarted`, `OperationCompleted`, `OperationFailed`
- **RAII Operation Tracker** - Automatic lifecycle tracking with panic safety

#### Context System
- **Multi-Level Contexts**:
  - `GlobalContext` - Application-wide settings (service name, version, environment)
  - `ExecutionContext` - Workflow execution scope (execution_id, workflow_id, tenant_id)
  - `NodeContext` - Node execution scope (node_id, action_id, resources)
- **RAII Guards** - Automatic context cleanup via `Drop`
- **Thread-Local Storage** - Context propagation using `thread_local!`
- **Span-Like Nesting** - Contexts nest like `tracing` spans with automatic resource merging

#### Resource Management
- **`LoggerResource`** - Complete logging configuration (Sentry, webhooks, tags, sampling)
- **`NotificationPrefs`** - Email/webhook notification settings with rate limiting
- **Resource Scoping** - Per-node resource isolation for security
- **`ResourceAwareHook`** - Hooks that access node-scoped resources
- **`ResourceAwareAdapter`** - Adapter for integrating resource-aware hooks
- **Span-Like Resource Merging** - Automatic resource inheritance from parent contexts

#### Advanced Features
- **Event Filtering**:
  - `EventFilter::prefix()` - Filter by event name prefix
  - `EventFilter::exact()` - Filter by exact name
  - `EventFilter::set()` - Filter by name set
  - `EventFilter::custom()` - Custom predicate filtering
  - Combinators: `and()`, `or()`, `not()`
- **`get_current_logger_resource()`** - Get merged LoggerResource from all active contexts
- **Context Snapshot** - `current_contexts()` for capturing all active contexts

### Documentation

#### Guides
- **OBSERVABILITY_GUIDE.md** - Comprehensive integration guide with patterns and best practices
- **MIGRATION_GUIDE.md** - Step-by-step migration from custom observability solutions
- **SPAN_LIKE_ARCHITECTURE.md** - Span-like nested context architecture documentation
- **OBSERVABILITY_ARCHITECTURE.md** - Separation of concerns and architecture principles

#### Examples
- `examples/metrics_basic.rs` - Basic metrics usage
- `examples/observability_hooks.rs` - Custom hook implementation
- `examples/resource_based_observability.rs` - Resource-scoped logging demonstration
- `examples/span_like_resources.rs` - Span-like nested resource contexts

### Changed

#### Dependencies
- Added `metrics = { version = "0.23", optional = true }`
- Added `arc-swap = "1.7"` for lock-free registry
- Added `serde = { version = "1.0", features = ["derive"] }`

#### Features
- New `observability` feature enables metrics and observability system
- `full` feature now includes `observability`
- `telemetry` feature depends on `observability`

#### Module Structure
- Added `src/metrics/` - Metrics support module
- Added `src/observability/` - Complete observability system:
  - `context.rs` - Multi-level context system
  - `events.rs` - Lifecycle events
  - `filter.rs` - Event filtering
  - `hooks.rs` - Hook traits and implementations
  - `registry.rs` - Lock-free global registry
  - `resources.rs` - LoggerResource and related types
  - `span.rs` - Span-like resource merging

### Performance

- **Lock-Free Emission** - Zero contention for `emit_event()` using `arc-swap`
- **Panic Safety** - No system crashes from panicking hooks
- **Event Filtering** - Reduce overhead by filtering at hook level
- **Zero-Cost Abstractions** - When `observability` feature is disabled, no runtime cost

### Security

- **Resource Isolation** - Per-node resource scoping prevents credential leakage
- **Multi-Tenant Safe** - Different nodes cannot access each other's resources
- **No Global Credentials** - Resources scoped to execution contexts, not global

### Testing

- **41 Tests Total**:
  - 6 context tests (global, execution, node, guards, snapshots)
  - 5 registry tests (basic, multi-hook, thread safety, panic safety)
  - 9 filter tests (all filter types and combinators)
  - 6 event tests (lifecycle events, tracker)
  - 7 resource tests (builder, serialization, notifications)
  - 4 span tests (merging, override, isolation)
  - 4 other tests (error, result extensions)

### Migration Notes

For migrating existing crates:

1. **nebula-resilience** (Sprint 3):
   - Removed custom `MetricsCollector`
   - Using standard `metrics!()` macros
   - Using `ObservabilityHook` for circuit breaker events
   - All tests passing

2. **Other Crates** (Future):
   - See `docs/MIGRATION_GUIDE.md` for step-by-step instructions
   - Migration checklist provided
   - Rollback plan documented

### Breaking Changes

None - All changes are additive and feature-gated.

## [0.1.0] - Previous Release

### Added
- Initial logging infrastructure
- Tracing integration
- File and console appenders
- OpenTelemetry support (optional)
- Sentry integration (optional)

[Unreleased]: https://github.com/your-org/nebula/compare/nebula-log-v0.1.0...HEAD
[0.1.0]: https://github.com/your-org/nebula/releases/tag/nebula-log-v0.1.0
