# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-02-08

### Added

- **Core workflow execution engine**: Complete implementation of workflow advancement with recursive execution through DAG stages
- **Event system**: Real-time workflow event streaming via tokio broadcast channel for monitoring and observability
  - 11 event types covering stage lifecycle, fan-out execution, reviews, and completion
  - Subscribe pattern for multiple observers
- **Human-in-the-loop support**: Stages can pause for review with approve/reject capabilities
  - `StageOutcome::NeedsReview` for requesting human judgment
  - `approve_review()` and `reject_review()` methods for workflow control
  - `ReviewData` type for passing context to reviewers
- **Fan-out execution**: Stages can spawn parallel subtasks with independent tracking
  - `StageOutcome::FanOut(Vec<SubTask>)` for declaring parallel work
  - Per-subtask status tracking and error handling
  - Automatic aggregation of subtask results
- **Pipeline status introspection**: Complete visibility into workflow state
  - `PipelineStatus` with progress tracking and stage details
  - Pretty-printed Display implementation with Unicode status indicators
  - Helper methods: `is_complete()`, `has_failures()`, `has_pending_reviews()`, `progress_percent()`
- **Tracing integration**: Structured logging with spans for production observability
  - Info-level spans for `advance()`, `stage`, and `fanout` operations
  - Debug/info/warn events throughout execution paths
  - Request correlation via span context
- **Comprehensive testing**: 166 total tests ensuring reliability
  - 149 unit tests covering all core functionality
  - 8 integration tests exercising full pipeline scenarios
  - 9 doc tests validating example code
- **Examples and documentation**: Production-ready documentation and runnable examples
  - Expanded crate-level docs with complete usage examples
  - `basic_pipeline` example demonstrating all features
  - Comprehensive API documentation on all public types
- **API improvements**:
  - Added `Send + Sync` bounds to `WorkItem` trait for proper async support
  - Added `StageStatus` import to workflow module

### Changed

- Workflow execution now properly emits events for `approve_review()` and `reject_review()`
- Fan-out execution now correctly tracks subtasks in stage state
- State stores now use `&str` for work item IDs throughout the API

### Fixed

- Fan-out stages now properly persist subtask list in state
- Review approval/rejection now emits appropriate completion/failure events
- Subtask status updates now correctly saved during fan-out execution

## [0.1.0] - 2025-12-XX

### Added

- Initial release of Treadle workflow engine
- Core types: `WorkItem`, `Stage`, `StageOutcome`, `StageContext`, `StageState`, `StageStatus`
- State persistence: `StateStore` trait with `MemoryStateStore` and `SqliteStateStore`
- Workflow construction: `Workflow::builder()` with DAG validation
- Basic workflow operations: `ready_stages()`, `is_complete()`, `is_blocked()`
- Error handling with `TreadleError` and `Result` types
- Comprehensive test coverage

### Features

- `sqlite` (default): Enables SQLite-backed state persistence

---

## Release Notes

### Version 0.2.0 - Production Ready

This release completes the core workflow execution engine with all planned features:

**Execution Engine**: The workflow now advances work items through DAG stages automatically, handling dependencies, fan-out execution, and human-in-the-loop reviews.

**Event Streaming**: Subscribe to real-time workflow events for monitoring, logging, or building UIs. Events cover the complete lifecycle from stage start to workflow completion.

**Human Reviews**: Stages can pause for human judgment with full context, then resume after approval or rejection.

**Fan-Out**: Stages can spawn parallel subtasks (e.g., enriching from multiple sources) with independent tracking and retry.

**Observability**: Structured tracing spans and events throughout execution, plus pipeline status introspection at any point.

**Production Ready**: Comprehensive test coverage, complete documentation, and runnable examples make this ready for real-world use.

See the [examples/](examples/) directory for working code, or check out the [online documentation](https://docs.rs/treadle) for the complete API reference.
